package com.sassyconsulting.sassytalkie.service

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.util.Log
import com.sassyconsulting.sassytalkie.SassyTalkNative
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Bluetooth RFCOMM transport for SassyTalkie PTT audio.
 *
 * Manages peer-to-peer RFCOMM connections with:
 * - Triple fallback for connection (standard → insecure → reflection port 1)
 * - Connection state gating on PTT (won't start mic if no peers)
 * - Channel sync on peer connect
 * - Robust RX frame reassembly (length-prefixed framing)
 * - Proper dead peer cleanup
 *
 * Data flow:
 *   TX: Rust mic → btEncodeFrame() → this class writes to RFCOMM sockets
 *   RX: RFCOMM socket → this class reads → btDecodeFrame() → Rust plays audio
 */
@SuppressLint("MissingPermission")
class BluetoothTransport(private val context: Context) {

    companion object {
        private const val TAG = "BluetoothTransport"

        // Standard SPP UUID for walkie-talkie audio
        private val SPP_UUID: UUID = UUID.fromString("00001101-0000-1000-8000-00805F9B34FB")

        // Service name for server socket
        private const val SERVICE_NAME = "SassyTalkBT"

        // Frame protocol: [length:4 LE][payload:N]
        private const val FRAME_HEADER_SIZE = 4
        private const val MAX_FRAME_SIZE = 4096

        // Dead peer detection timeout (ms)
        private const val DEAD_PEER_TIMEOUT_MS = 10_000L
    }

    // ── State ──

    enum class State { DISCONNECTED, CONNECTING, CONNECTED }

    @Volatile
    var state: State = State.DISCONNECTED
        private set

    private val btAdapter: BluetoothAdapter? by lazy {
        val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
        manager?.adapter
    }

    // ── Connected Peers ──

    data class ConnectedPeer(
        val device: BluetoothDevice,
        val socket: BluetoothSocket,
        val input: InputStream,
        val output: OutputStream,
        @Volatile var lastActivity: Long = System.currentTimeMillis()
    )

    private val connectedPeers = ConcurrentHashMap<String, ConnectedPeer>()
    private val running = AtomicBoolean(false)
    private val txPumpRunning = AtomicBoolean(false)
    private val peerCount = AtomicInteger(0)

    // Server socket for incoming connections
    private var serverSocket: BluetoothServerSocket? = null

    // ── Public API ──

    val isConnected: Boolean get() = connectedPeers.isNotEmpty()
    val connectedPeerCount: Int get() = peerCount.get()
    val peerNames: List<String> get() = connectedPeers.values.map { it.device.name ?: it.device.address }

    /**
     * Connect to a remote device using RFCOMM triple fallback.
     * Returns true if connection succeeded.
     */
    fun connectTo(device: BluetoothDevice): Boolean {
        val addr = device.address
        if (connectedPeers.containsKey(addr)) {
            Log.w(TAG, "Already connected to ${device.name}")
            return true
        }

        state = State.CONNECTING
        Log.i(TAG, "Connecting to ${device.name} ($addr)...")

        // === Fallback 1: Standard RFCOMM ===
        try {
            val socket = device.createRfcommSocketToServiceRecord(SPP_UUID)
            socket.connect()
            onPeerConnected(device, socket)
            Log.i(TAG, "RFCOMM connected (standard) to ${device.name}")
            return true
        } catch (e: IOException) {
            Log.w(TAG, "Standard RFCOMM failed for ${device.name}: ${e.message}")
        }

        // === Fallback 2: Insecure RFCOMM ===
        try {
            val socket = device.createInsecureRfcommSocketToServiceRecord(SPP_UUID)
            socket.connect()
            onPeerConnected(device, socket)
            Log.i(TAG, "RFCOMM connected (insecure) to ${device.name}")
            return true
        } catch (e: IOException) {
            Log.w(TAG, "Insecure RFCOMM failed for ${device.name}: ${e.message}")
        }

        // === Fallback 3: Reflection port 1 ===
        try {
            val method = device.javaClass.getMethod("createRfcommSocket", Int::class.java)
            val socket = method.invoke(device, 1) as BluetoothSocket
            socket.connect()
            onPeerConnected(device, socket)
            Log.i(TAG, "RFCOMM connected (reflection port 1) to ${device.name}")
            return true
        } catch (e: Exception) {
            Log.e(TAG, "All RFCOMM methods failed for ${device.name}: ${e.message}")
        }

        state = if (connectedPeers.isEmpty()) State.DISCONNECTED else State.CONNECTED
        return false
    }

    /**
     * Start accepting incoming RFCOMM connections.
     */
    fun startAcceptThread() {
        if (running.getAndSet(true)) return

        Thread {
            Thread.currentThread().name = "bt-accept"
            Log.i(TAG, "Accept thread started")

            try {
                serverSocket = btAdapter?.listenUsingRfcommWithServiceRecord(SERVICE_NAME, SPP_UUID)
                    ?: run {
                        // Insecure fallback
                        btAdapter?.listenUsingInsecureRfcommWithServiceRecord(SERVICE_NAME, SPP_UUID)
                    }
            } catch (e: IOException) {
                Log.e(TAG, "Failed to create server socket: ${e.message}")
                running.set(false)
                return@Thread
            }

            while (running.get()) {
                try {
                    val socket = serverSocket?.accept(30_000) ?: continue
                    val device = socket.remoteDevice
                    Log.i(TAG, "Accepted connection from ${device.name} (${device.address})")
                    onPeerConnected(device, socket)
                } catch (e: IOException) {
                    if (running.get()) {
                        // Timeout is normal, other errors are not
                        if (!e.message.orEmpty().contains("timeout", ignoreCase = true)) {
                            Log.w(TAG, "Accept error: ${e.message}")
                        }
                    }
                }
            }

            Log.i(TAG, "Accept thread stopped")
        }.start()
    }

    /**
     * Start the BT TX pump. While PTT is active, reads encoded frames from Rust
     * and writes them to all connected peer sockets.
     */
    fun startTxPump() {
        if (txPumpRunning.getAndSet(true)) return

        Thread {
            Thread.currentThread().name = "bt-tx-pump"
            val peerCountAtStart = connectedPeers.size
            Log.i(TAG, "BT TX pump started ($peerCountAtStart peers)")

            while (txPumpRunning.get() && SassyTalkNative.isPttActive()) {
                if (connectedPeers.isEmpty()) {
                    Log.w(TAG, "BT TX pump: no peers, stopping")
                    break
                }

                // Get one encoded frame from Rust (mic → ADPCM encode → wire frame)
                val frameData = SassyTalkNative.btEncodeFrame()
                if (frameData == null) {
                    // No audio data yet, wait for mic to fill buffer
                    Thread.sleep(2)
                    continue
                }

                // Write length-prefixed frame to all connected peers
                val header = ByteBuffer.allocate(FRAME_HEADER_SIZE)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(frameData.size)
                    .array()

                val deadPeers = mutableListOf<String>()

                for ((addr, peer) in connectedPeers) {
                    try {
                        synchronized(peer.output) {
                            peer.output.write(header)
                            peer.output.write(frameData)
                            peer.output.flush()
                        }
                        peer.lastActivity = System.currentTimeMillis()
                    } catch (e: IOException) {
                        Log.w(TAG, "TX write failed for ${peer.device.name}: ${e.message}")
                        deadPeers.add(addr)
                    }
                }

                // Cleanup dead peers
                deadPeers.forEach { removePeer(it) }
            }

            txPumpRunning.set(false)
            Log.i(TAG, "BT TX pump stopped")
        }.start()
    }

    /** Stop the TX pump (called on PTT release) */
    fun stopTxPump() {
        txPumpRunning.set(false)
    }

    /** Disconnect all peers and stop */
    fun shutdown() {
        Log.i(TAG, "Shutting down BluetoothTransport")
        running.set(false)
        txPumpRunning.set(false)

        try { serverSocket?.close() } catch (_: IOException) {}
        serverSocket = null

        val addrs = connectedPeers.keys.toList()
        addrs.forEach { removePeer(it) }

        state = State.DISCONNECTED
        SassyTalkNative.btDisconnected()
    }

    /** Disconnect a specific peer */
    fun disconnectPeer(address: String) {
        removePeer(address)
    }

    // ── Internal ──

    private fun onPeerConnected(device: BluetoothDevice, socket: BluetoothSocket) {
        val addr = device.address
        val peer = ConnectedPeer(
            device = device,
            socket = socket,
            input = socket.inputStream,
            output = socket.outputStream
        )

        connectedPeers[addr] = peer
        peerCount.set(connectedPeers.size)
        state = State.CONNECTED

        // Notify Rust that BT transport is active
        SassyTalkNative.btConnected()

        // Sync channel with peer: send current channel as first message
        sendChannelSync(peer)

        // Start RX thread for this peer
        startRxThread(peer)

        // Start dead peer cleanup
        startDeadPeerMonitor()

        Log.i(TAG, "Peer connected: ${device.name} ($addr), total peers: ${connectedPeers.size}")
    }

    /**
     * Send channel sync message to a newly connected peer.
     * Format: [0xFF][0xFF][channel:1] — distinguished from audio frames by the 0xFFFF prefix.
     */
    private fun sendChannelSync(peer: ConnectedPeer) {
        try {
            val channel = SassyTalkNative.getChannel()
            val syncMsg = byteArrayOf(0xFF.toByte(), 0xFF.toByte(), channel.toByte())
            val header = ByteBuffer.allocate(FRAME_HEADER_SIZE)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(syncMsg.size)
                .array()

            synchronized(peer.output) {
                peer.output.write(header)
                peer.output.write(syncMsg)
                peer.output.flush()
            }
            Log.d(TAG, "Channel sync sent: ch=$channel to ${peer.device.name}")
        } catch (e: IOException) {
            Log.w(TAG, "Channel sync failed: ${e.message}")
        }
    }

    /**
     * Start an RX thread for a connected peer.
     * Reads length-prefixed frames from the socket, reassembles, and passes to Rust for decoding.
     */
    private fun startRxThread(peer: ConnectedPeer) {
        Thread {
            Thread.currentThread().name = "bt-rx-${peer.device.address.takeLast(5)}"
            Log.i(TAG, "RX thread started for ${peer.device.name}")

            val headerBuf = ByteArray(FRAME_HEADER_SIZE)

            try {
                while (running.get() && peer.socket.isConnected) {
                    // Read frame header (4 bytes, little-endian length)
                    readFully(peer.input, headerBuf, FRAME_HEADER_SIZE)

                    val frameLen = ByteBuffer.wrap(headerBuf)
                        .order(ByteOrder.LITTLE_ENDIAN)
                        .getInt()

                    if (frameLen <= 0 || frameLen > MAX_FRAME_SIZE) {
                        Log.w(TAG, "RX: invalid frame length $frameLen from ${peer.device.name}, skipping")
                        continue
                    }

                    // Read frame payload
                    val payload = ByteArray(frameLen)
                    readFully(peer.input, payload, frameLen)

                    peer.lastActivity = System.currentTimeMillis()

                    // Check if it's a channel sync message (0xFF 0xFF prefix)
                    if (frameLen == 3 && payload[0] == 0xFF.toByte() && payload[1] == 0xFF.toByte()) {
                        val remoteChannel = payload[2].toInt() and 0xFF
                        Log.i(TAG, "RX: channel sync received: ch=$remoteChannel from ${peer.device.name}")
                        // Optionally sync local channel (or just log for now)
                        continue
                    }

                    // Audio frame — pass to Rust for decoding and playback
                    SassyTalkNative.btDecodeFrame(payload)
                }
            } catch (e: IOException) {
                if (running.get()) {
                    Log.w(TAG, "RX thread error for ${peer.device.name}: ${e.message}")
                }
            }

            // Peer disconnected or errored — clean up
            removePeer(peer.device.address)
            Log.i(TAG, "RX thread stopped for ${peer.device.name}")
        }.start()
    }

    /**
     * Read exactly `count` bytes from the input stream (blocking).
     * Handles partial reads from RFCOMM sockets.
     */
    @Throws(IOException::class)
    private fun readFully(input: InputStream, buffer: ByteArray, count: Int) {
        var offset = 0
        while (offset < count) {
            val bytesRead = input.read(buffer, offset, count - offset)
            if (bytesRead == -1) {
                throw IOException("read failed, socket might closed or timeout, read ret: -1")
            }
            offset += bytesRead
        }
    }

    private fun removePeer(address: String) {
        val peer = connectedPeers.remove(address) ?: return
        peerCount.set(connectedPeers.size)

        try { peer.socket.close() } catch (_: IOException) {}

        Log.i(TAG, "Peer removed: ${peer.device.name} ($address), remaining: ${connectedPeers.size}")

        if (connectedPeers.isEmpty()) {
            state = State.DISCONNECTED
            SassyTalkNative.btDisconnected()
        }
    }

    /**
     * Periodically check for dead peers (no activity within timeout).
     */
    private var deadPeerMonitorRunning = AtomicBoolean(false)

    private fun startDeadPeerMonitor() {
        if (deadPeerMonitorRunning.getAndSet(true)) return

        Thread {
            Thread.currentThread().name = "bt-dead-peer"
            while (running.get() && connectedPeers.isNotEmpty()) {
                Thread.sleep(5_000)

                val now = System.currentTimeMillis()
                val deadPeers = connectedPeers.filter { (_, peer) ->
                    now - peer.lastActivity > DEAD_PEER_TIMEOUT_MS
                }.keys.toList()

                for (addr in deadPeers) {
                    Log.w(TAG, "Dead peer detected: $addr (no activity for ${DEAD_PEER_TIMEOUT_MS}ms)")
                    removePeer(addr)
                }
            }
            deadPeerMonitorRunning.set(false)
        }.start()
    }
}
