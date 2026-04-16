package com.sassyconsulting.sassytalkie

import android.util.Log
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
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
 * 5. On release: stop TX, BLE broadcastPttStop() + PTT_STOP_V2
 *
 * RX Flow (receiver gets BLE signal):
 * 1. BleSignalingService.onPttStartReceived()
 * 2. Send READY_ACK via BLE
 * 3. RFCOMM RX is already running (started on connect)
 * 4. Audio will arrive and be decoded by the RX thread
 * 5. On PTT_STOP_V2 received: wait 300ms, send EOT_ACK
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
 *
 * EOT_ACK "delivered" tick (Task 4.3):
 * - Sender emits PTT_STOP_V2 on release
 * - Receiver waits 300ms (jitter buffer drain) then sends EOT_ACK
 * - Sender sets deliveredState = Delivered, then resets to Idle after 3s
 */

enum class DeliveryState { Idle, Sending, Delivered }
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
        private const val RECV_ACK_INTERVAL_MS = 500L
        private const val REACHING_PEER_TIMEOUT_MS = 1_000L

        // Task 7.1 — Sub-audible audio path probe marker
        const val PROBE_EPOCH    = -1L  // 0xFFFFFFFFFFFFFFFF as signed Long
        const val PROBE_SEQ      = -1   // 0xFFFFFFFF as signed Int
        const val PROBE_ECHO_SEQ = -2   // Echo response marker (PROBE_SEQ - 1)
    }

    private val transmitting = AtomicBoolean(false)
    private val readyAckCount = AtomicInteger(0)
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    // —— Delivery State (Task 4.3) — EOT_ACK "delivered" tick ——

    val deliveredState = MutableStateFlow(DeliveryState.Idle)

    /** Last transmitted audio frame seq (updated by txFrameCallback). */
    @Volatile private var lastTxSeq: Int = 0
    /** Last transmitted audio frame epoch (updated by txFrameCallback). */
    @Volatile private var lastTxEpoch: Long = 0L
    /** Timeout job: reset deliveredState to Idle if no EOT_ACK within 2s. */
    private var eotTimeoutJob: Job? = null
    /** Reset job: set deliveredState back to Idle 3s after Delivered. */
    private var deliveredResetJob: Job? = null

    // —— Heartbeat / Liveness (Task 2.2) ——

    val liveness = LivenessTracker()
    val selfEpoch: Long = SessionEpoch.generate()
    private var hbSeq = AtomicInteger(0)
    private var heartbeatJob: Job? = null

    // —— Audio Path Probe (Task 7.1) ——

    /** True when the last probe round-trip exceeded 400ms or timed out entirely. */
    val audioPathDegraded = MutableStateFlow(false)

    /** Timestamp (ms) when the probe frame was sent; 0 means no probe in-flight. */
    @Volatile private var probeSentMs = 0L

    /** Timeout job for in-flight probe — cancelled when echo arrives. */
    private var probeTimeoutJob: Job? = null

    // —— Stale-peer banner (Task 6.2) ——

    /** True when at least one connected peer has health == STALE. */
    val anyPeerStale = MutableStateFlow(false)

    // —— Talk-over indicator (Task 6.2) ——

    /** True while a peer is actively transmitting (OP_PTT_START / OP_PTT_START_V2 received). */
    val peerSpeaking = MutableStateFlow(false)

    /** Job that auto-clears peerSpeaking after 400ms of silence (no audio frames). */
    private var peerSpeakingTimeoutJob: Job? = null

    // —— RECV_ACK (Task 4.2) — Receiver side ——

    /** Last received audio frame epoch (updated on each incoming audio frame). */
    @Volatile private var lastRxEpoch: Long = 0L
    /** Last received audio frame sequence number (updated on each incoming audio frame). */
    @Volatile private var lastRxSeq: Int = 0
    /** Peer that sent the most recent audio frame. */
    @Volatile private var lastRxPeerId: String? = null
    /** Coroutine job that sends RECV_ACK every 500ms while audio is arriving. */
    private var recvAckJob: Job? = null

    // —— Reaching-Peer indicator (Task 4.2) — Sender side ——

    /** True while PTT is held and at least one RECV_ACK arrived within the last second. */
    private val _reachingPeer = MutableStateFlow(false)
    val reachingPeer = _reachingPeer.asStateFlow()

    /** Timestamp (ms) of the most recent OP_RECV_ACK received from any peer. */
    @Volatile private var lastAckMs: Long = 0L
    /** Watchdog coroutine while PTT is held — clears reachingPeer if no ACK for 1s. */
    private var watchdogJob: Job? = null

    /** Per-peer Capabilities we've received (keyed by peer device address). */
    private val peerCaps = ConcurrentHashMap<String, Capabilities>()

    init {
        bleSignaling.listener = this
        // Route raw control frames from BLE up to onControlFrame
        bleSignaling.controlFrameCallback = { peerId, bytes -> onControlFrame(peerId, bytes) }
        // Track incoming audio frames for RECV_ACK (Task 4.2) and probe detection (Task 7.1)
        btTransport.audioFrameV2Callback = { peerId, decoded -> onAudioFrameV2(peerId, decoded) }
        btTransport.audioFrameCallback = { peerId, epoch, seq -> onAudioFrameReceived(peerId, epoch, seq) }
        // Track outgoing audio frames for PTT_STOP_V2 / EOT_ACK (Task 4.3)
        btTransport.txFrameCallback = { epoch, seq ->
            lastTxEpoch = epoch
            lastTxSeq = seq
        }
        startHeartbeat()
    }

    // —— TX Side (We press PTT) ——

    /**
     * Lightweight PTT-press notification from the UI — updates delivery state only.
     * Audio and BLE signaling are handled separately by the direct SassyTalkNative calls in MainScreen.
     */
    fun notifyPttPressed() {
        eotTimeoutJob?.cancel()
        deliveredResetJob?.cancel()
        deliveredState.value = DeliveryState.Sending
        Log.d(TAG, "notifyPttPressed → deliveredState=Sending")
    }

    /**
     * Lightweight PTT-release notification from the UI — emits PTT_STOP_V2 and starts EOT_ACK timeout.
     * Audio stop is handled separately by the direct SassyTalkNative call in MainScreen.
     */
    fun notifyPttReleased() {
        val stopEpoch = if (lastTxEpoch != 0L) lastTxEpoch else selfEpoch
        val stopSeq = lastTxSeq
        val pttStopV2 = ControlFrame.encodePttStopV2(stopEpoch, stopSeq)
        bleSignaling.broadcastControl(pttStopV2)
        cellularClient?.sendBinary(pttStopV2)
        Log.d(TAG, "notifyPttReleased: PTT_STOP_V2 epoch=$stopEpoch seq=$stopSeq")

        eotTimeoutJob?.cancel()
        eotTimeoutJob = scope.launch {
            delay(2_000L)
            if (deliveredState.value == DeliveryState.Sending) {
                deliveredState.value = DeliveryState.Idle
                Log.d(TAG, "EOT_ACK timeout — deliveredState reset to Idle")
            }
        }
    }

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

        // Task 7.1: send sub-audible audio path probe before real audio starts
        sendAudioProbe()

        // Reset reaching-peer state on press
        _reachingPeer.value = false
        lastAckMs = 0L
        startWatchdog()

        // Mark delivery as Sending (Task 4.3)
        eotTimeoutJob?.cancel()
        deliveredResetJob?.cancel()
        deliveredState.value = DeliveryState.Sending

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

        // Stop watchdog and reset reaching-peer indicator
        stopWatchdog()
        _reachingPeer.value = false

        // Stop native audio
        SassyTalkNative.pttStop()

        // Stop RFCOMM TX
        btTransport.stopTxPump()

        // BLE signal to peers (legacy)
        bleSignaling.broadcastPttStop()

        // Emit PTT_STOP_V2 with epoch + lastTxSeq for EOT_ACK delivery tick (Task 4.3)
        val stopEpoch = if (lastTxEpoch != 0L) lastTxEpoch else selfEpoch
        val stopSeq = lastTxSeq
        val pttStopV2 = ControlFrame.encodePttStopV2(stopEpoch, stopSeq)
        bleSignaling.broadcastControl(pttStopV2)
        cellularClient?.sendBinary(pttStopV2)
        Log.d(TAG, "PTT_STOP_V2 epoch=$stopEpoch seq=$stopSeq broadcast")

        // Start 2s timeout: if no EOT_ACK, reset to Idle
        eotTimeoutJob?.cancel()
        eotTimeoutJob = scope.launch {
            delay(2_000L)
            if (deliveredState.value == DeliveryState.Sending) {
                deliveredState.value = DeliveryState.Idle
                Log.d(TAG, "EOT_ACK timeout — deliveredState reset to Idle")
            }
        }
    }

    // —— Audio Path Probe (Task 7.1) ——

    /**
     * Send a sub-audible audio path probe BEFORE real audio starts.
     * Uses a reserved epoch/seq marker so receivers can distinguish it from real audio.
     * Measures round-trip time; sets audioPathDegraded if RTT > 400ms or timeout at 800ms.
     */
    fun sendAudioProbe() {
        val silence = ByteArray(320) // 20ms at 16kHz 16-bit mono
        val frame = AudioFrameV2.encode(PROBE_EPOCH, PROBE_SEQ, silence)
        probeSentMs = System.currentTimeMillis()
        audioPathDegraded.value = false
        Log.d(TAG, "Probe sent at $probeSentMs")
        // Send through BLE only — probes must NOT go through the cellular relay
        btTransport.sendRaw(frame)
        // Timeout check: if no echo within 800ms, mark degraded
        probeTimeoutJob?.cancel()
        probeTimeoutJob = scope.launch {
            delay(800)
            if (probeSentMs > 0) {
                audioPathDegraded.value = true
                probeSentMs = 0
                Log.w(TAG, "Probe timeout — audioPathDegraded=true")
            }
        }
    }

    /**
     * Called by BluetoothTransport for every incoming V2 audio frame (full decoded).
     * Handles probe marker detection:
     *  - PROBE_EPOCH + PROBE_SEQ      → we are the receiver; echo it back
     *  - PROBE_EPOCH + PROBE_ECHO_SEQ → we are the sender; measure RTT
     * All other frames are handled by the normal RECV_ACK path.
     */
    fun onAudioFrameV2(peerId: String, decoded: AudioV2Decoded) {
        when {
            decoded.epoch == PROBE_EPOCH && decoded.seq == PROBE_SEQ -> {
                // We are the receiver — echo back via BLE only, NOT the relay
                Log.d(TAG, "Probe received from $peerId — echoing back via BLE")
                val echoFrame = AudioFrameV2.encode(PROBE_EPOCH, PROBE_ECHO_SEQ, ByteArray(0))
                btTransport.sendRaw(echoFrame)
            }
            decoded.epoch == PROBE_EPOCH && decoded.seq == PROBE_ECHO_SEQ -> {
                // We are the sender — measure RTT (drop if no probe in flight)
                if (probeSentMs == 0L) {
                    Log.w(TAG, "Spurious probe echo from $peerId — no probe in flight, dropping")
                    return
                }
                val rtt = System.currentTimeMillis() - probeSentMs
                probeSentMs = 0
                probeTimeoutJob?.cancel()
                probeTimeoutJob = null
                audioPathDegraded.value = rtt > 400
                Log.d(TAG, "Probe echo RTT=${rtt}ms → degraded=${audioPathDegraded.value}")
            }
            else -> {
                // Normal audio frame — update RECV_ACK tracking
                lastRxSeq = decoded.seq
            }
        }
    }

    // —— RX Side (Peer presses PTT, we receive) ——

    override fun onPttStartReceived(deviceAddress: String) {
        Log.i(TAG, "\u2190 PTT_START from $deviceAddress")

        // Send READY_ACK back
        bleSignaling.sendReadyAck(deviceAddress)

        // Mark peer as speaking (Task 6.2)
        setPeerSpeaking(true)

        // RFCOMM RX is already running (started on connect)
        // Audio will arrive and be decoded by the RX thread
    }

    override fun onPttStopReceived(deviceAddress: String) {
        Log.i(TAG, "\u2190 PTT_STOP from $deviceAddress")
        stopRecvAckJob()
        // Clear peer speaking indicator (Task 6.2)
        setPeerSpeaking(false)
        // TODO: Play roger beep
    }

    /** Set peerSpeaking = true and arm a 400ms auto-clear timeout. */
    private fun setPeerSpeaking(speaking: Boolean) {
        peerSpeaking.value = speaking
        peerSpeakingTimeoutJob?.cancel()
        if (speaking) {
            peerSpeakingTimeoutJob = scope.launch {
                delay(400L)
                peerSpeaking.value = false
                Log.d(TAG, "peerSpeaking auto-cleared after 400ms timeout")
            }
        } else {
            peerSpeakingTimeoutJob = null
        }
    }

    /** Called when a V2 audio frame arrives — resets the 400ms speaking timeout. */
    fun onPeerAudioFrame() {
        if (peerSpeaking.value) {
            // Extend timeout — cancel and rearm
            peerSpeakingTimeoutJob?.cancel()
            peerSpeakingTimeoutJob = scope.launch {
                delay(400L)
                peerSpeaking.value = false
                Log.d(TAG, "peerSpeaking auto-cleared after 400ms audio silence")
            }
        }
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
            var tickCount = 0
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

                // Poll stale status every 1s (heartbeat fires every 2s, check every tick)
                tickCount++
                val peerIds = liveness.peerIds()
                val stale = peerIds.isNotEmpty() && peerIds.any { liveness.health(it, nowMs) == PeerHealth.STALE }
                if (anyPeerStale.value != stale) anyPeerStale.value = stale

                delay(HEARTBEAT_INTERVAL_MS)
            }
        }
        // Also run a 1s stale-check loop independent of the 2s heartbeat
        scope.launch {
            while (true) {
                delay(1_000L)
                val nowMs = System.currentTimeMillis()
                val peerIds = liveness.peerIds()
                val stale = peerIds.isNotEmpty() && peerIds.any { liveness.health(it, nowMs) == PeerHealth.STALE }
                if (anyPeerStale.value != stale) anyPeerStale.value = stale
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
        val frame = ControlFrame.decode(bytes) ?: return

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
            ControlFrame.OP_PTT_START_V2 -> {
                Log.d(TAG, "onControlFrame: PTT_START_V2 from $peerId")
                setPeerSpeaking(true)
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
            ControlFrame.OP_RECV_ACK -> handleRecvAck(peerId, frame.payload)
            ControlFrame.OP_CAPABILITIES -> handleCapabilities(peerId, frame.payload)
            ControlFrame.OP_PTT_STOP_V2 -> handlePttStopV2(peerId, frame.payload)
            ControlFrame.OP_EOT_ACK -> handleEotAck(peerId, frame.payload)
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

    // —— RECV_ACK — Receiver side (Task 4.2) ——

    /**
     * Called by BluetoothTransport whenever a V2 audio frame arrives.
     * Updates lastRxEpoch/lastRxSeq and (re)starts the 500ms RECV_ACK loop.
     */
    private fun onAudioFrameReceived(peerId: String, epoch: Long, seq: Int) {
        lastRxEpoch = epoch
        lastRxSeq = seq
        lastRxPeerId = peerId
        // Extend peer-speaking timeout on each incoming audio frame (Task 6.2)
        onPeerAudioFrame()
        // Ensure the ACK loop is running
        if (recvAckJob?.isActive != true) {
            recvAckJob = scope.launch {
                Log.d(TAG, "RECV_ACK loop started for $peerId epoch=$epoch")
                while (isActive) {
                    val ackEpoch = lastRxEpoch
                    val ackSeq = lastRxSeq
                    val ackPeer = lastRxPeerId ?: break
                    val frame = ControlFrame.encodeRecvAck(ackEpoch, ackSeq, System.currentTimeMillis())
                    bleSignaling.sendControl(ackPeer, frame)
                    cellularClient?.sendBinary(frame)
                    Log.d(TAG, "RECV_ACK sent epoch=$ackEpoch seq=$ackSeq to $ackPeer")
                    delay(RECV_ACK_INTERVAL_MS)
                }
                Log.d(TAG, "RECV_ACK loop stopped")
            }
        }
    }

    /** Stop the RECV_ACK coroutine (called when PTT session ends). */
    private fun stopRecvAckJob() {
        recvAckJob?.cancel()
        recvAckJob = null
    }

    // —— PTT_STOP_V2 / EOT_ACK — Receiver + Sender (Task 4.3) ——

    /**
     * Receiver side: peer has released PTT and sent PTT_STOP_V2.
     * Wait 300ms for jitter buffer to drain, then send EOT_ACK back.
     */
    private fun handlePttStopV2(peerId: String, payload: ByteArray) {
        stopRecvAckJob()
        if (payload.size < 12) {
            Log.w(TAG, "PTT_STOP_V2 from $peerId: payload too short (${payload.size})")
            return
        }
        val bb = java.nio.ByteBuffer.wrap(payload).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        val epoch = bb.long
        val endSeq = bb.int
        Log.d(TAG, "PTT_STOP_V2 from $peerId epoch=$epoch endSeq=$endSeq — will EOT_ACK in 300ms")
        // Clear peer-speaking indicator (Task 6.2)
        setPeerSpeaking(false)
        scope.launch {
            delay(300L) // jitter buffer drain
            val ack = ControlFrame.encodeEotAck(epoch, endSeq)
            bleSignaling.sendControl(peerId, ack)
            cellularClient?.sendBinary(ack)
            Log.d(TAG, "EOT_ACK sent to $peerId epoch=$epoch upToSeq=$endSeq")
        }
    }

    /**
     * Sender side: received EOT_ACK — the peer confirmed they got our audio.
     * Set deliveredState = Delivered, then reset to Idle after 3s.
     */
    private fun handleEotAck(peerId: String, payload: ByteArray) {
        if (payload.size < 12) {
            Log.w(TAG, "EOT_ACK from $peerId: payload too short (${payload.size})")
            return
        }
        val bb = java.nio.ByteBuffer.wrap(payload).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        val ackEpoch = bb.long
        val ackSeq = bb.int
        val expectedEpoch = if (lastTxEpoch != 0L) lastTxEpoch else selfEpoch
        if (ackEpoch != expectedEpoch || ackSeq < lastTxSeq) {
            Log.w(TAG, "Stale/spoofed EOT_ACK: epoch=$ackEpoch seq=$ackSeq, expected epoch=$expectedEpoch seq>=$lastTxSeq")
            return
        }
        Log.d(TAG, "EOT_ACK from $peerId epoch=$ackEpoch upToSeq=$ackSeq → Delivered")
        eotTimeoutJob?.cancel()
        deliveredState.value = DeliveryState.Delivered
        deliveredResetJob?.cancel()
        deliveredResetJob = scope.launch {
            delay(3_000L)
            deliveredState.value = DeliveryState.Idle
            Log.d(TAG, "deliveredState reset to Idle after 3s")
        }
    }

    // —— Reaching-Peer Watchdog — Sender side (Task 4.2) ——

    /**
     * Handle an incoming OP_RECV_ACK while we are transmitting.
     * Sets reachingPeer = true and records the timestamp.
     */
    private fun handleRecvAck(peerId: String, payload: ByteArray) {
        if (payload.size < 20) {
            Log.w(TAG, "RECV_ACK from $peerId: payload too short (${payload.size})")
            return
        }
        val (epoch, seq, tsMs) = ControlFrame.parseRecvAck(payload)
        Log.d(TAG, "RECV_ACK from $peerId epoch=$epoch seq=$seq ts=$tsMs")
        lastAckMs = System.currentTimeMillis()
        _reachingPeer.value = true
    }

    /** Start the watchdog coroutine while PTT is held. */
    private fun startWatchdog() {
        watchdogJob?.cancel()
        watchdogJob = scope.launch {
            Log.d(TAG, "Reaching-peer watchdog started")
            while (isActive) {
                delay(200L)
                val elapsed = System.currentTimeMillis() - lastAckMs
                if (lastAckMs > 0L && elapsed > REACHING_PEER_TIMEOUT_MS) {
                    _reachingPeer.value = false
                    Log.d(TAG, "Reaching-peer: no ACK for ${elapsed}ms → false")
                }
            }
        }
    }

    /** Stop the watchdog coroutine. */
    private fun stopWatchdog() {
        watchdogJob?.cancel()
        watchdogJob = null
        Log.d(TAG, "Reaching-peer watchdog stopped")
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
        stopRecvAckJob()
        stopWatchdog()
        _reachingPeer.value = false
        audioPathDegraded.value = false
        probeSentMs = 0
        probeTimeoutJob?.cancel()
        probeTimeoutJob = null
        eotTimeoutJob?.cancel()
        deliveredResetJob?.cancel()
        deliveredState.value = DeliveryState.Idle
        scope.cancel()
        transmitting.set(false)
    }
}
