package com.sassyconsulting.sassytalkie

import android.util.Log
import kotlinx.coroutines.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import com.sassyconsulting.sassytalkie.service.BluetoothTransport

/**
 * PttCoordinator — Orchestrates BLE signaling + RFCOMM data for PTT.
 *
 * TX Flow (sender presses PTT):
 * 1. BLE: broadcastPttStart() to all peers (instant, 1 byte)
 * 2. Wait for READY_ACK from peers (200ms timeout)
 * 3. Native: start mic capture + ADPCM encode
 * 4. RFCOMM: TX pump reads encoded frames -> sends to peers
 * 5. On release: stop TX, BLE broadcastPttStop()
 *
 * RX Flow (receiver gets BLE signal):
 * 1. BleSignalingService.onPttStartReceived()
 * 2. Send READY_ACK via BLE
 * 3. RFCOMM RX is already running (started on connect)
 * 4. Audio will arrive and be decoded by the RX thread
 *
 * Cache-first RX (audio -> ring buffer -> drain thread -> AudioTrack)
 *
 * BLE PTT_STOP -> play roger beep
 *
 * Heartbeat loop (Task 2.2):
 * - Every 2s: encode HEARTBEAT TLV, broadcast via BLE
 * - On HEARTBEAT rx: update LivenessTracker (RTT, health, presence)
 * - Epoch change detected → re-send Capabilities to that peer
 * - onControlFrame() dispatches all TLV opcodes
 */
class PttCoordinator(
    private val bleSignaling: BleSignalingService,
    private val btTransport: BluetoothTransport
) : BleSignalingService.Listener {

    /** Optional cellular relay client — set after construction when relay connects. */
    var cellularClient: CellularWebSocketClient? = null

    /** Presence sensor — set after construction once a Context is available. */
    lateinit var presenceSensor: PresenceSensor

    private fun currentPresenceState(): PresenceState {
        if (::presenceSensor.isInitialized) presenceSensor.refresh()
        return if (::presenceSensor.isInitialized) presenceSensor.state.value else PresenceState.LISTENING
    }

    fun setMicMuted(muted: Boolean) {
        if (::presenceSensor.isInitialized) {
            presenceSensor.micMuted = muted
            presenceSensor.refresh()
        }
        // Immediate heartbeat push
        val now = System.currentTimeMillis()
        val frame = ControlFrame.encodeHeartbeat(selfEpoch, hbSeq.getAndIncrement(), now, currentPresenceState(), 0)
        bleSignaling.broadcastControl(frame)
        cellularClient?.sendBinary(frame)
    }

    companion object {
        private const val TAG = "PTT.Coord"
        private const val READY_ACK_TIMEOUT_MS = 200L
        private const val HEARTBEAT_INTERVAL_MS = 2_000L
    }

    private val transmitting = AtomicBoolean(false)
    private val readyAckCount = AtomicInteger(0)
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    // —— Heartbeat / Liveness (Task 2.2) ——

    val liveness = LivenessTracker()
    val selfEpoch: Long = SessionEpoch.generate()
    private var hbSeq = AtomicInteger(0)
    private var heartbeatJob: Job? = null

    /** Per-peer Capabilities we've received (keyed by peer device address). */
    private val peerCaps = ConcurrentHashMap<String, Capabilities>()

    init {
        bleSignaling.listener = this
        // Route raw control frames from BLE up to onControlFrame
        bleSignaling.controlFrameCallback = { peerId, bytes -> onControlFrame(peerId, bytes) }
        startHeartbeat()
    }

    // —— TX Side (We press PTT) ——

    fun onPttPressed() {
        if (transmitting.getAndSet(true)) return

        val blePeers = bleSignaling.blePeerCount
        val rfcommPeers = btTransport.connectedPeerCount

        Log.i(TAG, "PTT PRESSED \u2014 BLE peers: $blePeers, RFCOMM peers: $rfcommPeers")

        if (blePeers == 0 && rfcommPeers == 0) {
            Log.w(TAG, "PTT BLOCKED: No peers connected")
            transmitting.set(false)
            return
        }

        // Step 1: BLE signal to all peers
        readyAckCount.set(0)
        bleSignaling.broadcastPttStart()

        // Step 2: Brief wait for ACKs, then start audio regardless
        scope.launch {
            delay(READY_ACK_TIMEOUT_MS)
            val acks = readyAckCount.get()
            Log.i(TAG, "Got $acks/$blePeers READY_ACKs, proceeding")

            // Step 3: Start native audio (mic -> ADPCM -> transport)
            SassyTalkNative.pttStart()
            Log.i(TAG, "Native PTT started")

            // Step 4: Start RFCOMM TX pump
            if (rfcommPeers > 0) {
                btTransport.startTxPump()
            }
        }
    }

    fun onPttReleased() {
        if (!transmitting.getAndSet(false)) return

        Log.i(TAG, "PTT RELEASED")

        // Stop native audio
        SassyTalkNative.pttStop()

        // Stop RFCOMM TX
        btTransport.stopTxPump()

        // BLE signal to peers
        bleSignaling.broadcastPttStop()
    }

    // —— RX Side (Peer presses PTT, we receive) ——

    override fun onPttStartReceived(deviceAddress: String) {
        Log.i(TAG, "\u2190 PTT_START from $deviceAddress")

        // Send READY_ACK back
        bleSignaling.sendReadyAck(deviceAddress)

        // RFCOMM RX is already running (started on connect)
        // Audio will arrive and be decoded by the RX thread
    }

    override fun onPttStopReceived(deviceAddress: String) {
        Log.i(TAG, "\u2190 PTT_STOP from $deviceAddress")
        // TODO: Play roger beep
    }

    override fun onReadyAckReceived(deviceAddress: String) {
        val count = readyAckCount.incrementAndGet()
        Log.i(TAG, "\u2190 READY_ACK from $deviceAddress (total: $count)")
    }

    override fun onPeerDiscovered(device: android.bluetooth.BluetoothDevice) {
        Log.i(TAG, "Peer discovered: ${device.name ?: device.address}")
        // Auto-connect RFCOMM data channel when BLE peer found
        if (!btTransport.isConnectedTo(device.address)) {
            btTransport.connectDevice(device)
        }
    }

    override fun onPeerLost(deviceAddress: String) {
        Log.i(TAG, "Peer lost: $deviceAddress")
        liveness.removePeer(deviceAddress)
        peerCaps.remove(deviceAddress)
    }

    // —— Heartbeat Loop (Task 2.2) ——

    fun startHeartbeat() {
        if (heartbeatJob?.isActive == true) return
        heartbeatJob = scope.launch {
            Log.i(TAG, "Heartbeat loop started (epoch=$selfEpoch)")
            while (isActive) {
                val seq = hbSeq.getAndIncrement()
                val nowMs = System.currentTimeMillis()
                val frame = ControlFrame.encodeHeartbeat(
                    epoch  = selfEpoch,
                    seq    = seq,
                    tsMs   = nowMs,
                    state  = currentPresenceState(),
                    rttMs  = 0,
                )
                // Track sent heartbeat for RTT measurement per peer
                for (peerId in bleSignaling.blePeerAddresses) {
                    liveness.onHeartbeatSent(peerId, selfEpoch, seq, nowMs)
                }
                bleSignaling.broadcastControl(frame)
                cellularClient?.sendBinary(frame)
                Log.d(TAG, "HB seq=$seq broadcast to ${bleSignaling.blePeerCount} peers (relay=${cellularClient != null})")
                delay(HEARTBEAT_INTERVAL_MS)
            }
        }
    }

    fun stopHeartbeat() {
        heartbeatJob?.cancel()
        heartbeatJob = null
        Log.i(TAG, "Heartbeat loop stopped")
    }

    // —— Control Frame Dispatcher (Task 2.2) ——

    /**
     * Central TLV dispatcher. Called by BleSignalingService for every raw byte array
     * received on the PTT characteristic — both legacy single-byte commands and new TLV frames.
     *
     * Legacy opcodes 0x01–0x05: delegate to existing Listener methods.
     * OP_HEARTBEAT (0x10):       update liveness, check epoch, echo back.
     * OP_CAPABILITIES (0x13):    store peer caps.
     * Other new opcodes:         no-op for now (later phases add handlers).
     */
    fun onControlFrame(peerId: String, bytes: ByteArray) {
        if (bytes.isEmpty()) return
        val frame = ControlFrame.decode(bytes)

        when (frame.opcode) {
            // Legacy opcodes — delegate to existing handler methods (Listener contract preserved)
            ControlFrame.OP_PTT_START -> {
                Log.d(TAG, "onControlFrame: PTT_START from $peerId")
                onPttStartReceived(peerId)
            }
            ControlFrame.OP_PTT_STOP -> {
                Log.d(TAG, "onControlFrame: PTT_STOP from $peerId")
                onPttStopReceived(peerId)
            }
            ControlFrame.OP_READY_ACK -> {
                Log.d(TAG, "onControlFrame: READY_ACK from $peerId")
                onReadyAckReceived(peerId)
            }
            ControlFrame.OP_PING, ControlFrame.OP_CHANNEL_SYNC -> {
                // Handled elsewhere or no-op
            }

            // New TLV opcodes
            ControlFrame.OP_HEARTBEAT -> handleHeartbeat(peerId, frame.payload)
            ControlFrame.OP_CAPABILITIES -> handleCapabilities(peerId, frame.payload)
            ControlFrame.OP_PARTNER_OFFLINE -> {
                if (frame.payload.isNotEmpty()) {
                    val idLen = frame.payload[0].toInt() and 0xFF
                    if (frame.payload.size >= 1 + idLen) {
                        val offlinePeerId = String(frame.payload, 1, idLen, Charsets.UTF_8)
                        liveness.removePeer(offlinePeerId)
                        // Will be surfaced to UI in a later task
                        android.util.Log.w("PttCoord", "Partner offline: $offlinePeerId")
                    }
                }
            }

            else -> {
                // Future opcodes — no-op for now
                Log.d(TAG, "onControlFrame: unknown opcode=0x${frame.opcode.toInt().and(0xFF).toString(16)} from $peerId, ignoring")
            }
        }
    }

    private fun handleHeartbeat(peerId: String, payload: ByteArray) {
        if (payload.size < 23) {
            Log.w(TAG, "HB from $peerId: payload too short (${payload.size})")
            return
        }
        val hb = ControlFrame.parseHeartbeat(payload)
        val nowMs = System.currentTimeMillis()

        // Detect epoch change → peer restarted → re-send our Capabilities
        val epochFlipped = liveness.epochChanged(peerId, hb.epoch)

        liveness.onHeartbeat(peerId, hb.epoch, hb.seq, hb.tsMs, nowMs)
        liveness.updatePresence(peerId, hb.state)

        val health = liveness.health(peerId, nowMs)
        val rtt = liveness.rttMs(peerId)
        Log.d(TAG, "HB from $peerId seq=${hb.seq} epoch=${hb.epoch} state=${hb.state} health=$health rtt=${rtt}ms")

        if (epochFlipped) {
            Log.i(TAG, "Peer $peerId epoch changed → re-sending Capabilities")
            scope.launch { sendCapabilitiesToPeer(peerId) }
        }

        // Echo the heartbeat back (same seq) so the sender can measure RTT to us
        val echo = ControlFrame.encodeHeartbeat(
            epoch  = selfEpoch,
            seq    = hb.seq,      // echo same seq for RTT matching
            tsMs   = nowMs,
            state  = currentPresenceState(),
            rttMs  = rtt.coerceAtLeast(0),
        )
        bleSignaling.sendControl(peerId, echo)
    }

    private fun handleCapabilities(peerId: String, payload: ByteArray) {
        try {
            val caps = Capabilities.parse(payload)
            peerCaps[peerId] = caps
            Log.i(TAG, "Caps from $peerId: codec=${caps.codec} sr=${caps.sampleRate} audioV2=${caps.audioV2}")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to parse Capabilities from $peerId: $e")
        }
    }

    private fun sendCapabilitiesToPeer(peerId: String) {
        val caps = Capabilities(
            codec      = "adpcm",
            sampleRate = 8000,
            mute       = false,
            vol        = 100,
            battery    = -1,
            audioV2    = false,
            epoch      = selfEpoch,
        )
        bleSignaling.sendControl(peerId, caps.toFrame())
        Log.d(TAG, "Sent Capabilities to $peerId")
    }

    /** Retrieve cached Capabilities for a peer, or null if not yet received. */
    fun peerCapabilities(peerId: String): Capabilities? = peerCaps[peerId]

    // —— Lifecycle ——

    fun shutdown() {
        stopHeartbeat()
        scope.cancel()
        transmitting.set(false)
    }
}
