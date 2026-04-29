package com.sassyconsulting.sassytalkie.ui

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log
import kotlinx.coroutines.CoroutineScope
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
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

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

        val connected = withContext(Dispatchers.IO) {
            var wifiOk = false

            // Always try WiFi multicast for local peers
            if (hasWifi()) {
                _state.value = ConnectState.TRYING_WIFI
                _statusText.value = "Trying WiFi..."
                Log.d(TAG, "WiFi detected, trying multicast")

                walkieService?.acquireMulticastLock()
                wifiOk = SassyTalkNative.connectWifiMulticast()

                if (wifiOk) {
                    Log.d(TAG, "WiFi multicast connected")
                    activeTransport = "wifi"
                }
            }

            // ALWAYS also connect the relay for remote peers
            // (relay works alongside WiFi — local peers use multicast,
            // remote peers use relay, RX deduplicates by sender_id)
            val relayOk = connectCellularSilent(walkieService)

            if (wifiOk && relayOk) {
                _state.value = ConnectState.CONNECTED
                _statusText.value = "Connected \u2014 WiFi + Relay"
                walkieService?.updateNotification("Radio active \u2014 WiFi + Relay")
                activeTransport = "both"
            } else if (wifiOk) {
                _state.value = ConnectState.CONNECTED
                _statusText.value = "Connected via WiFi"
                walkieService?.updateNotification("Radio active \u2014 WiFi")
                activeTransport = "wifi"
            } else if (relayOk) {
                _state.value = ConnectState.CONNECTED
                _statusText.value = "Connected via Cloudflare Relay"
                walkieService?.updateNotification("Radio active \u2014 Cloudflare Relay")
                activeTransport = "cellular"
            } else {
                _state.value = ConnectState.FAILED
                _statusText.value = "Connection failed"
            }

            wifiOk || relayOk
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
        client.onRelayReady = { onRelayReady() }
        cellularClient = client
        client.connect()

        val iterations = (CELLULAR_TIMEOUT_MS / 100).toInt()
        for (i in 0 until iterations) {
            if (client.isConnected()) {
                Log.d(TAG, "Relay connected alongside WiFi")
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
        client.onRelayReady = { onRelayReady() }
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
        cellularClient?.disconnect()
        cellularClient = null
        activeTransport = "none"
        _relayReady.value = false
        _state.value = ConnectState.IDLE
        // Cancel any in-flight failover coroutines but leave the scope alive
        // so the manager can be reused for a future connect.
        (scope.coroutineContext[Job] as? Job)?.children?.forEach { it.cancel() }
    }
}
