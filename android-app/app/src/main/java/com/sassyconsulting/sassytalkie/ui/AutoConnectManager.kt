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

class AutoConnectManager(private val context: Context) {

    companion object {
        private const val TAG = "AutoConnect"
        private const val CELLULAR_TIMEOUT_MS = 5000L
        const val PREFS_TRANSPORT = "sassy_settings"
        const val KEY_ENABLE_WIFI = "enable_wifi_multicast"
        const val KEY_ENABLE_RELAY = "enable_cloudflare_relay"
        const val KEY_ENABLE_BLUETOOTH = "enable_bluetooth"
        private const val BT_PEER_DISCOVERY_TIMEOUT_MS = 6_000L
        private const val DEGRADED_RETRY_MS = 12_000L
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

    private fun btPeerCount(walkieService: WalkieService?): Int {
        val w = walkieService ?: return 0
        val ble = w.bleSignaling?.blePeerCount ?: 0
        val rfcomm = w.btTransport?.connectedPeerCount ?: 0
        return ble + rfcomm
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
        // Clear the back-reference too: a discarded client holding a stale
        // pttCoordinator suppressed its own keepalive (sendKeepAlive checks
        // it) and kept routing control frames into a coordinator that no
        // longer knows about it.
        cellularClient?.pttCoordinator = null
        cellularClient?.onRelayReady = null
        cellularClient?.disconnect()
        cellularClient = null
    }

    /** Re-score transports and publish a user-facing advisory. */
    fun refreshTransportAdvisory() {
        val wifiOk = activeTransport == "wifi" || activeTransport == "both"
        val relayOk = activeTransport == "cellular" || activeTransport == "both" ||
            cellularClient?.isConnected() == true
        val avail = TransportAvailability(
            wifiActive = wifiOk,
            relayActive = relayOk,
            bluetoothPeers = btPeerCount(walkieServiceRef),
            osHasWifi = hasWifi(),
            osHasCellular = hasCellular(),
            wifiAllowed = wifiEnabledPref(),
            relayAllowed = relayEnabledPref(),
            bluetoothAllowed = bluetoothEnabledPref(),
        )
        _transportAdvisory.value = TransportAdvisor.evaluate(activeTransport, avail)
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
        // Entitlement gate BELOW the UI: a locked user must not get live
        // transports or audio, even if a share-link import or FCM wake reached
        // this path while the UI is parked on the paywall. The purchase/gate
        // screens never call autoConnect, so this blocks only the radio, not the
        // unlock flow.
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
        Log.d(TAG, "Transport prefs: wifi=$wifiAllowed relay=$relayAllowed")

        val connected = withContext(Dispatchers.IO) {
            var wifiOk = false

            if (wifiAllowed && hasWifi()) {
                _state.value = ConnectState.TRYING_WIFI
                _statusText.value = "Trying WiFi..."
                Log.d(TAG, "WiFi detected, trying multicast")

                walkieService?.acquireMulticastLock()
                wifiOk = SassyTalkNative.connectWifiMulticast()

                if (wifiOk) {
                    Log.d(TAG, "WiFi multicast connected")
                    setActiveTransport("wifi")
                }
            } else if (!wifiAllowed) {
                Log.d(TAG, "WiFi multicast disabled by user pref — skipping")
            }

            val relayOk = if (relayAllowed) {
                connectCellularSilent(walkieService)
            } else {
                Log.d(TAG, "Cloudflare relay disabled by user pref — skipping")
                false
            }

            val btAllowed = bluetoothEnabledPref()
            var btPeers = if (btAllowed) btPeerCount(walkieService) else 0

            if (!wifiOk && !relayOk && btAllowed && btPeers == 0) {
                _statusText.value = "Searching for Bluetooth peers..."
                val deadline = System.currentTimeMillis() + BT_PEER_DISCOVERY_TIMEOUT_MS
                while (System.currentTimeMillis() < deadline) {
                    btPeers = btPeerCount(walkieService)
                    if (btPeers > 0) break
                    kotlinx.coroutines.delay(500)
                }
            }
            val btOk = btPeers > 0

            val parts = buildList {
                if (wifiOk) add("WiFi")
                if (relayOk) add("Relay")
                if (btOk) add("Bluetooth")
            }
            if (parts.isNotEmpty()) {
                _state.value = ConnectState.CONNECTED
                val label = if (parts.size == 1) "Connected via ${parts.first()}"
                            else "Connected — ${parts.joinToString(" + ")}"
                _statusText.value = label
                walkieService?.updateNotification("Radio active — ${parts.joinToString(" + ")}")
                setActiveTransport(when {
                    wifiOk && relayOk -> "both"
                    wifiOk -> "wifi"
                    relayOk -> "cellular"
                    btOk -> "bluetooth"
                    else -> "none"
                })
            } else {
                _state.value = ConnectState.FAILED
                _statusText.value = if (btAllowed)
                    "Connection failed — no peers in range" else "Connection failed"
                refreshTransportAdvisory()
            }

            wifiOk || relayOk || btOk
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

        Log.d(TAG, "Connecting relay alongside WiFi")

        val sessionId = SassyTalkNative.getSessionId()
        if (sessionId.isNullOrBlank()) {
            Log.d(TAG, "No session ID for relay — skipping")
            return false
        }

        SassyTalkNative.cellularSetRoom(sessionId)
        val client = CellularWebSocketClient()
        client.peerId = com.sassyconsulting.sassytalkie.InstallId.get(context)
        client.onRelayReady = { onRelayReady() }
        val coord = walkieService?.pttCoordinator
        client.pttCoordinator = coord
        coord?.cellularClient = client
        cellularClient = client
        client.connect()

        val iterations = (CELLULAR_TIMEOUT_MS / 100).toInt()
        for (i in 0 until iterations) {
            if (client.isConnected()) {
                Log.d(TAG, "Relay connected alongside WiFi")
                com.sassyconsulting.sassytalkie.PresenceClient
                    .uploadCurrentToken(context, sessionId)
                refreshTransportAdvisory()
                return true
            }
            kotlinx.coroutines.delay(100)
        }

        Log.d(TAG, "Relay connection timed out — WiFi-only mode")
        tearDownCellularClient()
        return false
    }

    private suspend fun reflectBluetoothFallback(): Boolean {
        if (!bluetoothEnabledPref()) return false
        var peers = btPeerCount(walkieServiceRef)
        if (peers == 0) {
            _statusText.value = "Searching for Bluetooth peers..."
            val deadline = System.currentTimeMillis() + BT_PEER_DISCOVERY_TIMEOUT_MS
            while (System.currentTimeMillis() < deadline) {
                peers = btPeerCount(walkieServiceRef)
                if (peers > 0) break
                kotlinx.coroutines.delay(500)
            }
        }
        if (peers > 0) {
            setActiveTransport("bluetooth")
            _state.value = ConnectState.CONNECTED
            _statusText.value = "Connected via Bluetooth"
            walkieServiceRef?.updateNotification("Radio active — Bluetooth")
            Log.i(TAG, "Bluetooth fallback active ($peers peer(s) in range)")
            return true
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
                val label = if (relayOk) "WiFi + Relay" else "WiFi"
                _state.value = ConnectState.CONNECTED
                _statusText.value = "Connected — $label"
                walkieServiceRef?.updateNotification("Radio active — $label")
                Log.i(TAG, "Upgraded to WiFi (activeTransport=$activeTransport)")
                stopDegradedRetry()
                return
            }
            walkieServiceRef?.releaseMulticastLock()
        }
        if (relayEnabledPref() && connectCellularSilent(walkieServiceRef)) {
            setActiveTransport("cellular")
            _state.value = ConnectState.CONNECTED
            _statusText.value = "Connected via Cloudflare Relay"
            walkieServiceRef?.updateNotification("Radio active — Cloudflare Relay")
            Log.i(TAG, "Upgraded to relay (no WiFi)")
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

    private fun registerNetworkCallback() {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        unregisterNetworkCallback()

        val callback = object : ConnectivityManager.NetworkCallback() {
            // The request below matches EVERY internet-capable network, so
            // onLost fires for cellular teardowns too — most commonly
            // Android's routine linger-expiry of the idle mobile-data network
            // ~30s after WiFi becomes the default. Treating that as "WiFi
            // lost" released the MulticastLock (deafening LAN RX) and ran a
            // full spurious failover while WiFi was healthy. Track which
            // networks are actually WiFi so onLost can tell them apart.
            private val wifiNetworks = java.util.Collections.synchronizedSet(mutableSetOf<Network>())

            private fun noteNetwork(network: Network) {
                val caps = try { cm.getNetworkCapabilities(network) } catch (_: Exception) { null }
                if (caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true) {
                    wifiNetworks.add(network)
                }
            }

            override fun onLost(network: Network) {
                val wasWifi = wifiNetworks.remove(network)
                // Not a WiFi network (or WiFi is otherwise still up): nothing
                // about our WiFi path changed — just refresh the advisory.
                if (!wasWifi || hasWifi()) {
                    Log.i(TAG, "Non-WiFi network lost (wasWifi=$wasWifi, wifiStillUp=${hasWifi()}) — no failover")
                    refreshTransportAdvisory()
                    return
                }
                Log.w(TAG, "WiFi network lost — activeTransport=$activeTransport")

                if (activeTransport == "both") {
                    Log.i(TAG, "WiFi lost, relay still active — seamless failover")
                    setActiveTransport("cellular")
                    walkieServiceRef?.releaseMulticastLock()
                    _state.value = ConnectState.CONNECTED
                    _statusText.value = "Connected via Cloudflare Relay"
                    walkieServiceRef?.updateNotification("Radio active — Cloudflare Relay")
                } else if (activeTransport == "wifi" || activeTransport == "cellular" || activeTransport == "none") {
                    Log.w(TAG, "IP path lost — relay then Bluetooth fallback chain")
                    setActiveTransport("none")
                    walkieServiceRef?.releaseMulticastLock()
                    _statusText.value = "Reconnecting..."

                    scope.launch {
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
                            _statusText.value = "Disconnected — tap to retry"
                            startDegradedRetry()
                        }
                    }
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

    fun isUsingRelay(): Boolean = activeTransport == "cellular" || activeTransport == "both"

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
