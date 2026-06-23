package com.sassyconsulting.sassytalkie

import android.util.Base64
import android.util.Log
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import com.sassyconsulting.sassytalkie.service.BluetoothTransport

/**
 * v2.7.1 peer-roster event.
 * Emitted from the 1 Hz stale-check loop whenever the active peer set changes.
 * Active = HEALTHY or DEGRADED; transitions to STALE fire [Left].
 */
sealed class PeerEvent {
    data class Joined(val peerId: String) : PeerEvent()
    data class Left(val peerId: String) : PeerEvent()
}

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
        val frame = ControlFrame.encodeHeartbeat(selfEpoch, hbSeq.getAndIncrement(), now, currentPresenceState(), 0, SassyTalkNative.localCapabilities())
        bleSignaling.broadcastControl(frame)
        cellularClient?.sendBinary(frame)
    }

    companion object {
        private const val TAG = "PTT.Coord"
        private const val READY_ACK_TIMEOUT_MS = 200L
        private const val HEARTBEAT_INTERVAL_MS = 2_000L
        /** Min gap between successive OP_WAKE broadcasts. Cheap frame, but
         *  emitting on every key-down would still be wasteful on a hot mic. */
        private const val WAKE_COOLDOWN_MS = 2_000L
        /** Extra delay before starting audio when we just emitted a wake — gives
         *  woken peers a beat to re-handshake / fire their first HB back so the
         *  initial talk burst isn't dropped on stale transports. */
        private const val WAKE_PRE_AUDIO_DELAY_MS = 350L
        // Send RECV_ACK once per ~1.5s instead of every 500ms. The ACK
        // shares the Bluetooth controller with RFCOMM audio; firing every
        // 500ms while audio is in flight caused 50-200ms RFCOMM stalls on
        // most chipsets (audible as garble/dropouts during BT fallback).
        private const val RECV_ACK_INTERVAL_MS = 1_500L
        // Bumped from 1s to 2.5s so one missed ACK at the new 1.5s interval
        // doesn't continuously flag reachingPeer=false. With the slower
        // ACK cadence we need at most one missed packet of slack.
        private const val REACHING_PEER_TIMEOUT_MS = 2_500L

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
    private var staleCheckJob: Job? = null

    // —— Wake beacon ——
    @Volatile private var lastWakeSentMs: Long = 0L

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

    // —— v2.7.1: Peer roster + join/leave events ——

    /** Set of currently-known peer IDs (HEALTHY or DEGRADED — excludes STALE
     *  so the chip reflects "who can hear me right now"). Polled at the
     *  same 1 Hz cadence as the stale-check loop. */
    val peerIds = MutableStateFlow<Set<String>>(emptySet())

    /** Hot signal for v2.7.1 toasts — emitted when a peer first appears
     *  (`Joined`) or vanishes from the live set (`Left`). One per change;
     *  no replay so a recomposing collector doesn't see stale events. */
    private val _peerEvents = kotlinx.coroutines.flow.MutableSharedFlow<PeerEvent>(
        replay = 0, extraBufferCapacity = 16
    )
    val peerEvents: kotlinx.coroutines.flow.SharedFlow<PeerEvent> = _peerEvents

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
    /** Wall-clock time of the last received audio frame; used to terminate
     *  the RECV_ACK loop when the peer goes silent without a clean PTT_STOP. */
    @Volatile private var lastRxFrameMs: Long = 0L
    /** Per-peer RECV_ACK state. Each transmitting peer gets its OWN ack loop so
     *  that when 2+ peers transmit at once, every sender is acknowledged. A single
     *  shared job keyed to the last-seen peer (the previous design) starved all but
     *  the most recent sender. Keyed by peer device address / "relay:<epoch>" id. */
    private class RxAckState(
        @Volatile var epoch: Long,
        @Volatile var seq: Int,
        @Volatile var lastFrameMs: Long,
    ) {
        @Volatile var job: Job? = null
    }
    private val rxAckStates = ConcurrentHashMap<String, RxAckState>()

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
        // Seed the BT transport's tx epoch so outbound-frame reports carry our
        // session identity. The audio payload is encrypted and can't be decoded
        // on the way out — BluetoothTransport maintains a local seq counter.
        btTransport.setTxEpoch(selfEpoch)
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

    /**
     * LEGACY full TX state machine (probe + READY_ACK gate + watchdog + wake).
     * NOT on the live path: the UI (MainScreen) drives PTT via
     * `SassyTalkNative.pttStart()` + [notifyPttPressed] directly, so this method
     * and [onPttReleased] are currently unreferenced. Kept because they encode
     * the audio-path probe / reaching-peer watchdog logic the indicators expect;
     * reviving them means routing MainScreen's press through here (and deleting
     * the duplicate notify* path). Until then, treat this as dead code — don't
     * assume the `transmitting` double-press guard or watchdog run on a real press.
     */
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

        // Wake beacon: if any tracked peer's liveness has gone STALE, broadcast
        // an OP_WAKE so they re-emit a heartbeat and re-warm their socket before
        // audio frames start arriving. Cooldown'd so a chatty operator can't
        // turn the relay into a wake spammer.
        val wakeEmitted = maybeWakeStalePeers()

        // Step 1: BLE signal to all peers
        readyAckCount.set(0)
        bleSignaling.broadcastPttStart()

        // Step 2: Brief wait for ACKs, then start audio regardless
        scope.launch {
            val preAudioDelay = if (wakeEmitted) {
                READY_ACK_TIMEOUT_MS + WAKE_PRE_AUDIO_DELAY_MS
            } else {
                READY_ACK_TIMEOUT_MS
            }
            delay(preAudioDelay)
            val acks = readyAckCount.get()
            Log.i(TAG, "Got $acks/$blePeers READY_ACKs, proceeding (delay=${preAudioDelay}ms)")

            // Step 3: Start native audio (mic -> ADPCM -> transport)
            SassyTalkNative.pttStart()
            Log.i(TAG, "Native PTT started")

            // Step 4: Start RFCOMM TX pump
            if (rfcommPeers > 0) {
                btTransport.startTxPump()
            }
        }
    }

    /**
     * Broadcast an OP_WAKE on both BLE and the cellular relay if any tracked
     * peer is STALE (no heartbeat in >8s). Returns true if a wake was sent —
     * the caller uses this to extend the pre-audio delay so woken peers have
     * a beat to fire a fresh heartbeat and re-warm their socket.
     */
    private fun maybeWakeStalePeers(): Boolean {
        val now = System.currentTimeMillis()
        val stalePeers = liveness.peerIds().filter {
            liveness.health(it, now) == PeerHealth.STALE
        }
        if (stalePeers.isEmpty()) return false
        if (now - lastWakeSentMs < WAKE_COOLDOWN_MS) {
            Log.d(TAG, "WAKE suppressed by cooldown (last=${now - lastWakeSentMs}ms ago)")
            return false
        }
        lastWakeSentMs = now
        val wake = ControlFrame.encodeWake(selfEpoch, now)
        bleSignaling.broadcastControl(wake)
        cellularClient?.sendBinary(wake)
        Log.i(TAG, "WAKE broadcast — stale peers: ${stalePeers.size}/${liveness.peerIds().size}")
        return true
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
        // The probe must measure the RFCOMM audio path, so it goes over BT (not
        // cellular). sendRaw writes directly to the RFCOMM socket without the
        // length prefix startTxPump adds; the receiver picks it up by reading
        // the V2 frame's own length field.
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
                // We are the receiver — echo back via RFCOMM (btTransport), NOT the relay
                Log.d(TAG, "Probe received from $peerId — echoing back via RFCOMM")
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
                lastRxEpoch = decoded.epoch
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
        stopRecvAckJob(deviceAddress)
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
        // Establish BLE GATT control channel (guards duplicates internally)
        bleSignaling.connectToPeer(device)
    }

    override fun onPeerLost(deviceAddress: String) {
        Log.i(TAG, "Peer lost: $deviceAddress")
        liveness.removePeer(deviceAddress)
        peerCaps.remove(deviceAddress)
        // Tear down this peer's RECV_ACK loop so it doesn't pump frames to a dead peer.
        stopRecvAckJob(deviceAddress)
        rxAckStates.remove(deviceAddress)
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
                    caps   = SassyTalkNative.localCapabilities(),
                )
                // Track sent heartbeat for RTT measurement per peer
                for (peerId in bleSignaling.blePeerAddresses) {
                    liveness.onHeartbeatSent(peerId, selfEpoch, seq, nowMs)
                }
                bleSignaling.broadcastControl(frame)
                cellularClient?.sendBinary(frame)
                Log.d(TAG, "HB seq=$seq broadcast to ${bleSignaling.blePeerCount} peers (relay=${cellularClient != null})")

                // Poll stale status every 1s (heartbeat fires every 2s, check every tick)
                val peerIds = liveness.peerIds()
                val stale = peerIds.isNotEmpty() && peerIds.any { liveness.health(it, nowMs) == PeerHealth.STALE }
                if (anyPeerStale.value != stale) anyPeerStale.value = stale

                delay(HEARTBEAT_INTERVAL_MS)
            }
        }
        // Also run a 1s stale-check loop independent of the 2s heartbeat.
        // Use `isActive` rather than `while(true)` so the loop terminates
        // cleanly on cancellation even if a caller higher up swallows
        // CancellationException (e.g. catch(Throwable)). Matches the
        // heartbeatJob loop above.
        staleCheckJob = scope.launch {
            while (isActive) {
                delay(1_000L)
                val nowMs = System.currentTimeMillis()
                val allPeers = liveness.peerIds()
                val stale = allPeers.isNotEmpty() && allPeers.any { liveness.health(it, nowMs) == PeerHealth.STALE }
                if (anyPeerStale.value != stale) anyPeerStale.value = stale

                // v2.7.1: publish active-peer set + join/leave events.
                // "Active" = HEALTHY or DEGRADED; STALE peers are excluded
                // so the chip shows who you can actually reach right now.
                val active = allPeers.filter { liveness.health(it, nowMs) != PeerHealth.STALE }.toSet()
                val previous = peerIds.value
                if (active != previous) {
                    val joined = active - previous
                    val left = previous - active
                    peerIds.value = active
                    joined.forEach { _peerEvents.tryEmit(PeerEvent.Joined(it)) }
                    left.forEach { _peerEvents.tryEmit(PeerEvent.Left(it)) }
                }
            }
        }
    }

    fun stopHeartbeat() {
        heartbeatJob?.cancel()
        heartbeatJob = null
        staleCheckJob?.cancel()
        staleCheckJob = null
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
            ControlFrame.OP_WAKE -> handleWake(peerId, frame.payload)
            ControlFrame.OP_HYBRID_INIT -> handleHybridInit(peerId, frame.payload)
            ControlFrame.OP_HYBRID_RESP -> handleHybridResp(peerId, frame.payload)
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

        liveness.onHeartbeat(peerId, hb.epoch, hb.seq, hb.tsMs, nowMs, hb.caps)
        liveness.updatePresence(peerId, hb.state)

        // Opportunistically upgrade a 2-party session to post-quantum (path a).
        // We now know this peer's caps + epoch, which is everything the gate needs.
        maybeAutoHybridHandshake(peerId, hb.epoch)

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
            caps   = SassyTalkNative.localCapabilities(),
        )
        bleSignaling.sendControl(peerId, echo)
    }

    /**
     * OP_WAKE received: the sender is about to transmit and our liveness with
     * them was stale. Fire an unscheduled heartbeat on every transport so they
     * see us as HEALTHY before the audio starts, and refresh our view of their
     * epoch in case they restarted while we weren't watching.
     *
     * No-op if the wake's epoch matches what we already knew — there's nothing
     * to recover from. Cheap (~25 bytes on the wire).
     */
    private fun handleWake(peerId: String, payload: ByteArray) {
        if (payload.size < 16) {
            Log.w(TAG, "WAKE from $peerId: payload too short (${payload.size})")
            return
        }
        val (senderEpoch, senderTsMs) = ControlFrame.parseWake(payload)
        val nowMs = System.currentTimeMillis()
        val epochFlipped = liveness.epochChanged(peerId, senderEpoch)
        Log.i(TAG, "WAKE from $peerId epoch=$senderEpoch dt=${nowMs - senderTsMs}ms epochFlip=$epochFlipped")

        // Immediate outbound HB on every transport — bypass the normal cadence.
        val hb = ControlFrame.encodeHeartbeat(
            epoch = selfEpoch,
            seq   = hbSeq.getAndIncrement(),
            tsMs  = nowMs,
            state = currentPresenceState(),
            rttMs = liveness.rttMs(peerId).coerceAtLeast(0),
            caps  = SassyTalkNative.localCapabilities(),
        )
        bleSignaling.broadcastControl(hb)
        cellularClient?.sendBinary(hb)

        // If the sender restarted (epoch flip), re-share our capabilities so
        // they don't drop our audio for codec-mismatch reasons.
        if (epochFlipped) {
            scope.launch { sendCapabilitiesToPeer(peerId) }
        }
    }

    // —— Hybrid PQC handshake (path a) — Phase 3 wire exchange ——
    //
    // The QR PSK already authenticates the pairing; this exchange layers an
    // ephemeral X25519 + ML-KEM-768 handshake on top so the live session key
    // gains forward secrecy + post-quantum protection. The native side does all
    // the crypto (SassyTalkNative.hybridHandshake*); here we just carry the two
    // ~1.2 KB messages between peers as OP_HYBRID_INIT / OP_HYBRID_RESP frames.
    //
    // CAUTION — pairwise vs group: the established key is shared by exactly the
    // two handshaking peers and REPLACES the channel's group-PSK session, so it
    // is only correct 2-party. Do NOT auto-start it on a channel with 3+ peers
    // (a pairwise key would lock the others out). Group PQC = MLS, a later track.
    // The reactive handlers below are always safe; only the INITIATOR action
    // ([startHybridHandshake]) needs the 2-party + both-support guard, which the
    // caller applies before invoking it. Default behavior is unchanged until a
    // caller chooses to upgrade a 2-party session.

    /**
     * Master switch for AUTO-upgrading a 2-party session to PQC. The reactive
     * responder/completer always work; this only governs whether THIS device
     * initiates unprompted. On a wip branch we default it on so it's live on real
     * hardware; a caller can flip it off. Note the pairwise-key constraint below.
     */
    @Volatile var hybridPqcAuto: Boolean = true

    /** Tracks the (peer → peer-epoch) we've already auto-handshaked, so we
     *  initiate at most once per peer session and re-handshake only on restart. */
    private val hybridHandshakeEpoch = java.util.concurrent.ConcurrentHashMap<String, Long>()

    /**
     * Auto-initiate a hybrid PQC upgrade with [peerId] (whose session epoch is
     * [peerEpoch]) when ALL of these hold:
     *   - the feature is enabled ([hybridPqcAuto]),
     *   - both sides advertise CAP_HYBRID_PQC,
     *   - exactly one other peer is active (2-party — a pairwise key would lock
     *     out a 3+ group on the shared channel),
     *   - WE are the deterministic initiator: the smaller session epoch starts,
     *     so of the two peers exactly one fires (epochs are random 64-bit, ties
     *     are negligible),
     *   - we haven't already handshaked this peer at this epoch.
     *
     * CAVEAT (production hardening): install is per-side — the responder installs
     * on RESP, the initiator on completing. If the RESP frame is lost the two can
     * diverge until the next epoch/re-pair; a robust rollout adds an ACK/retry or
     * a 3-way confirm. The crypto itself is verified (core pqc tests).
     */
    private fun maybeAutoHybridHandshake(peerId: String, peerEpoch: Long) {
        if (!hybridPqcAuto) return
        if (SassyTalkNative.localCapabilities() and ControlFrame.CAP_HYBRID_PQC == 0) return
        if (liveness.peerCaps(peerId) and ControlFrame.CAP_HYBRID_PQC == 0) return

        // 2-party only: count currently-active (non-stale) peers.
        val now = System.currentTimeMillis()
        val activePeers = liveness.peerIds().count { liveness.health(it, now) != PeerHealth.STALE }
        if (activePeers != 1) return

        // Deterministic single initiator: the smaller epoch starts.
        if (selfEpoch >= peerEpoch) return

        // Once per (peer, peerEpoch).
        if (hybridHandshakeEpoch[peerId] == peerEpoch) return
        hybridHandshakeEpoch[peerId] = peerEpoch

        Log.i(TAG, "auto-hybrid: initiating PQC upgrade with $peerId (selfEpoch=$selfEpoch < peerEpoch=$peerEpoch)")
        if (!startHybridHandshake(peerId, SassyTalkNative.getChannel())) {
            // Failed to send (no PSK / caps gone) — clear so we retry next HB.
            hybridHandshakeEpoch.remove(peerId)
        }
    }

    /** Responder side: a peer sent us OP_HYBRID_INIT. Establish the session and
     *  reply with OP_HYBRID_RESP. Idempotent-ish — a duplicate INIT just re-keys. */
    private fun handleHybridInit(peerId: String, payload: ByteArray) {
        val (channel, initMsg) = ControlFrame.parseHybridFrame(payload) ?: run {
            Log.w(TAG, "hybrid INIT from $peerId: malformed payload"); return
        }
        val initB64 = Base64.encodeToString(initMsg, Base64.NO_WRAP)
        val respB64 = SassyTalkNative.hybridHandshakeRespond(channel, initB64)
        if (respB64 == null) {
            Log.w(TAG, "hybrid INIT from $peerId ch=$channel: respond failed (no PSK / bad msg)")
            return
        }
        val respMsg = Base64.decode(respB64, Base64.NO_WRAP)
        val frame = ControlFrame.encodeHybridFrame(ControlFrame.OP_HYBRID_RESP, channel, respMsg)
        bleSignaling.sendControl(peerId, frame)
        cellularClient?.sendBinary(frame)
        Log.i(TAG, "hybrid handshake: responded to $peerId ch=$channel — PQ session installed")
    }

    /** Initiator side: the peer replied with OP_HYBRID_RESP. Complete + install. */
    private fun handleHybridResp(peerId: String, payload: ByteArray) {
        val (_, respMsg) = ControlFrame.parseHybridFrame(payload) ?: run {
            Log.w(TAG, "hybrid RESP from $peerId: malformed payload"); return
        }
        val respB64 = Base64.encodeToString(respMsg, Base64.NO_WRAP)
        if (SassyTalkNative.hybridHandshakeComplete(respB64)) {
            Log.i(TAG, "hybrid handshake: completed with $peerId — PQ session installed")
        } else {
            Log.w(TAG, "hybrid handshake: complete failed with $peerId")
        }
    }

    /**
     * Initiator entry point: start a hybrid PQC upgrade with [peerId] on
     * [channel]. Sends OP_HYBRID_INIT; the peer replies via OP_HYBRID_RESP and we
     * finish in [handleHybridResp]. Returns true if the init frame was sent.
     *
     * The CALLER must ensure this is a 2-party session and that both sides
     * advertise CAP_HYBRID_PQC (check `SassyTalkNative.localCapabilities()` and
     * `liveness.peerCaps(peerId)`), and must pick a single initiator (e.g. the
     * peer with the smaller stable id) to avoid a double handshake.
     */
    fun startHybridHandshake(peerId: String, channel: Int): Boolean {
        if (SassyTalkNative.localCapabilities() and ControlFrame.CAP_HYBRID_PQC == 0) return false
        if (liveness.peerCaps(peerId) and ControlFrame.CAP_HYBRID_PQC == 0) {
            Log.d(TAG, "hybrid start skipped: $peerId doesn't advertise hybrid support")
            return false
        }
        val initB64 = SassyTalkNative.hybridHandshakeInit(channel) ?: run {
            Log.w(TAG, "hybrid start: init failed for ch=$channel (no PSK?)"); return false
        }
        val initMsg = Base64.decode(initB64, Base64.NO_WRAP)
        val frame = ControlFrame.encodeHybridFrame(ControlFrame.OP_HYBRID_INIT, channel, initMsg)
        bleSignaling.sendControl(peerId, frame)
        cellularClient?.sendBinary(frame)
        Log.i(TAG, "hybrid handshake: INIT sent to $peerId ch=$channel")
        return true
    }

    // —— RECV_ACK — Receiver side (Task 4.2) ——

    /**
     * Called by BluetoothTransport whenever a V2 audio frame arrives.
     * Updates lastRxEpoch/lastRxSeq and (re)starts the 500ms RECV_ACK loop.
     */
    private fun onAudioFrameReceived(peerId: String, epoch: Long, seq: Int) {
        val now = System.currentTimeMillis()
        // Keep the legacy "last received" fields current for other readers.
        lastRxEpoch = epoch
        lastRxSeq = seq
        lastRxPeerId = peerId
        lastRxFrameMs = now
        // Extend peer-speaking timeout on each incoming audio frame (Task 6.2)
        onPeerAudioFrame()

        // Per-peer RECV_ACK: update this peer's state and ensure ITS OWN loop runs,
        // so concurrent senders are each acknowledged independently.
        val st = rxAckStates.getOrPut(peerId) { RxAckState(epoch, seq, now) }
        st.epoch = epoch
        st.seq = seq
        st.lastFrameMs = now
        if (st.job?.isActive != true) {
            st.job = scope.launch {
                Log.d(TAG, "RECV_ACK loop started for $peerId epoch=$epoch")
                while (isActive) {
                    // Stop if THIS peer's audio dried up and no clean PTT_STOP
                    // arrived. Without this, the loop pumps BLE control frames
                    // forever after a dropped session, contending with RFCOMM audio.
                    val silence = System.currentTimeMillis() - st.lastFrameMs
                    if (silence > RECV_ACK_INTERVAL_MS * 3) {
                        Log.d(TAG, "RECV_ACK loop stopping for $peerId: ${silence}ms since last frame")
                        break
                    }
                    val frame = ControlFrame.encodeRecvAck(st.epoch, st.seq, System.currentTimeMillis())
                    bleSignaling.sendControl(peerId, frame)
                    cellularClient?.sendBinary(frame)
                    Log.d(TAG, "RECV_ACK sent epoch=${st.epoch} seq=${st.seq} to $peerId")
                    delay(RECV_ACK_INTERVAL_MS)
                }
                Log.d(TAG, "RECV_ACK loop stopped for $peerId")
            }
        }
    }

    /** Stop the RECV_ACK loop for a SPECIFIC peer (called when that peer ends its PTT). */
    private fun stopRecvAckJob(peerId: String) {
        rxAckStates[peerId]?.let {
            it.job?.cancel()
            it.job = null
        }
    }

    /** Stop ALL RECV_ACK loops (called on shutdown). */
    private fun stopAllRecvAckJobs() {
        rxAckStates.values.forEach {
            it.job?.cancel()
            it.job = null
        }
        rxAckStates.clear()
    }

    // —— PTT_STOP_V2 / EOT_ACK — Receiver + Sender (Task 4.3) ——

    /**
     * Receiver side: peer has released PTT and sent PTT_STOP_V2.
     * Wait 300ms for jitter buffer to drain, then send EOT_ACK back.
     */
    private fun handlePttStopV2(peerId: String, payload: ByteArray) {
        stopRecvAckJob(peerId)
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
        // Advertise what we actually transmit. The Rust pipeline encodes
        // mono 48kHz PCM with Opus (see codec.rs); the previous "adpcm/8000"
        // stub was a leftover from an early prototype and would mislead
        // any cross-platform peer that read capabilities to pick a codec.
        val caps = Capabilities(
            codec      = "opus",
            sampleRate = 48000,
            mute       = false,
            vol        = 100,
            battery    = -1,
            audioV2    = false,
            epoch      = selfEpoch,
        )
        val frame = caps.toFrame()
        // Cellular peers carry IDs of the form "relay:<epoch>" or the legacy
        // constant "relay" — those don't exist as a BLE device address, so
        // sendControl would silently drop. Route via the cellular WS instead;
        // the relay's blind fan-out delivers to all attached peers (including
        // the one that just restarted), which is what we need on an epoch
        // flip recovery.
        if (peerId.startsWith("relay")) {
            cellularClient?.sendBinary(frame)
            Log.d(TAG, "Sent Capabilities to cellular peer $peerId")
        } else {
            bleSignaling.sendControl(peerId, frame)
            Log.d(TAG, "Sent Capabilities to BLE peer $peerId")
        }
    }

    /** Retrieve cached Capabilities for a peer, or null if not yet received. */
    fun peerCapabilities(peerId: String): Capabilities? = peerCaps[peerId]

    // —— Lifecycle ——

    fun shutdown() {
        stopHeartbeat()
        stopAllRecvAckJobs()
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
