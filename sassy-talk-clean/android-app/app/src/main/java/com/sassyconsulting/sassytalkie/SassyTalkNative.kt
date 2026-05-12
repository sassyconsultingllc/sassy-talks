package com.sassyconsulting.sassytalkie

import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.json.JSONArray
import org.json.JSONObject

/**
 * JNI bridge to Rust native library.
 *
 * The native library (libsassytalkie.so) handles:
 * - WiFi multicast + WiFi Direct transport
 * - AES-256-GCM encryption with QR-based key exchange
 * - Audio capture and playback
 * - User registry (mute/favorites)
 */
object SassyTalkNative {

    private const val TAG = "SassyTalkNative"
    private const val SESSION_PREFS = "sassy_session"
    private var initialized = false
    var appContext: android.content.Context? = null

    /**
     * Open the session-prefs store backed by Android Keystore via
     * EncryptedSharedPreferences. If Keystore init fails, return null —
     * sessions become in-memory-only for that launch, and the user must
     * re-pair via QR. We deliberately do NOT fall back to cleartext
     * SharedPreferences: writing AES session keys to disk in the clear
     * would defeat the threat model (post-conversation device recovery).
     *
     * On the first call after an upgrade from a build that used the
     * cleartext fallback, we also nuke the unencrypted prefs file so any
     * keys left over there are gone.
     */
    /**
     * Read the encrypted per-channel session JSON written by joinChannel /
     * createChannel paths. Returns the empty string when no session exists
     * (or the keystore is unavailable). The UI used to read directly from
     * MODE_PRIVATE SharedPreferences, but those are now both unwritten and
     * actively purged on launch — hence "No active session" in the QR dialog
     * even when one was just created. Always go through this accessor.
     */
    fun getChannelSessionJson(channel: Int): String {
        return try {
            sessionPrefs()?.getString("session_ch_$channel", null) ?: ""
        } catch (e: Exception) {
            Log.w(TAG, "getChannelSessionJson failed: ${e.message}")
            ""
        }
    }

    private fun sessionPrefs(): SharedPreferences? {
        val ctx = appContext ?: return null
        // One-shot: scrub any plaintext session prefs left behind by older
        // builds that fell back to MODE_PRIVATE. Idempotent.
        purgeLegacyPlaintextPrefs(ctx)
        return try {
            val masterKey = MasterKey.Builder(ctx)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            EncryptedSharedPreferences.create(
                ctx,
                SESSION_PREFS,
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            )
        } catch (e: Exception) {
            Log.w(TAG, "EncryptedSharedPreferences unavailable; session keys will be in-memory only this launch: ${e.message}")
            null
        }
    }

    @Volatile private var legacyPurged = false
    private fun purgeLegacyPlaintextPrefs(ctx: android.content.Context) {
        if (legacyPurged) return
        legacyPurged = true
        try {
            // If the file exists and isn't an EncryptedSharedPreferences blob,
            // it means an older build wrote plaintext keys here. Wipe it.
            val f = java.io.File(ctx.applicationInfo.dataDir, "shared_prefs/$SESSION_PREFS.xml")
            if (f.exists()) {
                val sample = f.readText(Charsets.UTF_8).take(256)
                val looksEncrypted = sample.contains("__androidx_security_crypto_encrypted_prefs_")
                if (!looksEncrypted) {
                    Log.w(TAG, "Wiping legacy plaintext session prefs at ${f.path}")
                    ctx.getSharedPreferences(SESSION_PREFS, android.content.Context.MODE_PRIVATE)
                        .edit().clear().commit()
                    f.delete()
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "Legacy prefs purge skipped: ${e.message}")
        }
    }

    /** Transport type constants matching Rust enum */
    const val TRANSPORT_NONE = 0
    const val TRANSPORT_WIFI = 2
    const val TRANSPORT_WIFI_DIRECT = 3
    const val TRANSPORT_CELLULAR = 4
    const val TRANSPORT_BLUETOOTH = 5

    init {
        try {
            System.loadLibrary("sassytalkie")
            Log.i(TAG, "Native library loaded successfully")
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "Failed to load native library: ${e.message}")
        }
    }

    // ── Lifecycle ──

    fun isInitialized(): Boolean = initialized

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

    /** Set the device display name (sent with audio so peers see who's talking) */
    fun setDeviceName(name: String) {
        if (initialized && name.isNotBlank()) {
            try {
                nativeSetDeviceName(name)
                Log.i(TAG, "Device name set to: $name")
            } catch (e: Exception) {
                Log.e(TAG, "setDeviceName failed: ${e.message}")
            }
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
        if (!initialized) return
        val transport = getTransport()
        val btConnected = transport == TRANSPORT_BLUETOOTH
        val btPeers = bluetoothTransport?.connectedPeerCount ?: 0
        Log.i(TAG, "PTT START pressed — BT connected: $btConnected, BT peers: $btPeers, transport: ${getTransportName()}")

        // Connection guard: don't start if no transport is active
        if (transport == TRANSPORT_NONE && !btConnected && btPeers == 0) {
            Log.w(TAG, "PTT blocked: no connected peers")
            return
        }

        nativePttStart()

        // Start BT TX pump if BT transport is active
        if (btConnected || btPeers > 0) {
            bluetoothTransport?.startTxPump()
            Log.i(TAG, "BT TX pump started ($btPeers peers)")
        }
        Log.d(TAG, "PTT Started")
    }

    fun pttStop() {
        if (!initialized) return
        nativePttStop()

        // Stop BT TX pump
        bluetoothTransport?.stopTxPump()
        Log.d(TAG, "PTT Stopped")
    }

    /** Set PTT buffer mode. true = buffer audio and burst-send on release. false = live stream. */
    fun setPttBufferMode(buffer: Boolean) {
        if (initialized) {
            try { nativeSetPttBufferMode(buffer) } catch (_: Exception) {}
        }
    }

    fun getPttBufferMode(): Boolean {
        if (!initialized) return true
        return try { nativeGetPttBufferMode() } catch (_: Exception) { true }
    }

    fun setChannel(channel: Int) {
        if (initialized && channel in 1..99) {
            nativeSetChannel(channel.toByte())
            Log.d(TAG, "Channel set to $channel")
        }
    }

    // ── Transport ──

    /** Get active transport: 0=None, 2=WiFi, 3=WiFi Direct */
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
            TRANSPORT_WIFI -> "WiFi"
            TRANSPORT_WIFI_DIRECT -> "P2P"
            TRANSPORT_CELLULAR -> "Cloudflare"
            TRANSPORT_BLUETOOTH -> "BT"
            else -> "---"
        }
    }

    /** Connect via WiFi multicast (cross-platform) */
    fun connectWifiMulticast(): Boolean {
        if (!initialized) return false
        return try {
            nativeConnectWifiMulticast()
        } catch (e: Exception) {
            Log.e(TAG, "connectWifiMulticast failed: ${e.message}")
            false
        }
    }

    // ── Connection Management ──

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
            val json = nativeGenerateSessionQR(durationHours)
            // Persist session so it survives app restart
            if (json.isNotEmpty()) {
                sessionPrefs()
                    ?.edit()?.putString("session_json", json)?.apply()
            }
            json
        } catch (e: Exception) {
            Log.e(TAG, "generateSessionQR failed: ${e.message}")
            ""
        }
    }

    fun importSessionFromQR(qrJson: String): Boolean {
        if (!initialized) return false
        return try {
            val ok = nativeImportSessionFromQR(qrJson)
            if (ok) {
                // Extract channel from JSON to persist per-channel
                val channel = try {
                    org.json.JSONObject(qrJson).optInt("channel", 1)
                } catch (_: Exception) { 1 }
                sessionPrefs()?.edit()?.putString("session_ch_$channel", qrJson)?.apply()
                saveCohortHistoryBlob()
            }
            ok
        } catch (e: Exception) {
            Log.e(TAG, "importSessionFromQR failed: ${e.message}")
            false
        }
    }

    /** Restore all previously persisted per-channel sessions (call after nativeInit). */
    fun restoreSession(): Boolean {
        if (!initialized) return false
        val prefs = sessionPrefs()
            ?: return false

        var anyRestored = false
        // Try per-channel keys first (new format)
        for (ch in 1..8) {
            val json = prefs.getString("session_ch_$ch", null) ?: continue
            try {
                if (nativeImportSessionFromQR(json)) {
                    anyRestored = true
                    Log.d(TAG, "Restored session for channel $ch")
                }
            } catch (e: Exception) {
                Log.d(TAG, "restoreSession ch$ch: expired or invalid, clearing")
                prefs.edit().remove("session_ch_$ch").apply()
            }
        }
        // Also try legacy single-session key for backward compat
        if (!anyRestored) {
            val legacyJson = prefs.getString("session_json", null)
            if (legacyJson != null) {
                try {
                    if (nativeImportSessionFromQR(legacyJson)) {
                        anyRestored = true
                        // Migrate legacy to per-channel
                        prefs.edit().putString("session_ch_1", legacyJson).remove("session_json").apply()
                    }
                } catch (_: Exception) {
                    prefs.edit().remove("session_json").apply()
                }
            }
        }
        return anyRestored
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
        // Clear per-channel sessions and legacy session_json, but preserve cohort_history_v1.
        val prefs = sessionPrefs() ?: return
        val editor = prefs.edit()
        for (ch in 1..8) editor.remove("session_ch_$ch")
        editor.remove("session_json")
        editor.apply()
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

    /** Replay a previous utterance from history by index (legacy) */
    fun replayUtterance(index: Int): Boolean {
        if (!initialized) return false
        return try {
            nativeReplayUtterance(index)
        } catch (e: Exception) {
            Log.e(TAG, "replayUtterance failed: ${e.message}")
            false
        }
    }

    /** Replay a previous utterance from history by unique ID */
    fun replayById(utteranceId: Long): Boolean {
        if (!initialized) return false
        return try {
            nativeReplayById(utteranceId)
        } catch (e: Exception) {
            Log.e(TAG, "replayById failed: ${e.message}")
            false
        }
    }

    /** Get the unique ID of the most recently added history utterance */
    fun lastHistoryId(): Long {
        if (!initialized) return -1
        return try { nativeLastHistoryId() } catch (_: Exception) { -1 }
    }

    /** Sync user info (mute/favorite status) from UserRegistry into the audio cache */
    fun syncCacheUserInfo() {
        if (initialized) {
            try { nativeSyncCacheUserInfo() } catch (e: Exception) {
                Log.e(TAG, "syncCacheUserInfo failed: ${e.message}")
            }
        }
    }

    // ── Cellular Transport (WebSocket relay) ──

    /** Set the cellular room ID (from QR session_id) */
    fun cellularSetRoom(roomId: String) {
        if (initialized && roomId.isNotBlank()) {
            try {
                nativeCellularSetRoom(roomId)
                Log.i(TAG, "Cellular room set: $roomId")
            } catch (e: Exception) {
                Log.e(TAG, "cellularSetRoom failed: ${e.message}")
            }
        }
    }

    /** Get the WebSocket URL for the cellular relay */
    fun cellularGetWsUrl(): String {
        if (!initialized) return ""
        return try {
            nativeCellularGetWsUrl()
        } catch (e: Exception) {
            Log.e(TAG, "cellularGetWsUrl failed: ${e.message}")
            ""
        }
    }

    /** Called when WebSocket connects successfully */
    fun cellularOnConnected(): Boolean {
        if (!initialized) return false
        return try {
            nativeCellularOnConnected()
        } catch (e: Exception) {
            Log.e(TAG, "cellularOnConnected failed: ${e.message}")
            false
        }
    }

    /** Called when WebSocket disconnects */
    fun cellularOnDisconnected(reason: String) {
        if (initialized) {
            try {
                nativeCellularOnDisconnected(reason)
            } catch (e: Exception) {
                Log.e(TAG, "cellularOnDisconnected failed: ${e.message}")
            }
        }
    }

    /** Called when WebSocket receives a binary message */
    fun cellularOnMessage(data: ByteArray) {
        if (initialized) {
            try {
                nativeCellularOnMessage(data)
            } catch (e: Exception) {
                Log.e(TAG, "cellularOnMessage failed: ${e.message}")
            }
        }
    }

    /** Called when WebSocket encounters an error */
    fun cellularOnError(error: String) {
        if (initialized) {
            try {
                nativeCellularOnError(error)
            } catch (e: Exception) {
                Log.e(TAG, "cellularOnError failed: ${e.message}")
            }
        }
    }

    /** Poll outbound queue — returns next packet to send via WS, or null */
    fun cellularPollOutbound(): ByteArray? {
        if (!initialized) return null
        return try {
            nativeCellularPollOutbound()
        } catch (e: Exception) { null }
    }

    /** Get cellular transport stats as JSON */
    fun cellularGetStats(): String {
        if (!initialized) return "{}"
        return try {
            nativeCellularGetStats()
        } catch (e: Exception) { "{}" }
    }

    /** Extract session_id from session status (used as room ID for cellular) */
    fun getSessionId(): String? {
        if (!initialized) return null
        return try {
            val json = JSONObject(nativeGetSessionStatus())
            // Per-channel format: find first active channel's session_id
            val channels = json.optJSONArray("channels")
            if (channels != null) {
                for (i in 0 until channels.length()) {
                    val ch = channels.getJSONObject(i)
                    if (ch.optBoolean("active", false)) {
                        val id = ch.optString("session_id", "")
                        if (id.isNotEmpty()) return id
                    }
                }
            }
            // Legacy fallback
            val id = json.optString("session_id", "")
            if (id.isNotEmpty()) id else null
        } catch (e: Exception) { null }
    }

    // ── Bluetooth Transport ──

    /** Reference to Kotlin-managed BT transport (set by Activity) */
    @Volatile
    var bluetoothTransport: com.sassyconsulting.sassytalkie.service.BluetoothTransport? = null

    /** Called by BluetoothTransport when RFCOMM connects */
    fun btConnected() {
        if (initialized) {
            try {
                nativeBtConnected()
                Log.i(TAG, "BT: native transport notified (connected)")
            } catch (e: Exception) {
                Log.e(TAG, "btConnected failed: ${e.message}")
            }
        }
    }

    /** Called by BluetoothTransport when RFCOMM disconnects */
    fun btDisconnected() {
        if (initialized) {
            try {
                nativeBtDisconnected()
                Log.i(TAG, "BT: native transport notified (disconnected)")
            } catch (e: Exception) {
                Log.e(TAG, "btDisconnected failed: ${e.message}")
            }
        }
    }

    /** Get current channel for BT channel sync */
    fun getChannel(): Int {
        if (!initialized) return 1
        return try {
            nativeGetChannel().toInt() and 0xFF
        } catch (e: Exception) { 1 }
    }

    /** Encode one audio frame for BT TX (mic → ADPCM → wire frame bytes) */
    fun btEncodeFrame(): ByteArray? {
        if (!initialized) return null
        return try {
            nativeBtEncodeFrame()
        } catch (e: Exception) { null }
    }

    /** Decode a BT-received audio frame (wire frame → ADPCM → play) */
    fun btDecodeFrame(data: ByteArray): Boolean {
        if (!initialized) return false
        return try {
            nativeBtDecodeFrame(data)
        } catch (e: Exception) { false }
    }

    // ── Status ──

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
    @JvmStatic private external fun nativeSetPttBufferMode(bufferMode: Boolean)
    @JvmStatic private external fun nativeGetPttBufferMode(): Boolean
    @JvmStatic private external fun nativeReplayById(utteranceId: Long): Boolean
    @JvmStatic private external fun nativeLastHistoryId(): Long
    @JvmStatic private external fun nativeSetChannel(channel: Byte)

    // Transport
    @JvmStatic private external fun nativeGetTransport(): Byte

    // Connection
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

    // WiFi status, session, users, permissions
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
    @JvmStatic private external fun nativeIsPttActive(): Boolean
    @JvmStatic private external fun nativeInitWifi(): Boolean
    @JvmStatic private external fun nativeGetDeviceName(): String
    @JvmStatic private external fun nativeSetDeviceName(name: String)
    @JvmStatic private external fun nativeHasWifiPeers(): Boolean
    @JvmStatic private external fun nativeIsEncrypted(): Boolean
    @JvmStatic private external fun nativeConnectWifiMulticast(): Boolean

    // Audio Cache (multi-speaker store/replay)
    @JvmStatic private external fun nativeGetCacheStatus(): String
    @JvmStatic private external fun nativeSkipCurrentUtterance()
    @JvmStatic private external fun nativeSetCacheMode(mode: Byte)
    @JvmStatic private external fun nativeClearAudioCache()
    @JvmStatic private external fun nativeReplayUtterance(index: Int): Boolean
    @JvmStatic private external fun nativeSyncCacheUserInfo()

    // Cellular Transport (WebSocket relay)
    @JvmStatic private external fun nativeCellularSetRoom(roomId: String)
    @JvmStatic private external fun nativeCellularGetWsUrl(): String
    @JvmStatic private external fun nativeCellularOnConnected(): Boolean
    @JvmStatic private external fun nativeCellularOnDisconnected(reason: String)
    @JvmStatic private external fun nativeCellularOnMessage(data: ByteArray)
    @JvmStatic private external fun nativeCellularOnError(error: String)
    @JvmStatic private external fun nativeCellularPollOutbound(): ByteArray?
    @JvmStatic private external fun nativeCellularGetStats(): String

    // Bluetooth Transport (RFCOMM, Kotlin-managed sockets)
    @JvmStatic private external fun nativeGetChannel(): Byte
    @JvmStatic private external fun nativeBtConnected()
    @JvmStatic private external fun nativeBtDisconnected()
    @JvmStatic private external fun nativeBtEncodeFrame(): ByteArray?
    @JvmStatic private external fun nativeBtDecodeFrame(data: ByteArray): Boolean

    // ── Per-channel session management ──

    @JvmStatic private external fun nativeGenerateChannelQR(
        channel: Int, durationHours: Int, groupName: String?, cohortId: String?,
    ): String
    @JvmStatic private external fun nativeGetCohortHistory(): String
    @JvmStatic private external fun nativeLoadCohortHistory(blob: String)
    @JvmStatic private external fun nativeRemoveCohort(cohortId: String)
    @JvmStatic private external fun nativeClearCohortHistory()
    @JvmStatic private external fun nativeGetActiveCohortId(channel: Int): String
    @JvmStatic private external fun nativeSnapshotCohortParticipants(channel: Int, participantsJson: String)
    @JvmStatic private external fun nativeSetSubchannel(subchannel: Byte)
    @JvmStatic private external fun nativeGetChannelInfo(): String

    fun setSubchannel(sub: Int) {
        if (initialized) try { nativeSetSubchannel(sub.toByte()) } catch (_: Exception) {}
    }
    @JvmStatic private external fun nativeSetGroupName(channel: Int, name: String)
    @JvmStatic private external fun nativeGetGroupName(channel: Int): String

    private const val COHORT_HISTORY_PREF_KEY = "cohort_history_v1"

    fun getCohortHistory(): String {
        if (!initialized) return "[]"
        return try { nativeGetCohortHistory() } catch (_: Exception) { "[]" }
    }

    fun removeCohort(cohortId: String) {
        if (!initialized) return
        try {
            nativeRemoveCohort(cohortId)
            saveCohortHistoryBlob()
        } catch (e: Exception) {
            Log.e(TAG, "removeCohort failed: ${e.message}")
        }
    }

    fun clearCohortHistory() {
        if (!initialized) return
        try {
            nativeClearCohortHistory()
            sessionPrefs()?.edit()?.remove(COHORT_HISTORY_PREF_KEY)?.apply()
        } catch (e: Exception) {
            Log.e(TAG, "clearCohortHistory failed: ${e.message}")
        }
    }

    fun getActiveCohortId(channel: Int): String {
        if (!initialized) return ""
        return try { nativeGetActiveCohortId(channel) } catch (_: Exception) { "" }
    }

    fun snapshotCohortParticipants(channel: Int, participantsJson: String) {
        if (!initialized) return
        try {
            nativeSnapshotCohortParticipants(channel, participantsJson)
            saveCohortHistoryBlob()
        } catch (e: Exception) {
            Log.w(TAG, "snapshotCohortParticipants failed: ${e.message}")
        }
    }

    private fun saveCohortHistoryBlob() {
        try {
            val blob = nativeGetCohortHistory()
            sessionPrefs()?.edit()?.putString(COHORT_HISTORY_PREF_KEY, blob)?.apply()
        } catch (e: Exception) {
            Log.w(TAG, "saveCohortHistoryBlob failed: ${e.message}")
        }
    }

    /** Call once after nativeInit succeeds — restores history from prefs into Rust. */
    fun restoreCohortHistory() {
        if (!initialized) return
        try {
            val blob = sessionPrefs()?.getString(COHORT_HISTORY_PREF_KEY, null) ?: "[]"
            nativeLoadCohortHistory(blob)
        } catch (e: Exception) {
            Log.w(TAG, "restoreCohortHistory failed: ${e.message}")
        }
    }

    fun generateChannelQR(
        channel: Int,
        durationHours: Int = 24,
        groupName: String = "",
        cohortId: String? = null,
    ): String {
        if (!initialized) return ""
        return try {
            val json = nativeGenerateChannelQR(
                channel, durationHours, groupName.ifEmpty { null }, cohortId?.ifEmpty { null },
            )
            if (json.isNotEmpty()) {
                sessionPrefs()?.edit()?.putString("session_ch_$channel", json)?.apply()
                saveCohortHistoryBlob()
            }
            json
        } catch (e: Exception) {
            Log.e(TAG, "generateChannelQR failed: ${e.message}")
            ""
        }
    }

    fun getChannelInfo(): String {
        if (!initialized) return "[]"
        return try { nativeGetChannelInfo() } catch (e: Exception) { "[]" }
    }

    fun setGroupName(channel: Int, name: String) {
        if (initialized) try { nativeSetGroupName(channel, name) } catch (_: Exception) {}
    }

    fun getGroupName(channel: Int): String {
        if (!initialized) return "Channel $channel"
        return try { nativeGetGroupName(channel) } catch (_: Exception) { "Channel $channel" }
    }

}
