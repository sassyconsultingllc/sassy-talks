// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-J6TBQOEBHU5G
package com.sassyconsulting.sassytalkie.ui

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.BtAudioPath
import com.sassyconsulting.sassytalkie.CellularWebSocketClient
import com.sassyconsulting.sassytalkie.ManagedConfig
import com.sassyconsulting.sassytalkie.RelayConnectionState
import com.sassyconsulting.sassytalkie.RelayTlsPins
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.TxBitratePolicy
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.license.Entitlements

enum class ConnectState {
    IDLE,
    DETECTING,
    TRYING_WIFI,
    TRYING_CELLULAR,
    CONNECTED,
    FAILED
}

/**
 * Relay-first transport policy:
 *   1. Cloudflare relay is the preferred common meeting point
 *   2. WiFi multicast runs alongside relay for nearby peers
 *   3. Bluetooth is last-resort when no IP path is up
 *
 * Failover is sticky: a brief relay blip does not abandon the hub.
 * Mixed-protocol groups hear each other because native TX fans out to
 * every live IP plane and the BT RFCOMM pump runs in parallel.
 */
class AutoConnectManager private constructor(private val context: Context) {

    companion object {
        private const val TAG = "AutoConnect"

        @Volatile private var instance: AutoConnectManager? = null

        /**
         * The process-wide manager. The UI and WalkieService share it, so the
         * service can bring the radio back after the process is killed and the
         * UI, when it opens, sees that connection instead of starting another.
         */
        fun shared(context: Context): AutoConnectManager =
            instance ?: synchronized(this) {
                instance ?: AutoConnectManager(context.applicationContext).also { instance = it }
            }
        /** Auth + WS open is async; 5s used to tear down a still-connecting client. */
        private const val CELLULAR_TIMEOUT_MS = 15_000L
        const val PREFS_TRANSPORT = "sassy_settings"
        const val KEY_ENABLE_WIFI = "enable_wifi_multicast"
        const val KEY_ENABLE_RELAY = "enable_cloudflare_relay"
        const val KEY_ENABLE_BLUETOOTH = "enable_bluetooth"
        private const val BT_PEER_DISCOVERY_TIMEOUT_MS = 6_000L
        /** Extra wait after BLE sighting for RFCOMM (audio plane) to come up. */
        private const val BT_RFCOMM_LINK_TIMEOUT_MS = 8_000L
        private const val DEGRADED_RETRY_MS = 12_000L
        /** Ignore brief WiFi flaps before showing Reconnecting / FAILED. */
        private const val WIFI_LOSS_GRACE_MS = 1_800L
        /** After Wi-Fi validates, wait this long for it to become the default network. */
        private const val WIFI_VALIDATED_SETTLE_MS = 1_500L
        /** Stay on relay this long after the first WS drop before considering BT. */
        private const val RELAY_STICKY_MS = 20_000L
    }

    private fun wifiEnabledPref(): Boolean = ManagedConfig.wifiEnabled(context)

    private fun relayEnabledPref(): Boolean = ManagedConfig.relayEnabled(context)

    private fun bluetoothEnabledPref(): Boolean = ManagedConfig.bluetoothEnabled(context)

    /** BLE control-plane peers (not enough for audio alone). */
    private fun blePeerCount(walkieService: WalkieService?): Int =
        walkieService?.bleSignaling?.blePeerCount ?: 0

    /** RFCOMM data-plane peers — required for Bluetooth PTT audio. */
    private fun rfcommPeerCount(walkieService: WalkieService?): Int =
        walkieService?.btTransport?.connectedPeerCount ?: 0

    /** Kick RFCOMM dials for every BLE peer that is not yet socket-linked. */
    private fun kickRfcommLinks(walkieService: WalkieService?) {
        val ble = walkieService?.bleSignaling ?: return
        val bt = walkieService.btTransport ?: return
        for (device in ble.blePeers) {
            if (!bt.isConnectedTo(device.address)) {
                bt.connectDevice(device)
            }
        }
    }

    /**
     * Wait until RFCOMM is up (or timeout). BLE sightings alone never count as
     * Bluetooth-connected for transport selection.
     */
    private suspend fun waitForRfcommReady(walkieService: WalkieService?, generation: Int? = null): Int {
        kickRfcommLinks(walkieService)
        var rfcomm = rfcommPeerCount(walkieService)
        if (rfcomm > 0) return rfcomm
        val ble = blePeerCount(walkieService)
        if (ble > 0) {
            _statusText.value = BtAudioPath.linkingStatus(ble, 0) ?: "Linking Bluetooth…"
        }
        val deadline = System.currentTimeMillis() + BT_RFCOMM_LINK_TIMEOUT_MS
        while (System.currentTimeMillis() < deadline && !overtaken(generation)) {
            kickRfcommLinks(walkieService)
            rfcomm = rfcommPeerCount(walkieService)
            if (rfcomm > 0) return rfcomm
            kotlinx.coroutines.delay(400)
        }
        return rfcommPeerCount(walkieService)
    }

    private val _state = MutableStateFlow(ConnectState.IDLE)
    val state: StateFlow<ConnectState> = _state

    private val _statusText = MutableStateFlow("")
    val statusText: StateFlow<String> = _statusText

    private val _relayReady = MutableStateFlow(false)
    val relayReady: StateFlow<Boolean> = _relayReady

    private var cellularClient: CellularWebSocketClient? = null
    private var walkieServiceRef: WalkieService? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    @Volatile
    private var activeTransport: String = "none"

    private val _transportAdvisory = MutableStateFlow<TransportAdvisory?>(null)
    val transportAdvisory: StateFlow<TransportAdvisory?> = _transportAdvisory

    private var scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private var degradedRetryJob: Job? = null
    private var relayLossJob: Job? = null
    @Volatile
    private var relayDownSinceMs: Long = 0L

    private fun hasWifi(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = cm.activeNetwork ?: return false
        val caps = cm.getNetworkCapabilities(network) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
    }

    private fun hasCellular(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = cm.activeNetwork ?: return false
        val caps = cm.getNetworkCapabilities(network) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
    }

    private fun hasInternet(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = cm.activeNetwork ?: return false
        val caps = cm.getNetworkCapabilities(network) ?: return false
        return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
    }

    private fun setActiveTransport(value: String) {
        activeTransport = value
        refreshTransportAdvisory()
    }

    private fun tearDownCellularClient() {
        walkieServiceRef?.pttCoordinator?.forceStop("cellular-client-teardown")
        walkieServiceRef?.pttCoordinator?.cellularClient = null
        // Clear callbacks BEFORE disconnect so intentional teardown does not
        // look like an unexpected relay loss and trigger BT failover.
        cellularClient?.pttCoordinator = null
        cellularClient?.onRelayReady = null
        cellularClient?.onRelayLost = null
        cellularClient?.shutdown()
        cellularClient = null
        _relayReady.value = false
    }

    /** Re-score transports and publish a user-facing advisory. */
    fun refreshTransportAdvisory() {
        val wifiOk = activeTransport == "wifi" || activeTransport == "both"
        val cellState = try {
            com.sassyconsulting.sassytalkie.debug.DiagnosticsCollector
                .parseCellularStats(SassyTalkNative.cellularGetStats()).state
        } catch (_: Throwable) { "" }
        val relayOk = RelayConnectionState.isLive(
            cellularClient?.isConnected() == true,
            cellState,
            false,
        )
        val avail = TransportAvailability(
            wifiActive = wifiOk,
            relayActive = relayOk,
            bluetoothPeers = rfcommPeerCount(walkieServiceRef),
            osHasWifi = hasWifi(),
            osHasCellular = hasCellular(),
            osHasInternet = hasInternet(),
            wifiAllowed = wifiEnabledPref(),
            relayAllowed = relayEnabledPref(),
            bluetoothAllowed = bluetoothEnabledPref(),
        )
        // Every network and transport change lands here, so this is also where
        // the transmit bitrate follows the uplink (lower on mobile data).
        SassyTalkNative.setTxBitrate(
            TxBitratePolicy.forUplink(
                relayActive = relayOk,
                osHasCellular = avail.osHasCellular,
                osHasWifi = avail.osHasWifi,
            )
        )
        val reportedActive = when {
            relayOk && wifiOk -> "both"
            relayOk && activeTransport == "cellular" -> "cellular"
            wifiOk -> "wifi"
            activeTransport == "bluetooth" -> "bluetooth"
            relayOk -> "cellular"
            else -> activeTransport
        }
        _transportAdvisory.value = TransportAdvisor.evaluate(reportedActive, avail)
    }

    private fun connectedLabels(transport: String, relayOk: Boolean): Pair<String, String> =
        when (transport) {
            "both" -> "Connected via Cloudflare Relay (WiFi also on)" to
                "Radio active — Relay + WiFi"
            "cellular" -> "Connected via Cloudflare Relay" to
                "Radio active — Cloudflare Relay"
            "wifi" -> "Connected via WiFi" to "Radio active — WiFi"
            "bluetooth" -> {
                val extra = if (relayOk) " (relay also on)" else ""
                "Connected via Bluetooth$extra" to "Radio active — Bluetooth"
            }
            else -> "Connected" to "Radio active"
        }

    private fun applyConnected(transport: String, relayOk: Boolean) {
        setActiveTransport(transport)
        _state.value = ConnectState.CONNECTED
        val (status, notif) = connectedLabels(transport, relayOk)
        _statusText.value = status
        walkieServiceRef?.updateNotification(notif)
    }

    /**
     * The service is being destroyed: forget it, and unwire the relay client
     * from its coordinator. A later service re-attaches through
     * [attachWalkieService] / [ensureConnected].
     */
    fun detach(walkieService: WalkieService) {
        if (walkieServiceRef !== walkieService) return
        walkieServiceRef = null
        val coord = walkieService.pttCoordinator
        if (coord != null && cellularClient?.pttCoordinator === coord) {
            cellularClient?.pttCoordinator = null
        }
    }

    /** Wire an existing cellular client to the coordinator after late service bind. */
    fun attachWalkieService(walkieService: WalkieService) {
        walkieServiceRef = walkieService
        val client = cellularClient ?: return
        val coord = walkieService.pttCoordinator ?: return
        client.pttCoordinator = coord
        if (coord.cellularClient == null) {
            coord.cellularClient = client
        }
    }

    /**
     * Serializes connection attempts. The UI and WalkieService can both start
     * one (screen opened while the service resumes the radio); two at once
     * would build two relay clients.
     */
    private val connectMutex = kotlinx.coroutines.sync.Mutex()

    /**
     * Bumped by [reset] and [disconnect]. A connect that started before the
     * bump was overtaken (session changed, user reconnected) and must not
     * report CONNECTED: [ensureConnected] would then trust it and never
     * connect the new room. Before the connection manager was shared with the
     * service, only the screen started connects, so this could not happen.
     */
    @Volatile private var connectGeneration = 0

    /** Connect from scratch with the current settings. */
    suspend fun autoConnect(walkieService: WalkieService?): Boolean =
        connectMutex.withLock { autoConnectLocked(walkieService) }

    /**
     * Connect only if not already connected. Used where a connection may
     * already exist: the radio screen opening, and the service resuming the
     * radio on its own.
     */
    suspend fun ensureConnected(
        walkieService: WalkieService?,
        stillWanted: () -> Boolean = { true },
    ): Boolean {
        // Taken before waiting for the lock: a disconnect or reset while we
        // wait (End Session, invite, reconnect) means this request is stale.
        val requestedAt = connectGeneration
        return connectMutex.withLock {
            if (walkieService != null) attachWalkieService(walkieService)
            when {
                _state.value == ConnectState.CONNECTED -> true
                requestedAt != connectGeneration -> false
                !stillWanted() -> false
                else -> {
                    resetState()
                    autoConnectLocked(walkieService)
                }
            }
        }
    }

    private fun overtaken(generation: Int?): Boolean =
        generation != null && generation != connectGeneration

    private suspend fun autoConnectLocked(walkieService: WalkieService?): Boolean {
        walkieServiceRef = walkieService
        val generation = connectGeneration
        if (!com.sassyconsulting.sassytalkie.license.TrialGate.mayUseRadio(context)) {
            Log.i(TAG, "autoConnect: not entitled — refusing to start transports")
            _state.value = ConnectState.FAILED
            _statusText.value = "Locked — unlock to connect"
            return false
        }
        _state.value = ConnectState.DETECTING
        _statusText.value = "Detecting network..."

        val wifiAllowed = wifiEnabledPref()
        val relayAllowed = relayEnabledPref()
        val btAllowed = bluetoothEnabledPref()
        Log.d(TAG, "Transport prefs: wifi=$wifiAllowed relay=$relayAllowed bt=$btAllowed")

        val connected = withContext(Dispatchers.IO) {
            var wifiOk = false

            // 1) Relay first — common meeting point for mixed-protocol peers.
            val relayOk = if (relayAllowed) {
                _state.value = ConnectState.TRYING_CELLULAR
                _statusText.value = "Connecting Cloudflare Relay..."
                connectCellularSilent(walkieService, generation)
            } else {
                Log.d(TAG, "Cloudflare relay disabled by user pref — skipping")
                false
            }

            // 2) WiFi multicast as an extra plane (does not replace relay).
            if (wifiAllowed && hasWifi() && !overtaken(generation)) {
                _state.value = ConnectState.TRYING_WIFI
                _statusText.value = if (relayOk) "Relay up — adding WiFi..." else "Trying WiFi..."
                Log.d(TAG, "WiFi detected, trying multicast (relayOk=$relayOk)")

                walkieService?.acquireMulticastLock()
                wifiOk = SassyTalkNative.connectWifiMulticast()

                if (wifiOk) {
                    Log.d(TAG, "WiFi multicast connected")
                } else {
                    walkieService?.releaseMulticastLock()
                }
            } else if (!wifiAllowed) {
                Log.d(TAG, "WiFi multicast disabled by user pref — skipping")
            }

            // 3) Bluetooth only when no IP path is up — RFCOMM required.
            var rfcommPeers = if (btAllowed) rfcommPeerCount(walkieService) else 0
            if (!wifiOk && !relayOk && btAllowed && rfcommPeers == 0) {
                _statusText.value = "Searching for Bluetooth peers..."
                val discoverDeadline = System.currentTimeMillis() + BT_PEER_DISCOVERY_TIMEOUT_MS
                while (System.currentTimeMillis() < discoverDeadline && !overtaken(generation)) {
                    if (blePeerCount(walkieService) > 0 || rfcommPeerCount(walkieService) > 0) break
                    kotlinx.coroutines.delay(500)
                }
                if (rfcommPeerCount(walkieService) == 0 && blePeerCount(walkieService) > 0) {
                    rfcommPeers = waitForRfcommReady(walkieService, generation)
                } else {
                    rfcommPeers = rfcommPeerCount(walkieService)
                }
            }
            val btOk = btAllowed && BtAudioPath.isBluetoothAudioReady(rfcommPeers)
            if (!wifiOk && !relayOk && btOk) {
                Log.i(TAG, "Bluetooth selected as last-resort primary ($rfcommPeers RFCOMM peer(s))")
            } else if (!wifiOk && !relayOk && btAllowed && blePeerCount(walkieService) > 0 && !btOk) {
                Log.w(TAG, "BLE peers nearby but RFCOMM not ready — not claiming Bluetooth primary")
            }

            val transport = when {
                wifiOk && relayOk -> "both"
                relayOk -> "cellular"
                wifiOk -> "wifi"
                btOk -> "bluetooth"
                else -> "none"
            }

            if (generation != connectGeneration) {
                // Overtaken by disconnect()/reset() while connecting. Leave the
                // state to whoever bumped the generation.
                Log.i(TAG, "autoConnect overtaken by a disconnect or reset — not reporting $transport")
                // Give back the multicast lock this attempt took; whoever
                // overtook it connects again (or the session has ended).
                if (wifiOk) walkieService?.releaseMulticastLock()
                return@withContext false
            }
            if (transport != "none") {
                applyConnected(transport, relayOk)
                if (transport == "bluetooth") {
                    startDegradedRetry()
                }
            } else {
                _state.value = ConnectState.FAILED
                _statusText.value = if (btAllowed)
                    "Connection failed — no peers in range" else "Connection failed"
                refreshTransportAdvisory()
            }

            transport != "none"
        }

        if (connected) {
            registerNetworkCallback()
        }

        return connected
    }

    private suspend fun connectCellularSilent(walkieService: WalkieService?, generation: Int? = null): Boolean {
        if (cellularClient?.isConnected() == true) {
            refreshTransportAdvisory()
            return true
        }

        val existing = cellularClient
        if (existing != null && existing.isAttempting()) {
            Log.d(TAG, "Relay still attempting — waiting instead of tearing down")
            return waitForRelay(existing, generation)
        }

        tearDownCellularClient()

        Log.d(TAG, "Connecting relay (preferred hub)")

        val sessionId = SassyTalkNative.getSessionId()
        if (sessionId.isNullOrBlank()) {
            Log.d(TAG, "No session ID for relay — skipping")
            return false
        }

        SassyTalkNative.cellularSetRoom(sessionId)
        RelayTlsPins.processEnabled = ManagedConfig.tlsPinningEnabled(context)
        val client = CellularWebSocketClient()
        client.peerId = com.sassyconsulting.sassytalkie.InstallId.get(context)
        client.onRelayReady = { onRelayReady() }
        client.onRelayLost = { reason -> onRelaySocketLost(reason) }
        val coord = walkieService?.pttCoordinator
        client.pttCoordinator = coord
        coord?.cellularClient = client
        cellularClient = client
        client.connect()

        return waitForRelay(client, generation)
    }

    private suspend fun waitForRelay(client: CellularWebSocketClient, generation: Int? = null): Boolean {
        val iterations = (CELLULAR_TIMEOUT_MS / 100).toInt()
        for (i in 0 until iterations) {
            // Overtaken by a disconnect/reset: stop holding the connect lock.
            if (overtaken(generation)) return false
            if (client.isConnected()) {
                Log.d(TAG, "Relay connected")
                val sid = SassyTalkNative.getSessionId()
                if (!sid.isNullOrBlank()) {
                    com.sassyconsulting.sassytalkie.PresenceClient
                        .uploadCurrentToken(context, sid)
                }
                relayDownSinceMs = 0L
                refreshTransportAdvisory()
                return true
            }
            kotlinx.coroutines.delay(100)
        }

        Log.d(TAG, "Relay not ready yet — leaving client up for background connect")
        refreshTransportAdvisory()
        return false
    }

    /**
     * Relay WS dropped. Stay on relay through brief blips; only fall back after
     * [RELAY_STICKY_MS] of continuous downtime. WiFi (if up) stays as a parallel
     * plane — we do not tear a working LAN path, and we do not promote BT while
     * the hub is still reconnecting.
     */
    private fun onRelaySocketLost(reason: String) {
        Log.w(TAG, "Relay socket lost: $reason (activeTransport=$activeTransport)")
        _relayReady.value = false
        if (relayDownSinceMs == 0L) {
            relayDownSinceMs = System.currentTimeMillis()
        }
        relayLossJob?.cancel()
        relayLossJob = scope.launch {
            when {
                activeTransport == "both" || activeTransport == "wifi" -> {
                    Log.i(TAG, "Relay down — staying on WiFi, relay will reconnect")
                    if (activeTransport == "both") {
                        setActiveTransport("wifi")
                        _statusText.value = "Connected via WiFi (relay reconnecting)"
                        walkieServiceRef?.updateNotification("Radio active — WiFi")
                    }
                    refreshTransportAdvisory()
                }
                activeTransport == "bluetooth" -> refreshTransportAdvisory()
                activeTransport == "cellular" || activeTransport == "none" -> {
                    val already = System.currentTimeMillis() - relayDownSinceMs
                    val remaining = (RELAY_STICKY_MS - already).coerceAtLeast(0L)
                    _statusText.value = "Reconnecting via Relay…"
                    if (remaining > 0L) {
                        Log.i(TAG, "Relay sticky wait ${remaining}ms (down since ${relayDownSinceMs})")
                        kotlinx.coroutines.delay(remaining)
                    }
                    if (cellularClient?.isConnected() == true) {
                        Log.i(TAG, "Relay recovered during sticky window — keeping relay")
                        relayDownSinceMs = 0L
                        applyConnected(
                            if (wifiEnabledPref() && hasWifi() &&
                                (activeTransport == "wifi" || activeTransport == "both")
                            ) "both" else "cellular",
                            true,
                        )
                        return@launch
                    }
                    Log.w(TAG, "Relay down ${RELAY_STICKY_MS}ms — trying Bluetooth fallback")
                    setActiveTransport("none")
                    if (!bluetoothFallbackOrRelay()) {
                        _state.value = ConnectState.FAILED
                        _statusText.value = "No local link — tap to retry"
                        startDegradedRetry()
                    }
                }
                else -> refreshTransportAdvisory()
            }
        }
    }

    /**
     * Bluetooth fallback that yields to the relay. The search can take 14 s,
     * and the relay often reconnects meanwhile: onRelayReady then marks the
     * session as on the relay, and a fallback that finished afterwards used to
     * switch it back to Bluetooth (or to "No local link"). The relay is the
     * preferred hub, so it wins. Returns true when either link is up.
     */
    private suspend fun bluetoothFallbackOrRelay(): Boolean {
        val bluetoothUp = reflectBluetoothFallback()
        if (cellularClient?.isConnected() == true) {
            applyConnected("cellular", true)
            stopDegradedRetry()
            return true
        }
        if (bluetoothUp) startDegradedRetry()
        return bluetoothUp
    }

    private suspend fun reflectBluetoothFallback(): Boolean {
        if (!bluetoothEnabledPref()) return false
        var rfcomm = rfcommPeerCount(walkieServiceRef)
        if (rfcomm == 0) {
            _statusText.value = "Searching for Bluetooth peers..."
            val discoverDeadline = System.currentTimeMillis() + BT_PEER_DISCOVERY_TIMEOUT_MS
            while (System.currentTimeMillis() < discoverDeadline) {
                if (blePeerCount(walkieServiceRef) > 0 || rfcommPeerCount(walkieServiceRef) > 0) break
                kotlinx.coroutines.delay(500)
            }
            rfcomm = waitForRfcommReady(walkieServiceRef)
        }
        if (BtAudioPath.isBluetoothAudioReady(rfcomm)) {
            setActiveTransport("bluetooth")
            _state.value = ConnectState.CONNECTED
            _statusText.value = BtAudioPath.connectedBluetoothStatus(rfcomm)
            walkieServiceRef?.updateNotification("Radio active — Bluetooth")
            Log.i(TAG, "Bluetooth fallback active ($rfcomm RFCOMM peer(s))")
            return true
        }
        if (blePeerCount(walkieServiceRef) > 0) {
            _statusText.value = BtAudioPath.linkingStatus(
                blePeerCount(walkieServiceRef),
                0,
            ) ?: "Linking Bluetooth…"
            Log.w(TAG, "Bluetooth fallback deferred — BLE seen, RFCOMM not ready")
        }
        return false
    }

    private suspend fun upgradeToBestIp() {
        val relayOk = if (relayEnabledPref()) {
            connectCellularSilent(walkieServiceRef)
        } else {
            false
        }

        var wifiOk = false
        if (wifiEnabledPref() && hasWifi()) {
            walkieServiceRef?.acquireMulticastLock()
            wifiOk = SassyTalkNative.connectWifiMulticast()
            if (!wifiOk) {
                walkieServiceRef?.releaseMulticastLock()
            }
        }

        when {
            relayOk && wifiOk -> {
                applyConnected("both", true)
                Log.i(TAG, "Paths: relay + WiFi (activeTransport=$activeTransport)")
                stopDegradedRetry()
            }
            relayOk -> {
                applyConnected("cellular", true)
                Log.i(TAG, "Preferred path: relay (activeTransport=$activeTransport)")
                stopDegradedRetry()
            }
            wifiOk -> {
                applyConnected("wifi", false)
                Log.i(TAG, "WiFi only — relay not up yet (activeTransport=$activeTransport)")
                stopDegradedRetry()
            }
            else -> {
                if (bluetoothEnabledPref() && activeTransport != "bluetooth") {
                    reflectBluetoothFallback()
                }
            }
        }
    }

    private fun startDegradedRetry() {
        if (degradedRetryJob?.isActive == true) return
        degradedRetryJob = scope.launch {
            while (activeTransport == "bluetooth" || activeTransport == "none") {
                kotlinx.coroutines.delay(DEGRADED_RETRY_MS)
                if (activeTransport == "bluetooth" || activeTransport == "none") {
                    upgradeToBestIp()
                }
            }
        }
    }

    private fun stopDegradedRetry() {
        degradedRetryJob?.cancel()
        degradedRetryJob = null
    }

    /** WiFi still down after grace — keep relay if up; don't flap a working hub. */
    private suspend fun handleConfirmedWifiLoss(lostAtMs: Long) {
        when (activeTransport) {
            "both" -> {
                walkieServiceRef?.releaseMulticastLock()
                if (cellularClient?.isConnected() == true) {
                    Log.i(TAG, "WiFi lost — staying on relay")
                    applyConnected("cellular", true)
                    // A socket opened before the loss rode the Wi-Fi that just
                    // went away; re-dial it on the new network instead of
                    // waiting for it to time out. One that onNetworkAvailable
                    // already reopened on mobile data is kept.
                    cellularClient?.redialAfterNetworkChange(lostAtMs, "wifi-lost")
                } else {
                    // The relay socket usually dies with the Wi-Fi it rode on
                    // and comes back on mobile data within a second or two.
                    // Going straight to the Bluetooth chain here showed
                    // "No local link" while the relay was already reconnecting.
                    recoverAfterWifiLoss("WiFi lost and relay down", lostAtMs)
                }
            }
            "wifi" -> recoverAfterWifiLoss("WiFi path lost", lostAtMs)
            "cellular" -> refreshTransportAdvisory()
            "bluetooth", "none" -> refreshTransportAdvisory()
            else -> refreshTransportAdvisory()
        }
    }

    /** Wi-Fi is gone and the relay is not up: relay first, then Bluetooth. */
    private suspend fun recoverAfterWifiLoss(reason: String, lostAtMs: Long) {
        Log.w(TAG, "$reason — preferring relay then Bluetooth")
        _statusText.value = "Checking radio link…"
        setActiveTransport("none")
        walkieServiceRef?.releaseMulticastLock()
        // A retry scheduled while Wi-Fi was failing can be most of a minute
        // away; the network just changed, so dial on the new one now.
        cellularClient?.redialAfterNetworkChange(lostAtMs, "wifi-lost")

        val relayOk = relayEnabledPref() && connectCellularSilent(walkieServiceRef)
        when {
            relayOk -> applyConnected("cellular", true)
            bluetoothFallbackOrRelay() -> Unit
            else -> {
                Log.e(TAG, "All transports down after signal loss")
                _state.value = ConnectState.FAILED
                _statusText.value = "No local link — tap to retry"
                startDegradedRetry()
            }
        }
    }

    private fun registerNetworkCallback() {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        unregisterNetworkCallback()

        val callback = object : ConnectivityManager.NetworkCallback() {
            private val wifiNetworks = java.util.Collections.synchronizedSet(mutableSetOf<Network>())
            /** Wi-Fi networks already seen validated, so each one triggers one LAN upgrade. */
            private val validatedWifi = java.util.Collections.synchronizedSet(mutableSetOf<Network>())

            private fun noteNetwork(network: Network) {
                val caps = try { cm.getNetworkCapabilities(network) } catch (_: Exception) { null }
                if (caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true) {
                    wifiNetworks.add(network)
                }
            }

            override fun onLost(network: Network) {
                val wasWifi = wifiNetworks.remove(network)
                validatedWifi.remove(network)
                // "Is Wi-Fi still up" comes from the Wi-Fi networks this
                // callback is tracking, not from hasWifi(). When onLost fires
                // the system can still name the network that just went away as
                // the active one, which read as "Wi-Fi still up": the failover
                // below was skipped, the session stayed labelled WiFi + Relay
                // on mobile data, and the relay was never re-dialed.
                val otherWifiUp = wifiNetworks.isNotEmpty()
                if (!wasWifi || otherWifiUp) {
                    Log.i(TAG, "Network lost (wasWifi=$wasWifi, otherWifiUp=$otherWifiUp) — no failover")
                    refreshTransportAdvisory()
                    return
                }
                Log.w(TAG, "WiFi network lost — activeTransport=$activeTransport (grace ${WIFI_LOSS_GRACE_MS}ms)")
                val lostAtMs = System.currentTimeMillis()

                // Debounce: Android often fires onLost during AP roam / brief
                // radio blips. Wait before tearing MulticastLock or flashing
                // "Reconnecting…" so the UI stays calm on sticky sessions.
                scope.launch {
                    kotlinx.coroutines.delay(WIFI_LOSS_GRACE_MS)
                    if (hasWifi()) {
                        Log.i(TAG, "WiFi returned within grace — skipping failover")
                        refreshTransportAdvisory()
                        return@launch
                    }
                    handleConfirmedWifiLoss(lostAtMs)
                }
            }

            override fun onAvailable(network: Network) {
                noteNetwork(network)
                Log.d(TAG, "Network available — activeTransport=$activeTransport")
                cellularClient?.onNetworkAvailable()
                if (activeTransport == "wifi" || activeTransport == "both") {
                    refreshTransportAdvisory()
                } else {
                    scope.launch { upgradeToBestIp() }
                }
            }

            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                // A VPN reports its underlying transport, which changes when it
                // moves from Wi-Fi to mobile data; drop it from the set then.
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                    wifiNetworks.add(network)
                    // Wi-Fi that came back after a loss: onAvailable fired
                    // before it was validated and became the default network,
                    // so the LAN path was not restored then. Do it once now.
                    if (caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) &&
                        validatedWifi.add(network) &&
                        activeTransport != "wifi" && activeTransport != "both" &&
                        wifiEnabledPref()
                    ) {
                        scope.launch {
                            // Let the system make it the default network first;
                            // upgradeToBestIp checks the active network.
                            kotlinx.coroutines.delay(WIFI_VALIDATED_SETTLE_MS)
                            if (activeTransport != "wifi" && activeTransport != "both") {
                                Log.i(TAG, "WiFi validated — restoring the LAN path")
                                upgradeToBestIp()
                            }
                        }
                    }
                } else {
                    wifiNetworks.remove(network)
                    validatedWifi.remove(network)
                }
                refreshTransportAdvisory()
            }
        }

        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()

        try {
            cm.registerNetworkCallback(request, callback)
            networkCallback = callback
            Log.d(TAG, "NetworkCallback registered for failover monitoring")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to register NetworkCallback: ${e.message}")
        }
    }

    private fun unregisterNetworkCallback() {
        networkCallback?.let {
            try {
                val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
                cm.unregisterNetworkCallback(it)
            } catch (_: Exception) { }
            networkCallback = null
        }
    }

    fun isUsingRelay(): Boolean = cellularClient?.isConnected() == true

    fun onRelayReady() {
        _relayReady.value = true
        // Register for wake pushes on every relay open, not only the first
        // one that happened to finish inside the startup wait. A device with
        // no presence row cannot be woken when its socket drops.
        try {
            val sid = SassyTalkNative.getSessionId()
            if (!sid.isNullOrBlank()) {
                com.sassyconsulting.sassytalkie.PresenceClient.uploadCurrentToken(context, sid)
            }
        } catch (t: Throwable) {
            Log.w(TAG, "Presence upload on relay ready failed: ${t.message}")
        }
        relayDownSinceMs = 0L
        relayLossJob?.cancel()
        relayLossJob = null
        val wifiUp = activeTransport == "wifi" || activeTransport == "both"
        when {
            wifiUp -> applyConnected("both", true)
            activeTransport == "bluetooth" -> {
                applyConnected("cellular", true)
                stopDegradedRetry()
            }
            else -> applyConnected("cellular", true)
        }
        Log.i(TAG, "Relay confirmed ready (DO welcome received, activeTransport=$activeTransport)")
    }

    fun reset() {
        connectGeneration++
        resetState()
    }

    /** Clear the shown state without overtaking an attempt in progress. */
    private fun resetState() {
        _state.value = ConnectState.IDLE
        _statusText.value = ""
        _relayReady.value = false
        relayLossJob?.cancel()
        relayLossJob = null
        relayDownSinceMs = 0L
        setActiveTransport("none")
    }

    fun disconnect() {
        connectGeneration++
        unregisterNetworkCallback()
        stopDegradedRetry()
        relayLossJob?.cancel()
        relayLossJob = null
        relayDownSinceMs = 0L
        tearDownCellularClient()
        setActiveTransport("none")
        _relayReady.value = false
        _state.value = ConnectState.IDLE
        _transportAdvisory.value = null
        scope.coroutineContext[Job]?.children?.forEach { it.cancel() }
    }

    fun shutdown() {
        disconnect()
        walkieServiceRef = null
        scope.cancel()
        scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    }
}
