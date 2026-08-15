// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-HRWCKTQX2RJC
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
 * Session-roster events (sticky channel model — not a chat lobby).
 * [Joined] fires the first time we learn a peer is in the session.
 * [Left] fires only on explicit removal (session clear / leave), never when
 * heartbeats go quiet — idle peers stay on the roster and are woken via
 * OP_WAKE / FCM → foreground service.
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

enum class DeliveryState { Idle, Sending, Acknowledged }
data class DeliveryProgress(val acknowledged: Int = 0, val expected: Int = 0)
class PttCoordinator(
    private var bleSignaling: BleSignalingService? = null,
    private var btTransport: BluetoothTransport? = null,
) : BleSignalingService.Listener {

    /** Optional cellular relay client — set after construction when relay connects. */
    var cellularClient: CellularWebSocketClient? = null

    /** Fired on PTT / inbound audio so the service can renew a short WakeLock. */
    var onRadioActivity: (() -> Unit)? = null
    /** Keeps Android route monitoring scoped to a verified TX interval. */
    var onTxStateChanged: ((Boolean) -> Unit)? = null
    /** Local persistence/audit hook; never changes core wire framing. */
    var onEmergencyAudit: ((event: String, detail: String) -> Unit)? = null

    /** Last PTT rejection reason for UI snackbars (null when OK). */
    private val _pttRejectReason = MutableStateFlow<String?>(null)
    val pttRejectReason = _pttRejectReason.asStateFlow()

    /** True while BLE peers are nearby but RFCOMM audio link is still dialing. */
    private val _linkingBluetooth = MutableStateFlow(false)
    val linkingBluetooth = _linkingBluetooth.asStateFlow()

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
        bleSignaling?.broadcastControl(frame)
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
        /** Bound RFCOMM dial retries for BLE peers without a data socket. */
        private const val RFCOMM_LINK_RETRY_MS = 3_000L
        /** Hard safety ceiling for every TX source, including toggle accessories. */
        const val DEFAULT_MAX_TX_MS = 60_000L

        // Task 7.1 — Sub-audible audio path probe marker
        const val PROBE_EPOCH    = -1L  // 0xFFFFFFFFFFFFFFFF as signed Long
        const val PROBE_SEQ      = -1   // 0xFFFFFFFF as signed Int
        const val PROBE_ECHO_SEQ = -2   // Echo response marker (PROBE_SEQ - 1)
    }

    private val transmitting = AtomicBoolean(false)
    private val readyAckCount = AtomicInteger(0)
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    /** Delayed READY_ACK → native pttStart job. Must be cancelled on release. */
    private var preAudioJob: Job? = null
    private var rfcommLinkJob: Job? = null
    private var maxTxJob: Job? = null
    private var hybridConfirmTimeoutJob: Job? = null
    private var hybridConfirmRetryJob: Job? = null
    private var txHealthJob: Job? = null
    @Volatile var maxTxMs: Long = DEFAULT_MAX_TX_MS

    // —— Delivery State (Task 4.3) — EOT_ACK "delivered" tick ——

    val deliveredState = MutableStateFlow(DeliveryState.Idle)
    /** Per-recipient receipt semantics. Never infer group delivery from one ACK. */
    val deliveryProgress = MutableStateFlow(DeliveryProgress())
    private val expectedEotPeers = ConcurrentHashMap.newKeySet<String>()
    private val acknowledgedEotPeers = ConcurrentHashMap.newKeySet<String>()

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

    /** True when we expected RECV_ACKs but none arrived within the timeout. */
    val peerReachFailed = MutableStateFlow(false)

    /** Timestamp (ms) when the probe frame was sent; 0 means no probe in-flight. */
    @Volatile private var probeSentMs = 0L

    /** Timeout job for in-flight probe — cancelled when echo arrives. */
    private var probeTimeoutJob: Job? = null

    // —— Stale-peer banner (Task 6.2) ——

    /** True when at least one connected peer has health == STALE. */
    val anyPeerStale = MutableStateFlow(false)

    // —— v2.7.1: Peer roster + join/leave events ——

    /** Sticky session roster (includes idle/STALE peers). Polled at 1 Hz. */
    val peerIds = MutableStateFlow<Set<String>>(emptySet())

    /** Toasts for first sighting / explicit leave — not HB silence. */
    private val _peerEvents = kotlinx.coroutines.flow.MutableSharedFlow<PeerEvent>(
        replay = 0, extraBufferCapacity = 16
    )
    val peerEvents: kotlinx.coroutines.flow.SharedFlow<PeerEvent> = _peerEvents

    /**
     * Relay JSON `peer_joined`. NOTE: this key is "relay:<serverClientId>",
     * which is NOT the key real heartbeats arrive under ("relay:<epoch>", see
     * CellularWebSocketClient.relayPeerIdFromFrame). Seeding a liveness identity
     * here created a phantom peer that never got heartbeats, went STALE every
     * ~8s, and — because the stale relay worker mints a new clientId on every
     * reconnect — churned Joined/Left events (raw-id snackbar spam + a duplicate
     * roster row). We now only register the friendly NAME (deduped by the Users
     * list); liveness/presence is driven solely by the genuine epoch heartbeat,
     * which arrives within ~2s.
     */
    fun onRelayPeerSeen(peerKey: String, deviceName: String) {
        try {
            if (deviceName.isNotBlank()) {
                SassyTalkNative.registerUser(peerKey, deviceName)
            }
        } catch (t: Throwable) {
            Log.w(TAG, "onRelayPeerSeen($peerKey) failed: ${t.message}")
        }
    }

    fun onRelayPeerGone(peerKey: String) {
        // Sticky channel: relay WS leave ≠ left the session. Keep roster /
        // liveness so they go idle→wakeable instead of join/leave spam.
        // (peerKey here is often "relay:<serverClientId>", which is not the
        // epoch-based heartbeat id anyway — removing it never matched HB.)
        Log.i(TAG, "Relay socket idle for $peerKey — keeping on channel")
    }

    // —— Emergency / SOS (life-safety) ——
    //
    // Beacons are built and parsed natively so every platform is byte-identical
    // and the payload is AEAD-sealed before it touches a transport. This layer
    // owns only: broadcast on BOTH transports, re-broadcast on the core-defined
    // cadence, and surface state to the UI.
    //
    // Broadcast goes out on BLE *and* relay unconditionally — unlike audio,
    // which picks an active plane. A distress beacon is ~40 bytes and its
    // delivery matters more than the duplicate cost, so it takes every path
    // available and lets receivers de-dupe by (sender, timestamp).

    /** An emergency raised by a peer, as decoded from the wire. */
    data class PeerEmergency(
        val senderId: String,
        val kind: String,
        val timestampMs: Long,
        val lat: Double?,
        val lon: Double?,
        val note: String?,
        /** False when the beacon arrived unsealed — the sender had no session
         *  key, so treat sender identity as unauthenticated. */
        val sealed: Boolean,
    )

    /** Active emergencies from peers, keyed by sender id. Cleared on stand-down. */
    val peerEmergencies = MutableStateFlow<Map<String, PeerEmergency>>(emptyMap())

    /** True while THIS device is broadcasting a distress beacon. */
    val selfEmergencyActive = MutableStateFlow(false)

    private var emergencyBeaconJob: Job? = null

    /**
     * Raise a distress beacon and start the re-broadcast loop.
     *
     * @param kind [SassyTalkNative.EMERGENCY_KIND_SOS] or ..._MANDOWN
     * @param coord optional (latE7, lonE7) fixed-point fix; attached only when
     *   a session key exists to seal it (enforced natively).
     * @return true if a beacon frame went out.
     */
    fun raiseEmergency(
        kind: Byte = SassyTalkNative.EMERGENCY_KIND_SOS,
        coord: Pair<Int, Int>? = null,
        note: String = "",
    ): Boolean {
        val frame = SassyTalkNative.emergencyActivate(
            kind = kind,
            hasCoord = coord != null,
            latE7 = coord?.first ?: 0,
            lonE7 = coord?.second ?: 0,
            note = note,
        ) ?: run {
            Log.e(TAG, "EMERGENCY: native activate returned no frame")
            return false
        }
        broadcastEmergencyFrame(frame)
        selfEmergencyActive.value = true
        try { onEmergencyAudit?.invoke("self_raised", "kind=$kind coord=${coord != null}") } catch (_: Throwable) {}
        Log.w(TAG, "EMERGENCY RAISED kind=$kind coord=${coord != null}")
        startEmergencyBeacon()
        return true
    }

    /** Stand down. Broadcasts the clear frame once and stops the beacon loop. */
    fun clearEmergency() {
        emergencyBeaconJob?.cancel()
        emergencyBeaconJob = null
        val frame = SassyTalkNative.emergencyClear()
        selfEmergencyActive.value = false
        try { onEmergencyAudit?.invoke("self_cleared", "") } catch (_: Throwable) {}
        if (frame != null) {
            broadcastEmergencyFrame(frame)
            Log.w(TAG, "EMERGENCY CLEARED — stand-down sent")
        }
    }

    /**
     * Re-broadcast loop. The cadence lives in core (`DEFAULT_BEACON_INTERVAL_MS`);
     * this polls faster than the interval and lets native decide when a frame is
     * actually due, so the two can never drift apart.
     */
    private fun startEmergencyBeacon() {
        if (emergencyBeaconJob?.isActive == true) return
        emergencyBeaconJob = scope.launch {
            while (isActive && SassyTalkNative.emergencyIsActive()) {
                delay(1_000L)
                val frame = SassyTalkNative.emergencyTick() ?: continue
                broadcastEmergencyFrame(frame)
                Log.d(TAG, "EMERGENCY beacon re-broadcast")
            }
        }
    }

    /** Put a beacon frame on every transport we have. */
    private fun broadcastEmergencyFrame(frame: ByteArray) {
        try { bleSignaling?.broadcastControl(frame) } catch (t: Throwable) {
            Log.w(TAG, "EMERGENCY: BLE broadcast failed: ${t.message}")
        }
        try { cellularClient?.sendBinary(frame) } catch (t: Throwable) {
            Log.w(TAG, "EMERGENCY: relay send failed: ${t.message}")
        }
    }

    /**
     * Inbound beacon / stand-down. Decoding is native (bounds-checked, tries
     * the sealed payload then cleartext); this only maps it into UI state.
     * A malformed or hostile frame decodes to null and is dropped.
     */
    private fun handleEmergencyFrame(peerId: String, raw: ByteArray) {
        val json = SassyTalkNative.emergencyDecode(raw) ?: run {
            Log.w(TAG, "EMERGENCY: undecodable frame from $peerId — dropped")
            return
        }
        try {
            val o = org.json.JSONObject(json)
            val sender = o.optString("sender", "").ifEmpty { peerId }
            when (o.optString("op")) {
                "clear" -> {
                    peerEmergencies.value = peerEmergencies.value - sender
                    try { onEmergencyAudit?.invoke("peer_cleared", "sender=$sender") } catch (_: Throwable) {}
                    Log.w(TAG, "EMERGENCY CLEARED by $sender")
                }
                "emergency", "mandown" -> {
                    val timestamp = o.optLong("ts", 0L)
                    val ageMs = System.currentTimeMillis() - timestamp
                    if (timestamp <= 0L || ageMs > 5 * 60_000L || ageMs < -60_000L) {
                        Log.w(TAG, "EMERGENCY from $sender rejected: stale/future timestamp age=${ageMs}ms")
                        return
                    }
                    val e = PeerEmergency(
                        senderId = sender,
                        kind = o.optString("kind", "sos"),
                        timestampMs = timestamp,
                        lat = if (o.has("lat")) o.optDouble("lat") else null,
                        lon = if (o.has("lon")) o.optDouble("lon") else null,
                        note = o.optString("note", "").ifEmpty { null },
                        sealed = o.optBoolean("sealed", false),
                    )
                    if (transmitting.get()) {
                        Log.w(TAG, "Local floor released for authenticated emergency from $sender")
                        onPttReleased()
                    }
                    peerEmergencies.value = peerEmergencies.value + (sender to e)
                    try {
                        onEmergencyAudit?.invoke(
                            "peer_raised",
                            "sender=$sender kind=${e.kind} sealed=${e.sealed}",
                        )
                    } catch (_: Throwable) {}
                    Log.w(
                        TAG,
                        "EMERGENCY from $sender kind=${e.kind} coord=${e.lat != null} sealed=${e.sealed}",
                    )
                }
            }
        } catch (t: Throwable) {
            Log.w(TAG, "EMERGENCY: bad decode JSON from $peerId: ${t.message}")
        }
    }

    // —— Talk-over indicator (Task 6.2) ——

    /** True while a peer is actively transmitting (OP_PTT_START / OP_PTT_START_V2 received). */
    val peerSpeaking = MutableStateFlow(false)

    /** Job that auto-clears the UI LED after [FloorArbitration.UI_SPEAKING_MS]. */
    private var peerSpeakingTimeoutJob: Job? = null

    /**
     * Authenticated floor owner. Independent of [peerSpeaking] so a 400 ms
     * cellular gap cannot look like "channel free" and let a second PTT key up.
     */
    @Volatile private var floorPeerId: String? = null
    private var floorHoldJob: Job? = null

    /** Local install id for equal-epoch floor tie-break. Set by WalkieService. */
    @Volatile var selfPeerId: String = ""

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
        attachBluetooth(bleSignaling, btTransport)
        startHeartbeat()
    }

    /**
     * Attach or replace the optional local Bluetooth plane. The coordinator
     * exists before Bluetooth permissions/adapter availability, so relay PTT
     * and SOS never depend on BLE initialization.
     */
    @Synchronized
    fun attachBluetooth(ble: BleSignalingService?, bt: BluetoothTransport?) {
        bleSignaling?.takeIf { it !== ble }?.let {
            it.listener = null
            it.controlFrameCallback = null
        }
        btTransport?.takeIf { it !== bt }?.let {
            it.audioFrameV2Callback = null
            it.audioFrameCallback = null
            it.txFrameCallback = null
        }
        bleSignaling = ble
        btTransport = bt
        ble?.listener = this
        ble?.controlFrameCallback = { peerId, bytes -> onControlFrame(peerId, bytes) }
        bt?.audioFrameV2Callback = { peerId, decoded -> onAudioFrameV2(peerId, decoded) }
        bt?.audioFrameCallback = { peerId, epoch, seq -> onAudioFrameReceived(peerId, epoch, seq) }
        bt?.setTxEpoch(selfEpoch)
        bt?.txFrameCallback = { epoch, seq ->
            lastTxEpoch = epoch
            lastTxSeq = seq
        }
        if (ble != null && bt != null) startRfcommLinkRetry() else {
            rfcommLinkJob?.cancel()
            rfcommLinkJob = null
            _linkingBluetooth.value = false
        }
    }

    /**
     * Keep dialing RFCOMM for BLE-discovered session peers until the data
     * plane is up. BLE alone cannot carry encrypted PTT audio.
     */
    private fun startRfcommLinkRetry() {
        if (rfcommLinkJob?.isActive == true) return
        rfcommLinkJob = scope.launch {
            while (isActive) {
                val ble = bleSignaling ?: break
                val bt = btTransport ?: break
                val blePeers = ble.blePeers
                val rfcomm = bt.connectedPeerCount
                val needLink = blePeers.any { !bt.isConnectedTo(it.address) }
                _linkingBluetooth.value = needLink && rfcomm == 0 && blePeers.isNotEmpty()
                if (needLink) {
                    for (device in blePeers) {
                        if (!bt.isConnectedTo(device.address)) {
                            Log.i(TAG, "RFCOMM retry → ${device.name ?: device.address}")
                            bt.connectDevice(device)
                        }
                    }
                }
                delay(RFCOMM_LINK_RETRY_MS)
            }
        }
    }

    /**
     * Lightweight PTT-press hook for notification shade toggle — delegates to [onPttPressed].
     */
    fun notifyPttPressed() {
        onPttPressed()
    }

    /**
     * Lightweight PTT-release hook for notification shade toggle — delegates to [onPttReleased].
     */
    fun notifyPttReleased() {
        onPttReleased()
    }

    // —— TX Side (We press PTT) ——

    /**
     * Full TX path: probe, BLE signal, native audio, RFCOMM pump, watchdog.
     * @return false if press was rejected (no transport / no peers).
     */
    fun onPttPressed(): Boolean {
        try { onRadioActivity?.invoke() } catch (_: Throwable) {}
        if (FloorArbitration.shouldBlockLocal(floorPeerId != null, selfEmergencyActive.value)) {
            _pttRejectReason.value = "Channel busy"
            Log.i(TAG, "PTT blocked by authenticated busy-channel state")
            try { onEmergencyAudit?.invoke("tx_rejected", "reason=channel_busy") } catch (_: Throwable) {}
            return false
        }
        if (transmitting.getAndSet(true)) return true

        val blePeers = bleSignaling?.blePeerCount ?: 0
        val rfcommPeers = btTransport?.connectedPeerCount ?: 0
        val ipUp = try { SassyTalkNative.isConnected() } catch (_: Throwable) { false }

        Log.i(TAG, "PTT PRESSED — BLE peers: $blePeers, RFCOMM peers: $rfcommPeers, ipUp=$ipUp")

        // Kick RFCOMM if BLE sees peers but sockets are not up yet.
        if (rfcommPeers == 0 && blePeers > 0) {
            for (device in bleSignaling?.blePeers.orEmpty()) {
                if (btTransport?.isConnectedTo(device.address) != true) {
                    btTransport?.connectDevice(device)
                }
            }
        }

        if (!BtAudioPath.canTransmit(ipUp, rfcommPeers)) {
            val reason = if (blePeers > 0) {
                BtAudioPath.REJECT_BT_LINKING
            } else {
                BtAudioPath.REJECT_NO_AUDIO_PATH
            }
            Log.w(TAG, "PTT BLOCKED: $reason (ble=$blePeers rfcomm=$rfcommPeers ip=$ipUp)")
            _pttRejectReason.value = reason
            transmitting.set(false)
            try { onEmergencyAudit?.invoke("tx_rejected", "reason=$reason") } catch (_: Throwable) {}
            return false
        }
        _pttRejectReason.value = null

        // Refuse cleartext TX before opening the mic (native also drops frames,
        // but without this gate hardware/notification PTT looked "live" with silence).
        val encrypted = try { SassyTalkNative.isEncrypted() } catch (_: Throwable) { false }
        if (!encrypted) {
            Log.w(TAG, "PTT BLOCKED: session not encrypted")
            _pttRejectReason.value = "Authenticate via QR first"
            transmitting.set(false)
            try { onEmergencyAudit?.invoke("tx_rejected", "reason=unencrypted") } catch (_: Throwable) {}
            return false
        }
        try { onTxStateChanged?.invoke(true) } catch (_: Throwable) {}
        try { onEmergencyAudit?.invoke("tx_requested", "emergency=${selfEmergencyActive.value}") } catch (_: Throwable) {}

        // Pause live captioning immediately (before the READY_ACK delay) so STT
        // releases the mic before native TX starts.
        try {
            com.sassyconsulting.sassytalkie.translate.LiveTranslationBridge.onPttStarted()
        } catch (_: Throwable) {}

        // Half-duplex: mute RX as soon as we key up (covers READY_ACK wait too).
        try { SassyTalkNative.setRxMuted(true) } catch (_: Throwable) {}

        if (rfcommPeers > 0) {
            sendAudioProbe()
        }

        _reachingPeer.value = false
        peerReachFailed.value = false
        lastAckMs = 0L
        startWatchdog()
        startMaxTxWatchdog()

        // Mark delivery as Sending (Task 4.3)
        eotTimeoutJob?.cancel()
        deliveredResetJob?.cancel()
        deliveredState.value = DeliveryState.Sending
        expectedEotPeers.clear()
        expectedEotPeers.addAll(liveness.peerIds())
        acknowledgedEotPeers.clear()
        deliveryProgress.value = DeliveryProgress(0, expectedEotPeers.size)

        // Wake beacon: if any tracked peer's liveness has gone STALE, broadcast
        // an OP_WAKE so they re-emit a heartbeat and re-warm their socket before
        // audio frames start arriving. Cooldown'd so a chatty operator can't
        // turn the relay into a wake spammer.
        val wakeEmitted = maybeWakeStalePeers()

        // Authenticated floor request. If two peers key simultaneously, every
        // upgraded receiver applies the same emergency-first, then lower-epoch
        // ordering in handlePttStartV2, so one floor owner wins deterministically.
        val startV2 = ControlFrame.encodePttStartV2(
            selfEpoch,
            lastTxSeq + 1,
            emergencyPriority = selfEmergencyActive.value,
        )
        bleSignaling?.broadcastControl(startV2)
        cellularClient?.sendBinary(startV2)

        // Step 1: BLE signal to all peers
        readyAckCount.set(0)
        bleSignaling?.broadcastPttStart()

        // Step 2: Brief wait for ACKs, then start audio regardless.
        // CRITICAL: cancel this job on release — a quick tap used to let the
        // delayed pttStart fire after pttStop (ghost TX / mic stuck open).
        preAudioJob?.cancel()
        preAudioJob = scope.launch {
            val preAudioDelay = when {
                blePeers == 0 && rfcommPeers == 0 && ipUp -> 0L
                wakeEmitted -> READY_ACK_TIMEOUT_MS + WAKE_PRE_AUDIO_DELAY_MS
                else -> READY_ACK_TIMEOUT_MS
            }
            if (preAudioDelay > 0) delay(preAudioDelay)
            if (!transmitting.get() || !isActive) {
                Log.i(TAG, "PTT released before audio start — aborting pttStart")
                return@launch
            }
            val acks = readyAckCount.get()
            Log.i(TAG, "Got $acks/$blePeers READY_ACKs, proceeding (delay=${preAudioDelay}ms)")

            // Step 3: Start native audio (mic -> ADPCM -> transport)
            SassyTalkNative.pttStart()
            if (!SassyTalkNative.isPttActive()) {
                _pttRejectReason.value = "Transmission failed: no live bearer"
                try { onEmergencyAudit?.invoke("tx_failed", "reason=native_start") } catch (_: Throwable) {}
                forceStop("native-start-failed")
                return@launch
            }
            startTxHealthWatchdog()
            if (!transmitting.get() || !isActive) {
                // Released while native start was in flight — force stop.
                Log.i(TAG, "PTT released during pttStart — forcing stop")
                SassyTalkNative.pttStop()
                btTransport?.stopTxPump()
                return@launch
            }
            Log.i(TAG, "Native PTT started")

            // Step 4: Start RFCOMM TX pump
            if (rfcommPeers > 0) {
                btTransport?.startTxPump()
            }
        }
        return true
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
        bleSignaling?.broadcastControl(wake)
        cellularClient?.sendBinary(wake)
        Log.i(TAG, "WAKE broadcast — stale peers: ${stalePeers.size}/${liveness.peerIds().size}")
        return true
    }

    fun onPttReleased() = forceStop("release")

    /**
     * Unconditionally close the mic and pumps. Used by ACTION_UP, focus loss,
     * accessory/transport loss, permission revocation, and lifecycle teardown.
     * It intentionally does not trust local UI/controller state.
     */
    fun forceStop(reason: String) {
        val wasTransmitting = transmitting.getAndSet(false)
        val nativeActive = try { SassyTalkNative.isPttActive() } catch (_: Throwable) { false }
        if (!wasTransmitting && !nativeActive && preAudioJob == null) return

        Log.i(TAG, "PTT FORCE STOP — $reason")
        try { onEmergencyAudit?.invoke("tx_stopped", "reason=$reason") } catch (_: Throwable) {}
        try { onTxStateChanged?.invoke(false) } catch (_: Throwable) {}
        if (floorPeerId == null || floorPeerId == selfPeerId) {
            floorHoldJob?.cancel()
            floorHoldJob = null
            floorPeerId = null
        }
        maxTxJob?.cancel()
        maxTxJob = null
        txHealthJob?.cancel()
        txHealthJob = null

        // Cancel pending READY_ACK → pttStart so a quick tap can't ghost-TX.
        preAudioJob?.cancel()
        preAudioJob = null

        // Stop watchdog and reset reaching-peer indicator
        stopWatchdog()
        _reachingPeer.value = false
        peerReachFailed.value = false

        // Stop native audio
        SassyTalkNative.pttStop()

        // Restore RX playback (half-duplex)
        try { SassyTalkNative.setRxMuted(false) } catch (_: Throwable) {}
        try { SassyTalkNative.reassertRxRoute() } catch (_: Throwable) {}

        // Stop RFCOMM TX
        btTransport?.stopTxPump()

        // BLE signal to peers (legacy)
        bleSignaling?.broadcastPttStop()

        // Emit PTT_STOP_V2 with epoch + lastTxSeq for EOT_ACK delivery tick (Task 4.3)
        val stopEpoch = if (lastTxEpoch != 0L) lastTxEpoch else selfEpoch
        val stopSeq = lastTxSeq
        val pttStopV2 = ControlFrame.encodePttStopV2(stopEpoch, stopSeq)
        bleSignaling?.broadcastControl(pttStopV2)
        cellularClient?.sendBinary(pttStopV2)
        Log.d(TAG, "PTT_STOP_V2 epoch=$stopEpoch seq=$stopSeq broadcast")

        // Start 2s timeout: if no EOT_ACK, reset to Idle
        eotTimeoutJob?.cancel()
        eotTimeoutJob = scope.launch {
            delay(2_000L)
            if (deliveredState.value == DeliveryState.Sending) {
                deliveredState.value = DeliveryState.Idle
                deliveryProgress.value = DeliveryProgress()
                expectedEotPeers.clear()
                acknowledgedEotPeers.clear()
                Log.d(TAG, "EOT_ACK timeout — deliveredState reset to Idle")
            }
        }

        // Belt-and-suspenders: resume captioning even if native pttStop early-returned.
        try {
            com.sassyconsulting.sassytalkie.translate.LiveTranslationBridge.onPttReleased()
        } catch (_: Throwable) {}
    }

    private fun startMaxTxWatchdog() {
        maxTxJob?.cancel()
        val limit = TxSafetyPolicy.normalizeMaxTxMs(maxTxMs)
        maxTxJob = scope.launch {
            delay(limit)
            if (transmitting.get()) {
                Log.e(TAG, "Maximum TX duration reached (${limit}ms)")
                forceStop("max-tx-watchdog")
                _pttRejectReason.value = "Transmission stopped after safety time limit"
            }
        }
    }

    private fun startTxHealthWatchdog() {
        txHealthJob?.cancel()
        txHealthJob = scope.launch {
            while (isActive && transmitting.get()) {
                delay(100L)
                if (transmitting.get() && !SassyTalkNative.isPttActive()) {
                    _pttRejectReason.value = "Transmission stopped: all send paths failed"
                    try { onEmergencyAudit?.invoke("tx_failed", "reason=all_send_paths") } catch (_: Throwable) {}
                    forceStop("all-send-paths-failed")
                    break
                }
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
        btTransport?.sendRaw(frame)
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
                btTransport?.sendRaw(echoFrame)
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
        bleSignaling?.sendReadyAck(deviceAddress)

        holdFloor(deviceAddress, FloorArbitration.STALE_HOLD_MS)
        setPeerSpeaking(true)

        // RFCOMM RX is already running (started on connect)
        // Audio will arrive and be decoded by the RX thread
    }

    override fun onPttStopReceived(deviceAddress: String) {
        Log.i(TAG, "\u2190 PTT_STOP from $deviceAddress")
        stopRecvAckJob(deviceAddress)
        releaseFloorAfterDrain(deviceAddress)
        // TODO: Play roger beep
    }

    /** Set peerSpeaking = true and arm the UI LED timeout. Not the TX floor. */
    private fun setPeerSpeaking(speaking: Boolean) {
        peerSpeaking.value = speaking
        peerSpeakingTimeoutJob?.cancel()
        if (speaking) {
            peerSpeakingTimeoutJob = scope.launch {
                delay(FloorArbitration.UI_SPEAKING_MS)
                peerSpeaking.value = false
                Log.d(TAG, "peerSpeaking UI auto-cleared after ${FloorArbitration.UI_SPEAKING_MS}ms")
            }
        } else {
            peerSpeakingTimeoutJob = null
        }
    }

    private fun holdFloor(peerId: String, holdMs: Long) {
        floorPeerId = peerId
        floorHoldJob?.cancel()
        floorHoldJob = scope.launch {
            delay(holdMs)
            if (floorPeerId == peerId) {
                floorPeerId = null
                Log.d(TAG, "floor released after ${holdMs}ms stale hold ($peerId)")
            }
        }
    }

    /** Keep the floor occupied through jitter-buffer drain, then drop the UI LED. */
    private fun releaseFloorAfterDrain(peerId: String) {
        floorHoldJob?.cancel()
        floorHoldJob = scope.launch {
            delay(FloorArbitration.DRAIN_HOLD_MS)
            if (floorPeerId == peerId) {
                floorPeerId = null
            }
            if (floorPeerId == null) {
                setPeerSpeaking(false)
            }
            Log.d(TAG, "floor drain complete for $peerId")
        }
    }

    /** Incoming audio re-asserts floor occupancy even if the UI LED already blinked off. */
    fun onPeerAudioFrame(peerId: String) {
        try { onRadioActivity?.invoke() } catch (_: Throwable) {}
        holdFloor(peerId, FloorArbitration.STALE_HOLD_MS)
        setPeerSpeaking(true)
    }

    override fun onReadyAckReceived(deviceAddress: String) {
        val count = readyAckCount.incrementAndGet()
        Log.i(TAG, "\u2190 READY_ACK from $deviceAddress (total: $count)")
    }

    override fun onPeerDiscovered(device: android.bluetooth.BluetoothDevice) {
        Log.i(TAG, "Peer discovered: ${device.name ?: device.address}")
        // Auto-connect RFCOMM data channel when BLE peer found
        if (btTransport?.isConnectedTo(device.address) != true) {
            btTransport?.connectDevice(device)
        }
        // Establish BLE GATT control channel (guards duplicates internally)
        bleSignaling?.connectToPeer(device)
    }

    override fun onPeerLost(deviceAddress: String) {
        // RFCOMM/BLE drop is a transport blip, not a channel leave. Keep the
        // liveness entry so the peer stays on the sticky roster and can be
        // woken over relay/FCM.
        Log.i(TAG, "BT peer link lost: $deviceAddress — keeping session roster entry")
        peerCaps.remove(deviceAddress)
        // Tear down this peer's RECV_ACK loop so it doesn't pump frames to a dead peer.
        stopRecvAckJob(deviceAddress)
        rxAckStates.remove(deviceAddress)
        onTransportAvailabilityChanged("bluetooth-peer-lost")
    }

    /**
     * Re-evaluate an in-flight TX after permission/network/accessory changes.
     * A remaining IP or RFCOMM path keeps TX alive; losing every data plane
     * forces an immediate stop.
     */
    fun onTransportAvailabilityChanged(reason: String) {
        if (!transmitting.get()) return
        val ipUp = try { SassyTalkNative.isConnected() } catch (_: Throwable) { false }
        val rfcomm = btTransport?.connectedPeerCount ?: 0
        if (TxSafetyPolicy.shouldForceForTransportLoss(transmitting.get(), ipUp, rfcomm)) {
            forceStop(reason)
        }
    }

    /** Restore the local indicator after process recreation and re-arm the beacon. */
    fun restoreLocalEmergencyIndicator(active: Boolean) {
        selfEmergencyActive.value = active
        if (!active) return
        scope.launch {
            repeat(40) {
                val ready = try {
                    SassyTalkNative.isInitialized() && SassyTalkNative.isEncrypted()
                } catch (_: Throwable) { false }
                if (ready) {
                    if (!SassyTalkNative.emergencyIsActive()) {
                        raiseEmergency()
                    } else {
                        startEmergencyBeacon()
                    }
                    return@launch
                }
                delay(250L)
            }
        }
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
                // Track sent heartbeat for RTT measurement per peer. Include
                // already-tracked relay peers (relay:<epoch>) — previously only
                // BLE addresses were registered, so cellular RTT always stayed
                // unknown and the diagnostics panel showed "--".
                val hbTargets = LinkedHashSet<String>().apply {
                    addAll(bleSignaling?.blePeerAddresses.orEmpty())
                    addAll(liveness.peerIds())
                }
                for (peerId in hbTargets) {
                    liveness.onHeartbeatSent(peerId, selfEpoch, seq, nowMs)
                }
                bleSignaling?.broadcastControl(frame)
                cellularClient?.sendBinary(frame)
                if (BuildConfig.DEBUG) Log.d(TAG, "HB seq=$seq broadcast to ${bleSignaling?.blePeerCount ?: 0} peers (relay=${cellularClient != null})")

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
                // Adaptive cadence: with no peers tracked there is nothing to
                // evaluate — health, roster and stale-state are all trivially
                // empty — so a 1s tick is pure battery burn on a radio sitting
                // idle on a belt all shift (3,600 wakeups/hour to compute an
                // empty set). Poll at 1s only while peers exist; otherwise
                // idle at 5s. First peer is picked up within 5s and the loop
                // immediately returns to 1s, which is well inside the 8s
                // stale threshold, so detection latency is unaffected.
                delay(if (liveness.peerIds().isEmpty()) 5_000L else 1_000L)
                val nowMs = System.currentTimeMillis()
                val allPeers = liveness.peerIds()
                val stale = allPeers.isNotEmpty() && allPeers.any { liveness.health(it, nowMs) == PeerHealth.STALE }
                if (anyPeerStale.value != stale) anyPeerStale.value = stale

                // Sticky session roster: keep STALE peers listed. Quiet HB is
                // "idle / wakeable", not "left the channel". Emitting Left +
                // removeUser on STALE caused lobby-style join/leave spam and
                // dropped peers from the Users list until they spoke again.
                val roster = allPeers
                val previous = peerIds.value
                if (roster != previous) {
                    try {
                        val joined = roster - previous
                        val left = previous - roster
                        peerIds.value = roster
                        joined.forEach { _peerEvents.tryEmit(PeerEvent.Joined(it)) }
                        // Left only if explicitly removed from LivenessTracker
                        // (session clear / rare purge) — never on idle.
                        left.forEach { _peerEvents.tryEmit(PeerEvent.Left(it)) }
                    } catch (t: Throwable) {
                        Log.e(TAG, "peer roster update failed: ${t.message}", t)
                    }
                }

                // Proactively nudge idle peers so their FGS/relay re-attaches
                // without waiting for the next PTT (OP_WAKE on wire; FCM is
                // server-side when audio/presence needs a cold start).
                if (stale) {
                    maybeWakeStalePeers()
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
        val outerOpcode = bytes[0].toInt() and 0xff
        val verified = if (outerOpcode >= 0x10) {
            if (outerOpcode != (ControlFrame.OP_AUTHENTICATED.toInt() and 0xff)) {
                Log.w(TAG, "Rejected unauthenticated control opcode=0x${outerOpcode.toString(16)} from $peerId")
                try { onEmergencyAudit?.invoke("control_rejected", "reason=unauthenticated opcode=$outerOpcode") } catch (_: Throwable) {}
                return
            }
            AuthenticatedControlPlane.open(bytes) ?: run {
                Log.w(TAG, "Rejected forged, stale, replayed, or wrong-room control from $peerId")
                try { onEmergencyAudit?.invoke("control_rejected", "reason=auth_or_replay") } catch (_: Throwable) {}
                return
            }
        } else {
            null // Legacy BLE hints remain non-privileged compatibility signals.
        }
        val authenticatedBytes = verified?.innerFrame ?: bytes
        val authenticatedPeerId = verified?.let { "auth:${it.senderId}" } ?: peerId
        val frame = ControlFrame.decode(authenticatedBytes) ?: return

        when (frame.opcode) {
            // Legacy opcodes — delegate to existing handler methods (Listener contract preserved)
            ControlFrame.OP_PTT_START -> {
                Log.d(TAG, "onControlFrame: PTT_START from $peerId")
                onPttStartReceived(authenticatedPeerId)
            }
            ControlFrame.OP_PTT_STOP -> {
                Log.d(TAG, "onControlFrame: PTT_STOP from $peerId")
                onPttStopReceived(authenticatedPeerId)
            }
            ControlFrame.OP_PTT_START_V2 -> {
                Log.d(TAG, "onControlFrame: PTT_START_V2 from $peerId")
                handlePttStartV2(authenticatedPeerId, frame.payload)
            }
            ControlFrame.OP_READY_ACK -> {
                Log.d(TAG, "onControlFrame: READY_ACK from $peerId")
                onReadyAckReceived(authenticatedPeerId)
            }
            ControlFrame.OP_PING, ControlFrame.OP_CHANNEL_SYNC -> {
                // Handled elsewhere or no-op
            }

            // New TLV opcodes
            ControlFrame.OP_HEARTBEAT -> handleHeartbeat(authenticatedPeerId, frame.payload)
            ControlFrame.OP_RECV_ACK -> handleRecvAck(authenticatedPeerId, frame.payload)
            ControlFrame.OP_CAPABILITIES -> handleCapabilities(authenticatedPeerId, frame.payload)
            ControlFrame.OP_PTT_STOP_V2 -> handlePttStopV2(authenticatedPeerId, frame.payload)
            ControlFrame.OP_EOT_ACK -> handleEotAck(authenticatedPeerId, frame.payload)
            ControlFrame.OP_WAKE -> handleWake(authenticatedPeerId, frame.payload)
            // Life-safety beacons. Handled BEFORE the crypto opcodes below as a
            // reminder of the ordering hazard that motivated the opcode audit:
            // man-down used to sit on OP_HYBRID_INIT's byte and would have been
            // swallowed by that handler. Payload parsing is native.
            ControlFrame.OP_EMERGENCY,
            ControlFrame.OP_MANDOWN,
            ControlFrame.OP_EMERGENCY_CLEAR -> handleEmergencyFrame(authenticatedPeerId, authenticatedBytes)

            ControlFrame.OP_HYBRID_INIT -> try {
                handleHybridInit(authenticatedPeerId, frame.payload)
            } catch (t: Throwable) {
                Log.e(TAG, "hybrid INIT handler failed: ${t.message}", t)
            }
            ControlFrame.OP_HYBRID_RESP -> try {
                handleHybridResp(authenticatedPeerId, frame.payload)
            } catch (t: Throwable) {
                Log.e(TAG, "hybrid RESP handler failed: ${t.message}", t)
            }
            ControlFrame.OP_HYBRID_CONFIRM -> try {
                handleHybridConfirm(authenticatedPeerId, frame.payload)
            } catch (t: Throwable) {
                Log.e(TAG, "hybrid CONFIRM handler failed: ${t.message}", t)
            }
            ControlFrame.OP_HYBRID_CONFIRM_ACK -> try {
                handleHybridConfirmAck(authenticatedPeerId, frame.payload)
            } catch (t: Throwable) {
                Log.e(TAG, "hybrid CONFIRM_ACK handler failed: ${t.message}", t)
            }
            ControlFrame.OP_PARTNER_OFFLINE -> {
                if (frame.payload.isNotEmpty()) {
                    val idLen = frame.payload[0].toInt() and 0xFF
                    if (frame.payload.size >= 1 + idLen) {
                        val offlinePeerId = String(frame.payload, 1, idLen, Charsets.UTF_8)
                        // Sticky session: WS drop ≠ left the channel. Keep the
                        // roster entry so health goes STALE and PTT/wake can
                        // still reach their foreground service via FCM.
                        android.util.Log.i(
                            "PttCoord",
                            "Partner socket idle: $offlinePeerId — keeping on channel for wake",
                        )
                        maybeWakeStalePeers()
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
        // Per-peer, every ~2s — gate behind DEBUG so shipped logcat isn't flooded.
        if (BuildConfig.DEBUG) Log.d(TAG, "HB from $peerId seq=${hb.seq} epoch=${hb.epoch} state=${hb.state} health=$health rtt=${rtt}ms")

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
        bleSignaling?.sendControl(peerId, echo)
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
        bleSignaling?.broadcastControl(hb)
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
     * initiates unprompted.
     *
     * OFF until a 4-way confirm-ack lands: the initiator stages on RESP and
     * installs only after authenticated OP_HYBRID_CONFIRM_ACK. The responder
     * stages on INIT and installs only after authenticated OP_HYBRID_CONFIRM.
     * Auto remains off because a lost confirm still needs an explicit retry.
     */
    @Volatile var hybridPqcAuto: Boolean = false

    /** Tracks the (peer → peer-epoch) we've already auto-handshaked, so we
     *  initiate at most once per peer session and re-handshake only on restart. */
    private val hybridHandshakeEpoch = java.util.concurrent.ConcurrentHashMap<String, Long>()
    @Volatile private var stagedHybridToken: ByteArray? = null
    @Volatile private var stagedHybridAtMs: Long = 0L
    @Volatile private var stagedHybridPeer: String? = null
    @Volatile private var stagedHybridChannel: Int = 1
    @Volatile private var stagedAsInitiator: Boolean = false

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
     * The responder stages the proposed key and installs only after
     * authenticated OP_HYBRID_CONFIRM. Forged/stale confirms are rejected.
     */
    private fun maybeAutoHybridHandshake(peerId: String, peerEpoch: Long) {
        if (!hybridPqcAuto) return
        // Relay-mediated peers use synthetic "relay:…" ids. Auto-upgrading their
        // session to a pairwise PQC key races with the shared QR PSK and has
        // been observed to crash or brick audio right as the peer joins.
        if (peerId.startsWith("relay")) return
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

    /** Responder: stage the proposed session and reply with OP_HYBRID_RESP. */
    private fun handleHybridInit(peerId: String, payload: ByteArray) {
        if (SassyTalkNative.localCapabilities() and ControlFrame.CAP_HYBRID_PQC == 0) {
            Log.w(TAG, "hybrid INIT rejected: staged key confirmation is not supported by this protocol version")
            return
        }
        val (channel, initMsg) = ControlFrame.parseHybridFrame(payload) ?: run {
            Log.w(TAG, "hybrid INIT from $peerId: malformed payload"); return
        }
        discardStagedHybrid("superseded-init")
        val initB64 = Base64.encodeToString(initMsg, Base64.NO_WRAP)
        val respB64 = SassyTalkNative.hybridHandshakeRespond(channel, initB64)
        if (respB64 == null) {
            Log.w(TAG, "hybrid INIT from $peerId ch=$channel: respond failed (no PSK / bad msg)")
            return
        }
        val respMsg = Base64.decode(respB64, Base64.NO_WRAP)
        stagedHybridToken = HybridRekeyPolicy.tokenFor(respMsg)
        stagedHybridAtMs = System.currentTimeMillis()
        stagedHybridPeer = peerId
        stagedHybridChannel = channel
        stagedAsInitiator = false
        startHybridConfirmWatchdog()
        val frame = ControlFrame.encodeHybridFrame(ControlFrame.OP_HYBRID_RESP, channel, respMsg)
        bleSignaling?.sendControl(peerId, frame)
        cellularClient?.sendBinary(frame)
        Log.i(TAG, "hybrid handshake: responded to $peerId ch=$channel — session staged pending confirm")
    }

    /** Initiator: stage locally, then send authenticated confirm. Do not install yet. */
    private fun handleHybridResp(peerId: String, payload: ByteArray) {
        if (SassyTalkNative.localCapabilities() and ControlFrame.CAP_HYBRID_PQC == 0) {
            Log.w(TAG, "hybrid RESP rejected: no negotiated staged handshake")
            return
        }
        val (channel, respMsg) = ControlFrame.parseHybridFrame(payload) ?: run {
            Log.w(TAG, "hybrid RESP from $peerId: malformed payload"); return
        }
        val respB64 = Base64.encodeToString(respMsg, Base64.NO_WRAP)
        if (SassyTalkNative.hybridHandshakeComplete(respB64)) {
            val token = HybridRekeyPolicy.tokenFor(respMsg)
            stagedHybridToken = token
            stagedHybridAtMs = System.currentTimeMillis()
            stagedHybridPeer = peerId
            stagedHybridChannel = channel
            stagedAsInitiator = true
            val confirm = ControlFrame.encodeHybridFrame(ControlFrame.OP_HYBRID_CONFIRM, channel, token)
            bleSignaling?.sendControl(peerId, confirm)
            cellularClient?.sendBinary(confirm)
            startHybridConfirmWatchdog()
            startHybridConfirmRetries(peerId, channel, token)
            Log.i(TAG, "hybrid handshake: RESP staged with $peerId — confirm sent, waiting for ack")
        } else {
            Log.w(TAG, "hybrid handshake: complete/stage failed with $peerId")
        }
    }

    /** Responder: arm staged RX after authenticated matching confirm, then ACK. Do not TX on the new key yet. */
    private fun handleHybridConfirm(peerId: String, payload: ByteArray) {
        val parsed = ControlFrame.parseHybridFrame(payload)
        val token = parsed?.second
        val channel = parsed?.first ?: stagedHybridChannel
        val now = System.currentTimeMillis()
        if (!HybridRekeyPolicy.confirmAcceptable(stagedHybridToken, token, now, stagedHybridAtMs)) {
            Log.w(TAG, "hybrid CONFIRM rejected: forged, stale, or no staged handshake from $peerId")
            try { onEmergencyAudit?.invoke("control_rejected", "reason=hybrid_confirm") } catch (_: Throwable) {}
            discardStagedHybrid("bad-confirm")
            return
        }
        val ackToken = token ?: stagedHybridToken
        if (ackToken != null) {
            val ack = ControlFrame.encodeHybridFrame(ControlFrame.OP_HYBRID_CONFIRM_ACK, channel, ackToken)
            bleSignaling?.sendControl(peerId, ack)
            cellularClient?.sendBinary(ack)
        }
        if (SassyTalkNative.hybridHandshakeConfirm()) {
            Log.i(TAG, "hybrid handshake: confirm from $peerId — RX armed, ACK sent, TX stays on old key")
            try { onEmergencyAudit?.invoke("rekey", "kind=hybrid_confirm_ack_sent") } catch (_: Throwable) {}
        } else {
            // Already armed from a prior CONFIRM retry — re-ACK only.
            Log.i(TAG, "hybrid handshake: confirm retry from $peerId — re-sent ACK")
        }
    }

    /** Initiator: install the staged session only after authenticated matching ack. */
    private fun handleHybridConfirmAck(peerId: String, payload: ByteArray) {
        val parsed = ControlFrame.parseHybridFrame(payload)
        val token = parsed?.second
        val now = System.currentTimeMillis()
        if (!HybridRekeyPolicy.ackAcceptable(stagedHybridToken, token, now, stagedHybridAtMs)) {
            Log.w(TAG, "hybrid CONFIRM_ACK rejected: forged, stale, or no staged handshake from $peerId")
            try { onEmergencyAudit?.invoke("control_rejected", "reason=hybrid_confirm_ack") } catch (_: Throwable) {}
            return
        }
        hybridConfirmRetryJob?.cancel()
        hybridConfirmRetryJob = null
        if (SassyTalkNative.hybridHandshakeInstallAck()) {
            Log.i(TAG, "hybrid handshake: ack from $peerId — initiator PQ session installed")
            try { onEmergencyAudit?.invoke("rekey", "kind=hybrid_confirm_ack") } catch (_: Throwable) {}
        } else {
            Log.w(TAG, "hybrid handshake: ack from $peerId had no staged initiator session")
        }
        discardStagedHybrid(null)
    }

    private fun startHybridConfirmRetries(peerId: String, channel: Int, token: ByteArray) {
        hybridConfirmRetryJob?.cancel()
        hybridConfirmRetryJob = scope.launch {
            repeat(HybridRekeyPolicy.CONFIRM_RETRY_MAX) {
                delay(HybridRekeyPolicy.CONFIRM_RETRY_MS)
                if (stagedHybridToken == null || !stagedAsInitiator) return@launch
                val confirm = ControlFrame.encodeHybridFrame(ControlFrame.OP_HYBRID_CONFIRM, channel, token)
                bleSignaling?.sendControl(peerId, confirm)
                cellularClient?.sendBinary(confirm)
                Log.i(TAG, "hybrid handshake: retrying CONFIRM to $peerId")
            }
        }
    }

    private fun startHybridConfirmWatchdog() {
        hybridConfirmTimeoutJob?.cancel()
        hybridConfirmTimeoutJob = scope.launch {
            delay(HybridRekeyPolicy.STAGED_TTL_MS)
            if (stagedHybridToken != null) {
                Log.w(TAG, "hybrid handshake: staged session expired without confirm/ack")
                discardStagedHybrid("timeout")
            }
        }
    }

    private fun discardStagedHybrid(reason: String?) {
        hybridConfirmTimeoutJob?.cancel()
        hybridConfirmTimeoutJob = null
        hybridConfirmRetryJob?.cancel()
        hybridConfirmRetryJob = null
        stagedHybridToken = null
        stagedHybridPeer = null
        stagedHybridAtMs = 0L
        stagedAsInitiator = false
        if (reason != null && (stagedAsInitiator || reason != "timeout")) {
            SassyTalkNative.hybridHandshakeDiscard()
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
        bleSignaling?.sendControl(peerId, frame)
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
        onPeerAudioFrame(peerId)

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
                    bleSignaling?.sendControl(peerId, frame)
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

    private fun handlePttStartV2(peerId: String, payload: ByteArray) {
        val (remoteEpoch, _, remoteEmergency) = ControlFrame.parsePttStartV2(payload) ?: run {
            Log.w(TAG, "PTT_START_V2 from $peerId: malformed payload")
            return
        }
        if (transmitting.get()) {
            val localEmergency = selfEmergencyActive.value
            val remoteWins = FloorArbitration.remoteWins(
                localEpoch = selfEpoch,
                localEmergency = localEmergency,
                remoteEpoch = remoteEpoch,
                remoteEmergency = remoteEmergency,
                localPeerId = selfPeerId,
                remotePeerId = peerId,
            )
            if (remoteWins) {
                Log.w(
                    TAG,
                    "Floor preempted by $peerId emergency=$remoteEmergency epoch=$remoteEpoch",
                )
                onPttReleased()
            } else {
                Log.i(TAG, "Concurrent floor request from $peerId denied by deterministic arbitration")
                return
            }
        }
        holdFloor(peerId, FloorArbitration.STALE_HOLD_MS)
        setPeerSpeaking(true)
    }

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
        Log.d(TAG, "PTT_STOP_V2 from $peerId epoch=$epoch endSeq=$endSeq — will EOT_ACK in ${FloorArbitration.DRAIN_HOLD_MS}ms")
        releaseFloorAfterDrain(peerId)
        scope.launch {
            delay(FloorArbitration.DRAIN_HOLD_MS)
            val ack = ControlFrame.encodeEotAck(epoch, endSeq)
            bleSignaling?.sendControl(peerId, ack)
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
        acknowledgedEotPeers.add(peerId)
        val expected = expectedEotPeers.size
        deliveryProgress.value = DeliveryProgress(acknowledgedEotPeers.size, expected)
        Log.d(
            TAG,
            "EOT_ACK from $peerId epoch=$ackEpoch upToSeq=$ackSeq " +
                "(${acknowledgedEotPeers.size}/${if (expected > 0) expected else "?"})",
        )
        eotTimeoutJob?.cancel()
        deliveredState.value = DeliveryState.Acknowledged
        deliveredResetJob?.cancel()
        deliveredResetJob = scope.launch {
            delay(3_000L)
            deliveredState.value = DeliveryState.Idle
            deliveryProgress.value = DeliveryProgress()
            expectedEotPeers.clear()
            acknowledgedEotPeers.clear()
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
        peerReachFailed.value = false
    }

    private fun expectsRecvAck(): Boolean =
        (bleSignaling?.blePeerCount ?: 0) > 0 ||
            (btTransport?.connectedPeerCount ?: 0) > 0 ||
            cellularClient?.isConnected() == true ||
            try { SassyTalkNative.hasLiveAudioPath() } catch (_: Throwable) { false }

    /** Start the watchdog coroutine while PTT is held. */
    private fun startWatchdog() {
        watchdogJob?.cancel()
        watchdogJob = scope.launch {
            Log.d(TAG, "Reaching-peer watchdog started")
            val pressMs = System.currentTimeMillis()
            while (isActive) {
                delay(200L)
                if (!expectsRecvAck()) continue
                val elapsed = System.currentTimeMillis() - pressMs
                if (lastAckMs > 0L && System.currentTimeMillis() - lastAckMs > REACHING_PEER_TIMEOUT_MS) {
                    _reachingPeer.value = false
                    peerReachFailed.value = true
                    Log.d(TAG, "Reaching-peer: ACK timed out → failed")
                } else if (lastAckMs == 0L && elapsed > REACHING_PEER_TIMEOUT_MS) {
                    peerReachFailed.value = true
                    Log.d(TAG, "Reaching-peer: no ACK within ${elapsed}ms → failed")
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
            bleSignaling?.sendControl(peerId, frame)
            Log.d(TAG, "Sent Capabilities to BLE peer $peerId")
        }
    }

    /** Retrieve cached Capabilities for a peer, or null if not yet received. */
    fun peerCapabilities(peerId: String): Capabilities? = peerCaps[peerId]

    // —— Lifecycle ——

    fun shutdown() {
        forceStop("coordinator-shutdown")
        stopHeartbeat()
        stopAllRecvAckJobs()
        stopWatchdog()
        // Stop the beacon loop, but do NOT auto-send a stand-down: the app
        // going away is not the user declaring they are OK. The beacon simply
        // stops; peers age it out. Clearing is an explicit user action.
        emergencyBeaconJob?.cancel()
        emergencyBeaconJob = null
        preAudioJob?.cancel()
        preAudioJob = null
        rfcommLinkJob?.cancel()
        rfcommLinkJob = null
        _linkingBluetooth.value = false
        _reachingPeer.value = false
        audioPathDegraded.value = false
        probeSentMs = 0
        probeTimeoutJob?.cancel()
        probeTimeoutJob = null
        eotTimeoutJob?.cancel()
        deliveredResetJob?.cancel()
        maxTxJob?.cancel()
        maxTxJob = null
        deliveredState.value = DeliveryState.Idle
        deliveryProgress.value = DeliveryProgress()
        expectedEotPeers.clear()
        acknowledgedEotPeers.clear()
        try { SassyTalkNative.setRxMuted(false) } catch (_: Throwable) {}
        bleSignaling?.listener = null
        bleSignaling?.controlFrameCallback = null
        btTransport?.audioFrameV2Callback = null
        btTransport?.audioFrameCallback = null
        btTransport?.txFrameCallback = null
        scope.cancel()
    }
}
