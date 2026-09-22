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
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.BtAudioPath
import com.sassyconsulting.sassytalkie.CellularWebSocketClient
import com.sassyconsulting.sassytalkie.ManagedConfig
import com.sassyconsulting.sassytalkie.RelayConnectionState
import com.sassyconsulting.sassytalkie.RelayTlsPins
import com.sassyconsulting.sassytalkie.SassyTalkNative
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
class AutoConnectManager(private val context: Context) {

    companion object {
        private const val TAG = "AutoConnect"
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
    private suspend fun waitForRfcommReady(walkieService: WalkieService?): Int {
        kickRfcommLinks(walkieService)
        var rfcomm = rfcommPeerCount(walkieService)
        if (rfcomm > 0) return rfcomm
        val ble = blePeerCount(walkieService)
        if (ble > 0) {
            _statusText.value = BtAudioPath.linkingStatus(ble, 0) ?: "Linking Bluetooth…"
        }
        val deadline = System.currentTimeMillis() + BT_RFCOMM_LINK_TIMEOUT_MS
        while (System.currentTimeMillis() < deadline) {
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

    suspend fun autoConnect(walkieService: WalkieService?): Boolean {
        walkieServiceRef = walkieService
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
                connectCellularSilent(walkieService)
            } else {
                Log.d(TAG, "Cloudflare relay disabled by user pref — skipping")
                false
            }

            // 2) WiFi multicast as an extra plane (does not replace relay).
            if (wifiAllowed && hasWifi()) {
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
                while (System.currentTimeMillis() < discoverDeadline) {
                    if (blePeerCount(walkieService) > 0 || rfcommPeerCount(walkieService) > 0) break
                    kotlinx.coroutines.delay(500)
                }
                if (rfcommPeerCount(walkieService) == 0 && blePeerCount(walkieService) > 0) {
                    rfcommPeers = waitForRfcommReady(walkieService)
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

    private suspend fun connectCellularSilent(walkieService: WalkieService?): Boolean {
        if (cellularClient?.isConnected() == true) {
            refreshTransportAdvisory()
            return true
        }

        val existing = cellularClient
        if (existing != null && existing.isAttempting()) {
            Log.d(TAG, "Relay still attempting — waiting instead of tearing down")
            return waitForRelay(existing)
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

        return waitForRelay(client)
    }

    private suspend fun waitForRelay(client: CellularWebSocketClient): Boolean {
        val iterations = (CELLULAR_TIMEOUT_MS / 100).toInt()
        for (i in 0 until iterations) {
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
                    if (reflectBluetoothFallback()) {
                        startDegradedRetry()
                    } else {
                        _state.value = ConnectState.FAILED
                        _statusText.value = "No local link — tap to retry"
                        startDegradedRetry()
                    }
                }
                else -> refreshTransportAdvisory()
            }
        }
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
    private suspend fun handleConfirmedWifiLoss() {
        when (activeTransport) {
            "both" -> {
                val relayUp = cellularClient?.isConnected() == true
                walkieServiceRef?.releaseMulticastLock()
                if (relayUp) {
                    Log.i(TAG, "WiFi lost — staying on relay")
                    applyConnected("cellular", true)
                } else {
                    Log.w(TAG, "WiFi lost and relay down — Bluetooth chain")
                    setActiveTransport("none")
                    if (reflectBluetoothFallback()) {
                        startDegradedRetry()
                    } else {
                        _state.value = ConnectState.FAILED
                        _statusText.value = "No local link — tap to retry"
                        startDegradedRetry()
                    }
                }
            }
            "wifi" -> {
                Log.w(TAG, "WiFi path lost — preferring relay then Bluetooth")
                _statusText.value = "Checking radio link…"
                setActiveTransport("none")
                walkieServiceRef?.releaseMulticastLock()

                val relayOk = relayEnabledPref() && connectCellularSilent(walkieServiceRef)
                if (relayOk) {
                    applyConnected("cellular", true)
                } else if (reflectBluetoothFallback()) {
                    startDegradedRetry()
                } else {
                    Log.e(TAG, "All transports down after signal loss")
                    _state.value = ConnectState.FAILED
                    _statusText.value = "No local link — tap to retry"
                    startDegradedRetry()
                }
            }
            "cellular" -> refreshTransportAdvisory()
            "bluetooth", "none" -> refreshTransportAdvisory()
            else -> refreshTransportAdvisory()
        }
    }

    private fun registerNetworkCallback() {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        unregisterNetworkCallback()

        val callback = object : ConnectivityManager.NetworkCallback() {
            private val wifiNetworks = java.util.Collections.synchronizedSet(mutableSetOf<Network>())

            private fun noteNetwork(network: Network) {
                val caps = try { cm.getNetworkCapabilities(network) } catch (_: Exception) { null }
                if (caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true) {
                    wifiNetworks.add(network)
                }
            }

            override fun onLost(network: Network) {
                val wasWifi = wifiNetworks.remove(network)
                if (!wasWifi || hasWifi()) {
                    Log.i(TAG, "Non-WiFi network lost (wasWifi=$wasWifi, wifiStillUp=${hasWifi()}) — no failover")
                    refreshTransportAdvisory()
                    return
                }
                Log.w(TAG, "WiFi network lost — activeTransport=$activeTransport (grace ${WIFI_LOSS_GRACE_MS}ms)")

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
                    handleConfirmedWifiLoss()
                }
            }

            override fun onAvailable(network: Network) {
                noteNetwork(network)
                Log.d(TAG, "Network available — activeTransport=$activeTransport")
                if (activeTransport == "wifi" || activeTransport == "both") {
                    refreshTransportAdvisory()
                } else {
                    scope.launch { upgradeToBestIp() }
                }
            }

            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                    wifiNetworks.add(network)
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
        _state.value = ConnectState.IDLE
        _statusText.value = ""
        _relayReady.value = false
        relayLossJob?.cancel()
        relayLossJob = null
        relayDownSinceMs = 0L
        setActiveTransport("none")
    }

    fun disconnect() {
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
        scope.cancel()
        scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    }
}
