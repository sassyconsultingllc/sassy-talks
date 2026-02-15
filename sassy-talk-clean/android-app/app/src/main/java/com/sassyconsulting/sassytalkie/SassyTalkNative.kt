package com.sassyconsulting.sassytalkie

import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * JNI bridge to Rust native library.
 *
 * The native library (libsassytalkie.so) handles:
 * - Bluetooth RFCOMM + WiFi multicast transport
 * - AES-256-GCM encryption with QR-based key exchange
 * - Audio capture and playback
 * - Smart transport selection (BT default, WiFi preferred)
 * - User registry (mute/favorites)
 */
object SassyTalkNative {

    private const val TAG = "SassyTalkNative"
    private var initialized = false

    /** Transport type constants matching Rust enum */
    const val TRANSPORT_NONE = 0
    const val TRANSPORT_BLUETOOTH = 1
    const val TRANSPORT_WIFI = 2

    init {
        try {
            System.loadLibrary("sassytalkie")
            Log.i(TAG, "Native library loaded successfully")
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "Failed to load native library: ${e.message}")
        }
    }

    // ── Lifecycle ──

    fun init(): Boolean {
        return try {
            initialized = nativeInit()
            Log.i(TAG, "Native init: $initialized")
            initialized
        } catch (e: Exception) {
            Log.e(TAG, "Init failed: ${e.message}")
            false
        }
    }

    fun shutdown() {
        if (initialized) {
            try {
                nativeShutdown()
                Log.i(TAG, "Shutdown complete")
            } catch (e: Exception) {
                Log.e(TAG, "Shutdown failed: ${e.message}")
            }
            initialized = false
        }
    }

    // ── PTT ──

    fun pttStart() {
        if (initialized) {
            nativePttStart()
            Log.d(TAG, "PTT Started")
        }
    }

    fun pttStop() {
        if (initialized) {
            nativePttStop()
            Log.d(TAG, "PTT Stopped")
        }
    }

    fun setChannel(channel: Int) {
        if (initialized && channel in 1..99) {
            nativeSetChannel(channel.toByte())
            Log.d(TAG, "Channel set to $channel")
        }
    }

    // ── Transport ──

    /** Get active transport: 0=None, 1=Bluetooth, 2=WiFi */
    fun getTransport(): Int {
        return if (initialized) {
            try {
                nativeGetTransport().toInt()
            } catch (e: Exception) {
                TRANSPORT_NONE
            }
        } else {
            TRANSPORT_NONE
        }
    }

    fun isConnected(): Boolean = getTransport() != TRANSPORT_NONE

    fun getTransportName(): String {
        return when (getTransport()) {
            TRANSPORT_BLUETOOTH -> "BT"
            TRANSPORT_WIFI -> "WiFi"
            else -> "---"
        }
    }

    // ── Device Management ──

    data class BluetoothDeviceInfo(val name: String, val address: String)

    fun getPairedDevices(): List<BluetoothDeviceInfo> {
        if (!initialized) return emptyList()
        return try {
            val json = nativeGetPairedDevices()
            val array = JSONArray(json)
            (0 until array.length()).map { i ->
                val obj = array.getJSONObject(i)
                BluetoothDeviceInfo(
                    name = obj.optString("name", "Unknown"),
                    address = obj.getString("address")
                )
            }
        } catch (e: Exception) {
            Log.e(TAG, "getPairedDevices failed: ${e.message}")
            emptyList()
        }
    }

    fun connectDevice(address: String): Boolean {
        if (!initialized) return false
        return try {
            nativeConnectDevice(address)
        } catch (e: Exception) {
            Log.e(TAG, "connectDevice failed: ${e.message}")
            false
        }
    }

    fun startListening(): Boolean {
        if (!initialized) return false
        return try {
            nativeStartListening()
        } catch (e: Exception) {
            Log.e(TAG, "startListening failed: ${e.message}")
            false
        }
    }

    fun disconnect(): Boolean {
        if (!initialized) return false
        return try {
            nativeDisconnect()
        } catch (e: Exception) {
            Log.e(TAG, "disconnect failed: ${e.message}")
            false
        }
    }

    // ── QR Auth / Session ──

    fun generateSessionQR(durationHours: Int = 24): String {
        if (!initialized) return ""
        return try {
            nativeGenerateSessionQR(durationHours)
        } catch (e: Exception) {
            Log.e(TAG, "generateSessionQR failed: ${e.message}")
            ""
        }
    }

    fun importSessionFromQR(qrJson: String): Boolean {
        if (!initialized) return false
        return try {
            nativeImportSessionFromQR(qrJson)
        } catch (e: Exception) {
            Log.e(TAG, "importSessionFromQR failed: ${e.message}")
            false
        }
    }

    fun isAuthenticated(): Boolean {
        if (!initialized) return false
        return try {
            nativeIsAuthenticated()
        } catch (e: Exception) {
            false
        }
    }

    fun getSessionStatus(): String {
        if (!initialized) return "{}"
        return try {
            nativeGetSessionStatus()
        } catch (e: Exception) {
            "{}"
        }
    }

    // ── User Management (Mute/Favorites) ──

    data class UserInfo(
        val id: String,
        val name: String,
        val isMuted: Boolean,
        val isFavorite: Boolean
    )

    fun getUsers(): List<UserInfo> {
        if (!initialized) return emptyList()
        return try {
            val json = nativeGetUsers()
            val array = JSONArray(json)
            (0 until array.length()).map { i ->
                val obj = array.getJSONObject(i)
                UserInfo(
                    id = obj.getString("id"),
                    name = obj.optString("name", "Unknown"),
                    isMuted = obj.optBoolean("is_muted", false),
                    isFavorite = obj.optBoolean("is_favorite", false)
                )
            }
        } catch (e: Exception) {
            Log.e(TAG, "getUsers failed: ${e.message}")
            emptyList()
        }
    }

    fun setUserMuted(userId: String, muted: Boolean) {
        if (initialized) {
            try {
                nativeSetMuted(userId, muted)
            } catch (e: Exception) {
                Log.e(TAG, "setUserMuted failed: ${e.message}")
            }
        }
    }

    fun setUserFavorite(userId: String, favorite: Boolean) {
        if (initialized) {
            try {
                nativeSetFavorite(userId, favorite)
            } catch (e: Exception) {
                Log.e(TAG, "setUserFavorite failed: ${e.message}")
            }
        }
    }

    // ── Bluetooth Status ──

    fun isBluetoothEnabled(): Boolean {
        if (!initialized) return false
        return try {
            nativeIsBluetoothEnabled()
        } catch (e: Exception) { false }
    }

    fun enableBluetooth(): Boolean {
        if (!initialized) return false
        return try {
            nativeEnableBluetooth()
        } catch (e: Exception) { false }
    }

    fun getConnectedDevice(): JSONObject? {
        if (!initialized) return null
        return try {
            val json = nativeGetConnectedDevice()
            if (json.isNotEmpty() && json != "{}") JSONObject(json) else null
        } catch (e: Exception) { null }
    }

    /** Get app state: 0=Init, 1=Ready, 2=Connecting, 3=Connected, 4=TX, 5=RX, 6=Disconnecting, 7=Error */
    fun getAppState(): Int {
        if (!initialized) return 0
        return try {
            nativeGetAppState().toInt()
        } catch (e: Exception) { 0 }
    }

    // ── Session Management ──

    fun clearSession() {
        if (initialized) {
            try { nativeClearSession() } catch (e: Exception) {
                Log.e(TAG, "clearSession failed: ${e.message}")
            }
        }
    }

    // ── User Registration ──

    fun registerUser(userId: String, userName: String) {
        if (initialized) {
            try { nativeRegisterUser(userId, userName) } catch (e: Exception) {
                Log.e(TAG, "registerUser failed: ${e.message}")
            }
        }
    }

    fun getFavorites(): JSONObject? {
        if (!initialized) return null
        return try {
            JSONObject(nativeGetFavorites())
        } catch (e: Exception) { null }
    }

    fun deriveUserId(sessionKeyB64: String): String? {
        if (!initialized) return null
        return try {
            nativeDeriveUserId(sessionKeyB64)
        } catch (e: Exception) { null }
    }

    // ── Crypto ──

    fun generatePsk(): String? {
        if (!initialized) return null
        return try {
            nativeGeneratePsk()
        } catch (e: Exception) { null }
    }

    fun setPsk(pskB64: String): Boolean {
        if (!initialized) return false
        return try {
            nativeSetPsk(pskB64)
        } catch (e: Exception) { false }
    }

    /** Start ECDH key exchange, returns local public key as base64 */
    fun keyExchangeInit(): String? {
        if (!initialized) return null
        return try {
            nativeKeyExchangeInit()
        } catch (e: Exception) { null }
    }

    /** Complete ECDH key exchange with remote public key (base64) */
    fun keyExchangeComplete(remotePubB64: String): Boolean {
        if (!initialized) return false
        return try {
            nativeKeyExchangeComplete(remotePubB64)
        } catch (e: Exception) { false }
    }

    // ── Permissions ──

    fun checkPermissions(): JSONObject? {
        if (!initialized) return null
        return try {
            JSONObject(nativeCheckPermissions())
        } catch (e: Exception) { null }
    }

    fun onPermissionResult(permission: String, granted: Boolean) {
        if (initialized) {
            try { nativeOnPermissionResult(permission, granted) } catch (e: Exception) {
                Log.e(TAG, "onPermissionResult failed: ${e.message}")
            }
        }
    }

    fun getMissingPermissions(): List<String> {
        if (!initialized) return emptyList()
        return try {
            val json = nativeGetMissingPermissions()
            val array = JSONArray(json)
            (0 until array.length()).map { array.getString(it) }
        } catch (e: Exception) { emptyList() }
    }

    fun getPermissionRationale(permission: String): String {
        if (!initialized) return ""
        return try {
            nativeGetPermissionRationale(permission)
        } catch (e: Exception) { "" }
    }

    // ── WiFi Transport ──

    /** Get WiFi state: 0=Inactive, 1=Discovering, 2=Active, 3=Error */
    fun getWifiState(): Int {
        if (!initialized) return 0
        return try {
            nativeGetWifiState().toInt()
        } catch (e: Exception) { 0 }
    }

    fun getWifiPeers(): List<JSONObject> {
        if (!initialized) return emptyList()
        return try {
            val json = nativeGetWifiPeers()
            val array = JSONArray(json)
            (0 until array.length()).map { array.getJSONObject(it) }
        } catch (e: Exception) { emptyList() }
    }

    fun hasWifiPeers(): Boolean {
        if (!initialized) return false
        return try {
            nativeHasWifiPeers()
        } catch (e: Exception) { false }
    }

    fun initWifi(): Boolean {
        if (!initialized) return false
        return try {
            nativeInitWifi()
        } catch (e: Exception) { false }
    }

    // ── Audio Cache (Dane.com-style multi-speaker store/replay) ──

    /** Cache mode constants */
    const val CACHE_MODE_LIVE = 0
    const val CACHE_MODE_QUEUE = 1
    const val CACHE_MODE_REPLAY = 2

    /** Get audio cache status as JSON: mode, queued_utterances, current_speaker, etc. */
    fun getCacheStatus(): JSONObject? {
        if (!initialized) return null
        return try {
            JSONObject(nativeGetCacheStatus())
        } catch (e: Exception) {
            Log.e(TAG, "getCacheStatus failed: ${e.message}")
            null
        }
    }

    /** Skip the currently playing utterance, advance to next in queue */
    fun skipCurrentUtterance() {
        if (initialized) {
            try { nativeSkipCurrentUtterance() } catch (e: Exception) {
                Log.e(TAG, "skipCurrentUtterance failed: ${e.message}")
            }
        }
    }

    /** Set audio cache mode: CACHE_MODE_LIVE=0, CACHE_MODE_QUEUE=1, CACHE_MODE_REPLAY=2 */
    fun setCacheMode(mode: Int) {
        if (initialized && mode in 0..2) {
            try { nativeSetCacheMode(mode.toByte()) } catch (e: Exception) {
                Log.e(TAG, "setCacheMode failed: ${e.message}")
            }
        }
    }

    /** Clear all cached audio, reset to Live mode */
    fun clearAudioCache() {
        if (initialized) {
            try { nativeClearAudioCache() } catch (e: Exception) {
                Log.e(TAG, "clearAudioCache failed: ${e.message}")
            }
        }
    }

    /** Replay a previous utterance from history by index */
    fun replayUtterance(index: Int): Boolean {
        if (!initialized) return false
        return try {
            nativeReplayUtterance(index)
        } catch (e: Exception) {
            Log.e(TAG, "replayUtterance failed: ${e.message}")
            false
        }
    }

    /** Sync user info (mute/favorite status) from UserRegistry into the audio cache */
    fun syncCacheUserInfo() {
        if (initialized) {
            try { nativeSyncCacheUserInfo() } catch (e: Exception) {
                Log.e(TAG, "syncCacheUserInfo failed: ${e.message}")
            }
        }
    }

    // ── Status ──

    /** Get BT state: 0=Disconnected, 1=Connecting, 2=Connected, 3=Listening */
    fun getBtState(): Int {
        if (!initialized) return 0
        return try {
            nativeGetBtState().toInt()
        } catch (e: Exception) { 0 }
    }

    fun isPttActive(): Boolean {
        if (!initialized) return false
        return try {
            nativeIsPttActive()
        } catch (e: Exception) { false }
    }

    fun getDeviceName(): String {
        if (!initialized) return ""
        return try {
            nativeGetDeviceName()
        } catch (e: Exception) { "" }
    }

    /** Check if encryption is active (QR auth completed). TX is blocked without this. */
    fun isEncrypted(): Boolean {
        if (!initialized) return false
        return try {
            nativeIsEncrypted()
        } catch (e: Exception) { false }
    }

    // ── Native method declarations ──

    // Lifecycle
    @JvmStatic private external fun nativeInit(): Boolean
    @JvmStatic private external fun nativeShutdown()

    // PTT
    @JvmStatic private external fun nativePttStart()
    @JvmStatic private external fun nativePttStop()
    @JvmStatic private external fun nativeSetChannel(channel: Byte)

    // Transport
    @JvmStatic private external fun nativeGetTransport(): Byte

    // Device management
    @JvmStatic private external fun nativeGetPairedDevices(): String
    @JvmStatic private external fun nativeConnectDevice(address: String): Boolean
    @JvmStatic private external fun nativeStartListening(): Boolean
    @JvmStatic private external fun nativeDisconnect(): Boolean

    // QR Auth / Session
    @JvmStatic private external fun nativeGenerateSessionQR(durationHours: Int): String
    @JvmStatic private external fun nativeImportSessionFromQR(qrJson: String): Boolean
    @JvmStatic private external fun nativeIsAuthenticated(): Boolean
    @JvmStatic private external fun nativeGetSessionStatus(): String

    // User management
    @JvmStatic private external fun nativeGetUsers(): String
    @JvmStatic private external fun nativeSetMuted(userId: String, muted: Boolean)
    @JvmStatic private external fun nativeSetFavorite(userId: String, favorite: Boolean)

    // Extended: BT/WiFi status, session, users, permissions
    @JvmStatic private external fun nativeIsBluetoothEnabled(): Boolean
    @JvmStatic private external fun nativeEnableBluetooth(): Boolean
    @JvmStatic private external fun nativeGetConnectedDevice(): String
    @JvmStatic private external fun nativeGetAppState(): Byte
    @JvmStatic private external fun nativeClearSession()
    @JvmStatic private external fun nativeRegisterUser(userId: String, userName: String)
    @JvmStatic private external fun nativeGetFavorites(): String
    @JvmStatic private external fun nativeDeriveUserId(sessionKeyB64: String): String
    @JvmStatic private external fun nativeGeneratePsk(): String
    @JvmStatic private external fun nativeSetPsk(pskB64: String): Boolean
    @JvmStatic private external fun nativeKeyExchangeInit(): String
    @JvmStatic private external fun nativeKeyExchangeComplete(remotePubB64: String): Boolean
    @JvmStatic private external fun nativeCheckPermissions(): String
    @JvmStatic private external fun nativeOnPermissionResult(permission: String, granted: Boolean)
    @JvmStatic private external fun nativeGetMissingPermissions(): String
    @JvmStatic private external fun nativeGetPermissionRationale(permission: String): String
    @JvmStatic private external fun nativeGetWifiState(): Byte
    @JvmStatic private external fun nativeGetWifiPeers(): String
    @JvmStatic private external fun nativeGetBtState(): Byte
    @JvmStatic private external fun nativeIsPttActive(): Boolean
    @JvmStatic private external fun nativeInitWifi(): Boolean
    @JvmStatic private external fun nativeGetDeviceName(): String
    @JvmStatic private external fun nativeHasWifiPeers(): Boolean
    @JvmStatic private external fun nativeIsEncrypted(): Boolean

    // Audio Cache (multi-speaker store/replay)
    @JvmStatic private external fun nativeGetCacheStatus(): String
    @JvmStatic private external fun nativeSkipCurrentUtterance()
    @JvmStatic private external fun nativeSetCacheMode(mode: Byte)
    @JvmStatic private external fun nativeClearAudioCache()
    @JvmStatic private external fun nativeReplayUtterance(index: Int): Boolean
    @JvmStatic private external fun nativeSyncCacheUserInfo()
}
