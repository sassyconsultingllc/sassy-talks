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
 * Local-first transport policy:
 *   1. WiFi multicast when OS WiFi + pref
 *   2. Bluetooth when nearby peers and WiFi unavailable
 *   3. Cloudflare relay as long-distance backup
 *
 * Failover: WiFi lost → relay → BT; relay lost with WiFi up → stay WiFi;
 * relay lost with WiFi down → BT + degraded upgrade retry.
 */
class AutoConnectManager(private val context: Context) {

    companion object {
        private const val TAG = "AutoConnect"
        private const val CELLULAR_TIMEOUT_MS = 5000L
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
    }

    private fun wifiEnabledPref(): Boolean =
        context.getSharedPreferences(PREFS_TRANSPORT, Context.MODE_PRIVATE)
            .getBoolean(KEY_ENABLE_WIFI, true)

    private fun relayEnabledPref(): Boolean =
        context.getSharedPreferences(PREFS_TRANSPORT, Context.MODE_PRIVATE)
            .getBoolean(KEY_ENABLE_RELAY, true)

    private fun bluetoothEnabledPref(): Boolean =
        context.getSharedPreferences(PREFS_TRANSPORT, Context.MODE_PRIVATE)
            .getBoolean(KEY_ENABLE_BLUETOOTH, true)

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
    private var activeTransport: String = "none"

    private val _transportAdvisory = MutableStateFlow<TransportAdvisory?>(null)
    val transportAdvisory: StateFlow<TransportAdvisory?> = _transportAdvisory

    private var scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private var degradedRetryJob: Job? = null

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

    private fun setActiveTransport(value: String) {
        activeTransport = value
        refreshTransportAdvisory()
    }

    private fun tearDownCellularClient() {
        walkieServiceRef?.pttCoordinator?.cellularClient = null
        // Clear callbacks BEFORE disconnect so intentional teardown does not
        // look like an unexpected relay loss and trigger BT failover.
        cellularClient?.pttCoordinator = null
        cellularClient?.onRelayReady = null
        cellularClient?.onRelayLost = null
        cellularClient?.disconnect()
        cellularClient = null
        _relayReady.value = false
    }

    /** Re-score transports and publish a user-facing advisory. */
    fun refreshTransportAdvisory() {
        // Local-first truth: WiFi from active label; relay ONLY when WS is up.
        val wifiOk = activeTransport == "wifi" || activeTransport == "both"
        val relayOk = cellularClient?.isConnected() == true
        val avail = TransportAvailability(
            wifiActive = wifiOk,
            relayActive = relayOk,
            bluetoothPeers = rfcommPeerCount(walkieServiceRef),
            osHasWifi = hasWifi(),
            osHasCellular = hasCellular(),
            wifiAllowed = wifiEnabledPref(),
            relayAllowed = relayEnabledPref(),
            bluetoothAllowed = bluetoothEnabledPref(),
        )
        // Prefer advertising wifi (not "both") as active when WiFi is primary
        // and relay is merely a quiet backup — keeps upgrade copy local-first.
        val reportedActive = when {
            wifiOk && relayOk -> "both"
            wifiOk -> "wifi"
            relayOk && activeTransport == "cellular" -> "cellular"
            activeTransport == "bluetooth" -> "bluetooth"
            relayOk -> "cellular"
            else -> activeTransport
        }
        _transportAdvisory.value = TransportAdvisor.evaluate(reportedActive, avail)
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
        if (!Entitlements.isUnlockedCached(context)) {
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

            // 1) Local-first: WiFi multicast
            if (wifiAllowed && hasWifi()) {
                _state.value = ConnectState.TRYING_WIFI
                _statusText.value = "Trying WiFi..."
                Log.d(TAG, "WiFi detected, trying multicast")

                walkieService?.acquireMulticastLock()
                wifiOk = SassyTalkNative.connectWifiMulticast()

                if (wifiOk) {
                    Log.d(TAG, "WiFi multicast connected")
                    setActiveTransport("wifi")
                } else {
                    walkieService?.releaseMulticastLock()
                }
            } else if (!wifiAllowed) {
                Log.d(TAG, "WiFi multicast disabled by user pref — skipping")
            }

            // 2) Local-first: Bluetooth when WiFi unavailable — RFCOMM required
            // (BLE alone is control-plane; audio needs sockets).
            var rfcommPeers = if (btAllowed) rfcommPeerCount(walkieService) else 0
            if (!wifiOk && btAllowed && rfcommPeers == 0) {
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
            if (!wifiOk && btOk) {
                setActiveTransport("bluetooth")
                Log.i(TAG, "Bluetooth selected as local primary ($rfcommPeers RFCOMM peer(s))")
            } else if (!wifiOk && btAllowed && blePeerCount(walkieService) > 0 && !btOk) {
                Log.w(TAG, "BLE peers nearby but RFCOMM not ready — not claiming Bluetooth primary")
            }

            // 3) Relay as long-distance backup when enabled (quiet while local
            // plane is primary; sole path when neither WiFi nor BT is up).
            val relayOk = if (relayAllowed) {
                if (!wifiOk && !btOk) {
                    _state.value = ConnectState.TRYING_CELLULAR
                    _statusText.value = "Trying Cloudflare Relay..."
                }
                connectCellularSilent(walkieService)
            } else {
                Log.d(TAG, "Cloudflare relay disabled by user pref — skipping")
                false
            }

            // Active label: local planes win; "both" only when WiFi primary + relay backup
            val transport = when {
                wifiOk && relayOk -> "both"
                wifiOk -> "wifi"
                btOk && !relayOk -> "bluetooth"
                btOk && relayOk -> "bluetooth" // local-first: BT primary, relay quiet backup
                relayOk -> "cellular"
                else -> "none"
            }

            if (transport != "none") {
                setActiveTransport(transport)
                _state.value = ConnectState.CONNECTED
                val label = when (transport) {
                    "both" -> "Connected via WiFi (relay backup)"
                    "wifi" -> "Connected via WiFi"
                    "bluetooth" -> if (relayOk) "Connected via Bluetooth (relay backup)"
                                   else "Connected via Bluetooth"
                    "cellular" -> "Connected via Cloudflare Relay"
                    else -> "Connected"
                }
                _statusText.value = label
                walkieService?.updateNotification(
                    when (transport) {
                        "both" -> "Radio active — WiFi + Relay backup"
                        "wifi" -> "Radio active — WiFi"
                        "bluetooth" -> "Radio active — Bluetooth"
                        "cellular" -> "Radio active — Cloudflare Relay"
                        else -> "Radio active"
                    },
                )
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

        tearDownCellularClient()

        Log.d(TAG, "Connecting relay (long-distance backup)")

        val sessionId = SassyTalkNative.getSessionId()
        if (sessionId.isNullOrBlank()) {
            Log.d(TAG, "No session ID for relay — skipping")
            return false
        }

        SassyTalkNative.cellularSetRoom(sessionId)
        val client = CellularWebSocketClient()
        client.peerId = com.sassyconsulting.sassytalkie.InstallId.get(context)
        client.onRelayReady = { onRelayReady() }
        client.onRelayLost = { reason -> onRelaySocketLost(reason) }
        val coord = walkieService?.pttCoordinator
        client.pttCoordinator = coord
        coord?.cellularClient = client
        cellularClient = client
        client.connect()

        val iterations = (CELLULAR_TIMEOUT_MS / 100).toInt()
        for (i in 0 until iterations) {
            if (client.isConnected()) {
                Log.d(TAG, "Relay connected (backup)")
                com.sassyconsulting.sassytalkie.PresenceClient
                    .uploadCurrentToken(context, sessionId)
                refreshTransportAdvisory()
                return true
            }
            kotlinx.coroutines.delay(100)
        }

        Log.d(TAG, "Relay connection timed out — continuing without relay")
        tearDownCellularClient()
        return false
    }

    /**
     * Relay WS dropped. Local-first: keep WiFi if up; otherwise BT failover.
     * Must not tear down a healthy WiFi session just because the backup died.
     */
    private fun onRelaySocketLost(reason: String) {
        Log.w(TAG, "Relay socket lost: $reason (activeTransport=$activeTransport)")
        _relayReady.value = false
        scope.launch {
            when {
                activeTransport == "both" || (activeTransport == "wifi" && hasWifi()) -> {
                    Log.i(TAG, "Relay down — staying on WiFi (local-first)")
                    setActiveTransport("wifi")
                    _state.value = ConnectState.CONNECTED
                    _statusText.value = "Connected via WiFi"
                    walkieServiceRef?.updateNotification("Radio active — WiFi")
                }
                activeTransport == "bluetooth" -> {
                    // Already on BT; relay was quiet backup — refresh advisory only.
                    refreshTransportAdvisory()
                }
                activeTransport == "cellular" || activeTransport == "none" -> {
                    Log.w(TAG, "Primary relay lost — trying Bluetooth fallback")
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
        if (wifiEnabledPref() && hasWifi()) {
            walkieServiceRef?.acquireMulticastLock()
            if (SassyTalkNative.connectWifiMulticast()) {
                val relayOk = cellularClient?.isConnected() == true ||
                    (relayEnabledPref() && connectCellularSilent(walkieServiceRef))
                setActiveTransport(if (relayOk) "both" else "wifi")
                val label = if (relayOk) "Connected via WiFi (relay backup)" else "Connected via WiFi"
                _state.value = ConnectState.CONNECTED
                _statusText.value = label
                walkieServiceRef?.updateNotification(
                    if (relayOk) "Radio active — WiFi + Relay backup" else "Radio active — WiFi",
                )
                Log.i(TAG, "Upgraded to WiFi (activeTransport=$activeTransport)")
                stopDegradedRetry()
                return
            }
            walkieServiceRef?.releaseMulticastLock()
        }
        // Prefer BT over relay when upgrading from none if RFCOMM is ready
        if (bluetoothEnabledPref() && activeTransport != "bluetooth") {
            if (reflectBluetoothFallback()) {
                // Still try relay as quiet backup
                if (relayEnabledPref()) {
                    connectCellularSilent(walkieServiceRef)
                    refreshTransportAdvisory()
                }
                return
            }
        }
        if (relayEnabledPref() && connectCellularSilent(walkieServiceRef)) {
            setActiveTransport("cellular")
            _state.value = ConnectState.CONNECTED
            _statusText.value = "Connected via Cloudflare Relay"
            walkieServiceRef?.updateNotification("Radio active — Cloudflare Relay")
            Log.i(TAG, "Upgraded to relay (no local path)")
            stopDegradedRetry()
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

    /** WiFi still down after grace — run failover without lobby-style thrash. */
    private suspend fun handleConfirmedWifiLoss() {
        when (activeTransport) {
            "both" -> {
                val relayUp = cellularClient?.isConnected() == true
                walkieServiceRef?.releaseMulticastLock()
                if (relayUp) {
                    Log.i(TAG, "WiFi lost, relay backup still active — failover to relay")
                    setActiveTransport("cellular")
                    _state.value = ConnectState.CONNECTED
                    _statusText.value = "Connected via Cloudflare Relay"
                    walkieServiceRef?.updateNotification("Radio active — Cloudflare Relay")
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
            "wifi", "cellular", "none" -> {
                Log.w(TAG, "IP path lost — relay then Bluetooth fallback chain")
                val previous = activeTransport
                // Keep prior label during quiet recovery; only show Checking if
                // we were on a local WiFi primary (not already on relay/none).
                if (previous == "wifi") {
                    _statusText.value = "Checking radio link…"
                }
                setActiveTransport("none")
                walkieServiceRef?.releaseMulticastLock()

                val relayOk = relayEnabledPref() && connectCellularSilent(walkieServiceRef)
                if (relayOk) {
                    setActiveTransport("cellular")
                    _state.value = ConnectState.CONNECTED
                    _statusText.value = "Connected via Cloudflare Relay"
                    walkieServiceRef?.updateNotification("Radio active — Cloudflare Relay")
                } else if (reflectBluetoothFallback()) {
                    startDegradedRetry()
                } else {
                    Log.e(TAG, "All transports down after signal loss")
                    _state.value = ConnectState.FAILED
                    _statusText.value = "No local link — tap to retry"
                    startDegradedRetry()
                }
            }
            "bluetooth" -> refreshTransportAdvisory()
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
                if (activeTransport != "wifi" && activeTransport != "both") {
                    scope.launch { upgradeToBestIp() }
                } else {
                    refreshTransportAdvisory()
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

    fun isUsingRelay(): Boolean =
        cellularClient?.isConnected() == true &&
            (activeTransport == "cellular" || activeTransport == "both")

    fun onRelayReady() {
        _relayReady.value = true
        refreshTransportAdvisory()
        Log.i(TAG, "Relay confirmed ready (DO welcome received)")
    }

    fun reset() {
        _state.value = ConnectState.IDLE
        _statusText.value = ""
        _relayReady.value = false
        setActiveTransport("none")
    }

    fun disconnect() {
        unregisterNetworkCallback()
        stopDegradedRetry()
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
