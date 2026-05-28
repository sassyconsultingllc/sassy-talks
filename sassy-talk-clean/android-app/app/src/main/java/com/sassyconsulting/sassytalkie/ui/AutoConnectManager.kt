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
        // BT discovery is async — give it this long to find a peer before
        // we declare the connection failed. Short enough that the user isn't
        // staring at a spinner forever; long enough that nRF/BLE SCAN_RSP
        // round-trips can finish in normal RF conditions.
        private const val BT_PEER_DISCOVERY_TIMEOUT_MS = 6_000L
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

    /** Current count of BT peers — BLE GATT + RFCOMM combined. 0 if BT not up. */
    private fun btPeerCount(walkieService: com.sassyconsulting.sassytalkie.WalkieService?): Int {
        val w = walkieService ?: return 0
        val ble = w.bleSignaling?.blePeerCount ?: 0
        val rfcomm = w.btTransport?.connectedPeerCount ?: 0
        return ble + rfcomm
    }

    private val _state = MutableStateFlow(ConnectState.IDLE)
    val state: StateFlow<ConnectState> = _state

    private val _statusText = MutableStateFlow("")
    val statusText: StateFlow<String> = _statusText

    /** True once the Cloudflare Durable Object sends a "welcome" confirmation. */
    private val _relayReady = MutableStateFlow(false)
    val relayReady: StateFlow<Boolean> = _relayReady

    private var cellularClient: CellularWebSocketClient? = null
    private var walkieServiceRef: WalkieService? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var activeTransport: String = "none" // "wifi", "cellular", "none"

    // Scope owned by this manager so that coroutines launched from the
    // ConnectivityManager.NetworkCallback can be cancelled in disconnect().
    // Using GlobalScope would leak the coroutines past the manager's lifetime.
    // `var` not `val` so disconnect()/shutdown() can replace it after a
    // full cancel — see notes in shutdown() below.
    private var scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    private fun hasWifi(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = cm.activeNetwork ?: return false
        val caps = cm.getNetworkCapabilities(network) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
    }

    suspend fun autoConnect(walkieService: WalkieService?): Boolean {
        walkieServiceRef = walkieService
        _state.value = ConnectState.DETECTING
        _statusText.value = "Detecting network..."

        val wifiAllowed = wifiEnabledPref()
        val relayAllowed = relayEnabledPref()
        Log.d(TAG, "Transport prefs: wifi=$wifiAllowed relay=$relayAllowed")

        val connected = withContext(Dispatchers.IO) {
            var wifiOk = false

            // Try WiFi multicast for local peers (gated by user pref).
            if (wifiAllowed && hasWifi()) {
                _state.value = ConnectState.TRYING_WIFI
                _statusText.value = "Trying WiFi..."
                Log.d(TAG, "WiFi detected, trying multicast")

                walkieService?.acquireMulticastLock()
                wifiOk = SassyTalkNative.connectWifiMulticast()

                if (wifiOk) {
                    Log.d(TAG, "WiFi multicast connected")
                    activeTransport = "wifi"
                }
            } else if (!wifiAllowed) {
                Log.d(TAG, "WiFi multicast disabled by user pref — skipping")
            }

            // Cloudflare relay for remote peers (also gated by user pref).
            // When disabled, no WS is opened and audio/control only flow via
            // the LAN path — same-network peers still talk; remote peers can't.
            val relayOk = if (relayAllowed) {
                connectCellularSilent(walkieService)
            } else {
                Log.d(TAG, "Cloudflare relay disabled by user pref — skipping")
                false
            }

            // Bluetooth is a peer-of-peers transport \u2014 it runs whenever the
            // service binds (initBleTransport in AppNavigation). Reflecting it
            // here means a user with no WiFi and no cellular signal still gets
            // a "Connected" badge when an in-range peer is discovered, instead
            // of staring at a "Connection failed" screen while audio actually
            // works over BT.
            val btAllowed = bluetoothEnabledPref()
            // First snapshot \u2014 covers the case where a peer was already paired
            // / connected before autoConnect ran.
            var btPeers = if (btAllowed) btPeerCount(walkieService) else 0

            // If neither IP-based transport landed, give BT a brief window to
            // surface an in-range peer before declaring FAILED.
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

            // Resolve the status display + foreground-service notification.
            val parts = buildList {
                if (wifiOk) add("WiFi")
                if (relayOk) add("Relay")
                if (btOk) add("Bluetooth")
            }
            if (parts.isNotEmpty()) {
                _state.value = ConnectState.CONNECTED
                val label = if (parts.size == 1) "Connected via ${parts.first()}"
                            else "Connected \u2014 ${parts.joinToString(" + ")}"
                _statusText.value = label
                walkieService?.updateNotification("Radio active \u2014 ${parts.joinToString(" + ")}")
                activeTransport = when {
                    wifiOk && relayOk -> "both"
                    wifiOk -> "wifi"
                    relayOk -> "cellular"
                    btOk -> "bluetooth"
                    else -> "none"
                }
            } else {
                _state.value = ConnectState.FAILED
                _statusText.value = if (btAllowed)
                    "Connection failed \u2014 no peers in range" else "Connection failed"
            }

            wifiOk || relayOk || btOk
        }

        if (connected) {
            registerNetworkCallback()
        }

        return connected
    }

    /** Connect relay silently alongside WiFi — doesn't override state/status. */
    private suspend fun connectCellularSilent(walkieService: WalkieService?): Boolean {
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
        // Wire bidirectional refs so PttCoordinator can push HEARTBEAT/PTT_STOP_V2/RECV_ACK
        // frames over the relay, and inbound control frames from the relay reach the
        // coordinator. Without this the DO sweeper closes the WS after 8s of silence.
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
                return true
            }
            kotlinx.coroutines.delay(100)
        }

        Log.d(TAG, "Relay connection timed out — WiFi-only mode")
        return false
    }

    private suspend fun connectCellular(walkieService: WalkieService?): Boolean {
        _state.value = ConnectState.TRYING_CELLULAR
        _statusText.value = "Connecting via relay..."
        Log.d(TAG, "Trying cellular relay")

        val sessionId = SassyTalkNative.getSessionId()
        if (sessionId.isNullOrBlank()) {
            Log.e(TAG, "No session ID for cellular")
            _state.value = ConnectState.FAILED
            _statusText.value = "Connection failed \u2014 no session"
            return false
        }

        SassyTalkNative.cellularSetRoom(sessionId)
        val client = CellularWebSocketClient()
        client.peerId = com.sassyconsulting.sassytalkie.InstallId.get(context)
        client.onRelayReady = { onRelayReady() }
        // Wire bidirectional refs so PttCoordinator can push HEARTBEAT/PTT_STOP_V2/RECV_ACK
        // frames over the relay, and inbound control frames from the relay reach the
        // coordinator. Without this the DO sweeper closes the WS after 8s of silence.
        val coord = walkieService?.pttCoordinator
        client.pttCoordinator = coord
        coord?.cellularClient = client
        cellularClient = client
        client.connect()

        val iterations = (CELLULAR_TIMEOUT_MS / 100).toInt()
        for (i in 0 until iterations) {
            if (client.isConnected()) {
                Log.d(TAG, "Cellular relay connected")
                walkieService?.updateNotification("Radio active \u2014 Cellular")
                _state.value = ConnectState.CONNECTED
                _statusText.value = "Connected via Cellular"
                activeTransport = "cellular"
                com.sassyconsulting.sassytalkie.PresenceClient
                    .uploadCurrentToken(context, sessionId)
                return true
            }
            kotlinx.coroutines.delay(100)
        }

        client.disconnect()
        _state.value = ConnectState.FAILED
        _statusText.value = "Connection failed"
        Log.e(TAG, "Cellular relay failed")
        return false
    }

    /**
     * Register a NetworkCallback to detect WiFi loss and auto-failover to cellular.
     */
    private fun registerNetworkCallback() {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

        // Unregister any previous callback
        unregisterNetworkCallback()

        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onLost(network: Network) {
                Log.w(TAG, "Network lost — activeTransport=$activeTransport")

                if (activeTransport == "both") {
                    // WiFi dropped but relay is still connected — seamless failover
                    Log.i(TAG, "WiFi lost, relay still active — seamless failover")
                    activeTransport = "cellular"
                    walkieServiceRef?.releaseMulticastLock()
                    _state.value = ConnectState.CONNECTED
                    _statusText.value = "Connected via Cloudflare Relay"
                    walkieServiceRef?.updateNotification("Radio active \u2014 Cloudflare Relay")
                } else if (activeTransport == "wifi") {
                    // WiFi-only mode lost — try to connect relay
                    Log.w(TAG, "WiFi lost, no relay — attempting reconnect")
                    activeTransport = "none"
                    walkieServiceRef?.releaseMulticastLock()
                    _statusText.value = "Reconnecting..."

                    scope.launch {
                        val ok = connectCellular(walkieServiceRef)
                        if (!ok) {
                            Log.e(TAG, "Relay fallback failed after WiFi loss")
                            _state.value = ConnectState.FAILED
                            _statusText.value = "Disconnected \u2014 tap to retry"
                        }
                    }
                }
            }

            override fun onAvailable(network: Network) {
                Log.d(TAG, "Network available — activeTransport=$activeTransport")
                // If we were relay-only and WiFi came back, reconnect multicast
                if (activeTransport == "cellular" && hasWifi()) {
                    scope.launch {
                        walkieServiceRef?.acquireMulticastLock()
                        val wifiOk = SassyTalkNative.connectWifiMulticast()
                        if (wifiOk) {
                            activeTransport = "both"
                            _state.value = ConnectState.CONNECTED
                            _statusText.value = "Connected \u2014 WiFi + Relay"
                            walkieServiceRef?.updateNotification("Radio active \u2014 WiFi + Relay")
                            Log.i(TAG, "WiFi restored, dual transport active")
                        }
                    }
                }
            }

            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
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

    /** Called by CellularWebSocketClient when DO sends "welcome" confirmation. */
    fun onRelayReady() {
        _relayReady.value = true
        Log.i(TAG, "Relay confirmed ready (DO welcome received)")
    }

    fun reset() {
        _state.value = ConnectState.IDLE
        _statusText.value = ""
        _relayReady.value = false
        activeTransport = "none"
    }

    fun disconnect() {
        unregisterNetworkCallback()
        walkieServiceRef?.pttCoordinator?.cellularClient = null
        cellularClient?.disconnect()
        cellularClient = null
        activeTransport = "none"
        _relayReady.value = false
        _state.value = ConnectState.IDLE
        // Cancel any in-flight failover coroutines but leave the scope alive
        // so the manager can be reused for a future connect.
        (scope.coroutineContext[Job] as? Job)?.children?.forEach { it.cancel() }
    }

    /**
     * Full teardown — cancels the entire scope (including the SupervisorJob
     * itself) and replaces it with a fresh one. Call this when the manager
     * is being recycled (e.g. process restart, config-change recreation).
     *
     * `disconnect()` alone only cancels CHILD jobs; the SupervisorJob lives
     * forever, leaking memory and (more subtly) keeping any structured-
     * concurrency parents alive that reference this scope.
     */
    fun shutdown() {
        disconnect()
        scope.cancel()
        scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    }
}
