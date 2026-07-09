package com.sassyconsulting.sassytalkie.service

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.util.Log
import com.sassyconsulting.sassytalkie.AudioFrameV2
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

        // Channel-sync control message framing. A plain [0xFF,0xFF,channel]
        // 3-byte payload collides with any legitimate 3-byte audio/control
        // frame whose first two bytes happen to be 0xFF. Use a reserved 3-byte
        // magic (0xFF 0xFF 0x53 = 'S') plus a dedicated type byte 0x01, so the
        // sync message is a fixed 5-byte payload that audio frames cannot match.
        // Layout: [0xFF][0xFF][0x53][0x01][channel].
        private val CHANNEL_SYNC_MAGIC = byteArrayOf(0xFF.toByte(), 0xFF.toByte(), 0x53.toByte())
        private const val CHANNEL_SYNC_TYPE: Byte = 0x01
        private const val CHANNEL_SYNC_LEN = 5

        // Dead peer detection timeout (ms)
        private const val DEAD_PEER_TIMEOUT_MS = 10_000L
    }

    // ── State ──

    enum class State { DISCONNECTED, CONNECTING, CONNECTED }

    @Volatile
    var state: State = State.DISCONNECTED
        private set

    // Single lock guarding all `state` transitions. Transitions are compound
    // (they depend on whether `connectedPeers` is empty) so plain @Volatile
    // visibility is not enough — concurrent connect/disconnect paths could
    // otherwise clobber each other (e.g. report CONNECTED with no peers, or a
    // racing failed-connect overwriting a successful one). All writes to
    // `state` must go through the helpers below.
    private val stateLock = Any()

    /**
     * Recompute `state` from the authoritative source of truth (`connectedPeers`).
     * CONNECTED iff at least one peer is present, otherwise DISCONNECTED. Never
     * downgrades or fabricates CONNECTING. Safe to call from any thread.
     */
    private fun refreshState() {
        synchronized(stateLock) {
            state = if (connectedPeers.isEmpty()) State.DISCONNECTED else State.CONNECTED
        }
    }

    /**
     * Mark CONNECTING only while genuinely idle (no peers yet). If peers already
     * exist we keep CONNECTED rather than regressing to CONNECTING — a new
     * connect attempt must not visually disconnect existing peers.
     */
    private fun markConnecting() {
        synchronized(stateLock) {
            if (connectedPeers.isEmpty()) {
                state = State.CONNECTING
            }
        }
    }

    /** Force DISCONNECTED (used by shutdown, which tears down all peers). */
    private fun forceDisconnected() {
        synchronized(stateLock) {
            state = State.DISCONNECTED
        }
    }

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

    // TX epoch/seq bookkeeping for the PTT_STOP_V2 → EOT_ACK "Delivered" tick.
    // The wire audio payload is encrypted (AES-GCM ciphertext) so we can't
    // recover seq by decoding the frame on the way out. Track it locally
    // instead — the seq only has to be monotonic and echoed by the receiver.
    @Volatile private var txEpoch: Long = 0L
    private val txSeqCounter = AtomicInteger(0)

    // Server socket for incoming connections
    private var serverSocket: BluetoothServerSocket? = null

    // ── Public API ──

    val isConnected: Boolean get() = connectedPeers.isNotEmpty()
    val connectedPeerCount: Int get() = peerCount.get()
    val peerNames: List<String> get() = connectedPeers.values.map { it.device.name ?: it.device.address }

    /**
     * Optional callback invoked whenever a V2 audio frame is received.
     * Called on the RX thread with (peerId, epoch, seq).
     * Used by PttCoordinator to track lastRxEpoch/lastRxSeq for RECV_ACK.
     */
    var audioFrameCallback: ((peerId: String, epoch: Long, seq: Int) -> Unit)? = null

    /**
     * Optional callback invoked whenever a V2 audio frame is received, with full decoded frame.
     * Called on the RX thread with (peerId, decoded).
     * Used by PttCoordinator for probe detection (Task 7.1).
     */
    var audioFrameV2Callback: ((peerId: String, decoded: com.sassyconsulting.sassytalkie.AudioV2Decoded) -> Unit)? = null

    /**
     * Optional callback invoked whenever a V2 audio frame is successfully transmitted.
     * Called on the TX pump thread with (epoch, seq).
     * Used by PttCoordinator to track lastTxSeq for PTT_STOP_V2 / EOT_ACK.
     */
    var txFrameCallback: ((epoch: Long, seq: Int) -> Unit)? = null

    /**
     * Set the session epoch used when reporting outbound audio frames via
     * [txFrameCallback]. Called by PttCoordinator with its selfEpoch so the
     * receiver's EOT_ACK (echoing our epoch+seq) can be matched on our side.
     * Resets the seq counter — subsequent TX frames start at seq 0.
     */
    fun setTxEpoch(epoch: Long) {
        txEpoch = epoch
        txSeqCounter.set(0)
    }

    /** Check if we have an active RFCOMM connection to a specific device */
    fun isConnectedTo(address: String): Boolean = connectedPeers.containsKey(address)

    /** Connect RFCOMM to a device (auto-connect from BLE peer discovery) */
    fun connectDevice(device: BluetoothDevice) {
        Thread {
            Thread.currentThread().name = "bt-connect-${device.address.takeLast(5)}"
            connectTo(device)
        }.start()
    }

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

        markConnecting()
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

        refreshState()
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
                val acceptStartNs = System.nanoTime()
                try {
                    // Null server socket → nothing to accept on; stop rather than
                    // spin on `continue`.
                    val socket = serverSocket?.accept(30_000) ?: break
                    val device = socket.remoteDevice
                    Log.i(TAG, "Accepted connection from ${device.name} (${device.address})")
                    onPeerConnected(device, socket)
                } catch (e: IOException) {
                    if (!running.get()) break
                    // A genuine 30s-elapsed accept throws a "timeout" and we just
                    // re-arm. But a bad/closed socket makes accept() throw almost
                    // immediately, and retrying with no delay spins this thread at
                    // 100% CPU — hundreds of BluetoothSocket.accept() calls/ms
                    // (battery drain + log flood). If it returned far faster than
                    // the timeout, treat it as an error and back off before retry.
                    val elapsedMs = (System.nanoTime() - acceptStartNs) / 1_000_000
                    if (elapsedMs < 5_000) {
                        if (!e.message.orEmpty().contains("timeout", ignoreCase = true)) {
                            Log.w(TAG, "Accept error (${elapsedMs}ms): ${e.message}")
                        }
                        try {
                            Thread.sleep(1000)
                        } catch (ie: InterruptedException) {
                            Thread.currentThread().interrupt()
                            break
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

            // Reuse a single 4-byte header buffer across the audio loop. At ~50
            // frames/sec a per-frame ByteBuffer allocation is a steady GC source
            // for a thread that runs for the full duration of every PTT press.
            val headerArr = ByteArray(FRAME_HEADER_SIZE)
            val headerBb = ByteBuffer.wrap(headerArr).order(ByteOrder.LITTLE_ENDIAN)

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

                // Guard frame size against the RX-side limit. The receiver
                // rejects any frame with length <= 0 or > MAX_FRAME_SIZE, so
                // sending one would be unreadable by peers (and risks receiver
                // buffer issues). Skip it rather than transmitting garbage.
                if (frameData.size <= 0 || frameData.size > MAX_FRAME_SIZE) {
                    Log.w(TAG, "BT TX pump: dropping oversized/empty frame (${frameData.size} bytes, max $MAX_FRAME_SIZE)")
                    continue
                }

                // Write length-prefixed frame to all connected peers
                headerBb.clear()
                headerBb.putInt(frameData.size)
                val header = headerArr

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

                // Report (epoch, seq) for PTT_STOP_V2 / EOT_ACK tracking. The
                // audio payload itself is encrypted so we can't recover this
                // by decoding the frame — maintain it as plaintext plumbing
                // metadata instead. The seq is purely local and only has to
                // round-trip through the receiver's echo intact.
                val seq = txSeqCounter.getAndIncrement()
                txFrameCallback?.invoke(txEpoch, seq)

                // Cleanup dead peers
                deadPeers.forEach { removePeer(it) }
            }

            txPumpRunning.set(false)
            // If the pump exited because all peers dropped, make sure `state`
            // reflects DISCONNECTED rather than a stale CONNECTED.
            refreshState()
            Log.i(TAG, "BT TX pump stopped")
        }.start()
    }

    /** Stop the TX pump (called on PTT release) */
    fun stopTxPump() {
        txPumpRunning.set(false)
    }

    /**
     * Send a raw byte array directly to all connected peers (no length prefix added).
     * The caller is responsible for including any required framing.
     * Used by probe/heartbeat paths that pre-build the full wire frame.
     */
    fun sendRaw(frame: ByteArray) {
        val deadPeers = mutableListOf<String>()
        for ((addr, peer) in connectedPeers) {
            try {
                synchronized(peer.output) {
                    peer.output.write(frame)
                    peer.output.flush()
                }
                peer.lastActivity = System.currentTimeMillis()
            } catch (e: IOException) {
                Log.w(TAG, "sendRaw write failed for ${peer.device.name}: ${e.message}")
                deadPeers.add(addr)
            }
        }
        deadPeers.forEach { removePeer(it) }
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

        forceDisconnected()
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

        // Dedup the symmetric-connect race. BLE discovery fires on BOTH peers, so
        // each may dial RFCOMM at the same instant: our outgoing connectTo() and
        // their connection landing on our accept thread can both reach this method
        // for the same address. putIfAbsent picks one winner atomically; the loser
        // closes its socket and bails. The previous `connectedPeers[addr] = peer`
        // overwrote the entry, leaking the first socket + its RX thread and
        // double-playing every received frame.
        val existing = connectedPeers.putIfAbsent(addr, peer)
        if (existing != null) {
            Log.i(TAG, "Duplicate RFCOMM link to ${device.name ?: addr} — closing late socket")
            try { socket.close() } catch (_: IOException) {}
            return
        }
        peerCount.set(connectedPeers.size)
        refreshState()

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
     * Format: [0xFF][0xFF][0x53][0x01][channel:1] — a reserved 5-byte control
     * frame (magic + type byte) that real audio frames cannot collide with.
     */
    private fun sendChannelSync(peer: ConnectedPeer) {
        try {
            val channel = SassyTalkNative.getChannel()
            val syncMsg = byteArrayOf(
                CHANNEL_SYNC_MAGIC[0], CHANNEL_SYNC_MAGIC[1], CHANNEL_SYNC_MAGIC[2],
                CHANNEL_SYNC_TYPE, channel.toByte()
            )
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

                    // Check if it's a channel sync message: reserved 5-byte
                    // control frame [0xFF][0xFF][0x53][0x01][channel]. The full
                    // magic + type byte makes this unambiguous so real audio
                    // frames (even 5-byte ones) cannot be misread as sync.
                    if (frameLen == CHANNEL_SYNC_LEN &&
                        payload[0] == CHANNEL_SYNC_MAGIC[0] &&
                        payload[1] == CHANNEL_SYNC_MAGIC[1] &&
                        payload[2] == CHANNEL_SYNC_MAGIC[2] &&
                        payload[3] == CHANNEL_SYNC_TYPE) {
                        val remoteChannel = payload[4].toInt() and 0xFF
                        Log.i(TAG, "RX: channel sync received: ch=$remoteChannel from ${peer.device.name}")
                        // Optionally sync local channel (or just log for now)
                        continue
                    }

                    // Probe frames are sent via sendRaw() as unencrypted V2-framed
                    // bytes: the V2 length field IS the RFCOMM length header, so the
                    // full V2 frame is `headerBuf + payload`. Regular audio frames
                    // are AES-GCM ciphertext and will NOT have a real V2 epoch — so
                    // we only treat a frame as V2 if the decoded epoch matches the
                    // reserved probe-marker sentinel (-1L). Otherwise ciphertext
                    // whose random leading bytes happen to parse as a valid V2
                    // frame would pollute lastRxSeq/lastRxEpoch and trigger bogus
                    // RECV_ACKs with garbage values.
                    val fullFrame = ByteArray(FRAME_HEADER_SIZE + frameLen)
                    System.arraycopy(headerBuf, 0, fullFrame, 0, FRAME_HEADER_SIZE)
                    System.arraycopy(payload, 0, fullFrame, FRAME_HEADER_SIZE, frameLen)
                    if (AudioFrameV2.isV2(fullFrame)) {
                        val frame = AudioFrameV2.decode(fullFrame)
                        if (frame != null && frame.epoch == -1L &&
                            (frame.seq == -1 || frame.seq == -2)) {
                            // Probe request or echo — fire V2 callbacks and skip
                            // regular-audio playback path entirely.
                            audioFrameV2Callback?.invoke(peer.device.address, frame)
                            audioFrameCallback?.invoke(peer.device.address, frame.epoch, frame.seq)
                            continue
                        }
                    }

                    // Audio frame — pass to Rust for decoding and playback.
                    // BT wire frames carry their own (sender_id, timestamp) metadata
                    // but no V2 (epoch, seq); the encrypted payload can't be probed
                    // for those without false positives (see comment above). Fire the
                    // callback with sentinel 0/0 so the coordinator can still update
                    // lastRxPeerId, kick the RECV_ACK loop, and re-arm the
                    // peer-speaking UI timeout — handleRecvAck only uses receipt
                    // (not the values) for the reachingPeer signal.
                    if (SassyTalkNative.btDecodeFrame(payload)) {
                        audioFrameCallback?.invoke(peer.device.address, 0L, 0)
                    }
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

        refreshState()
        if (connectedPeers.isEmpty()) {
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
