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

    /**
     * Pass an Android Context down to the native layer so audio routing
     * (MODE_IN_COMMUNICATION + speakerphone override on Moto/Xiaomi) can
     * obtain `AudioManager` via `getSystemService`. Idempotent — only the
     * first non-null Context sticks. Call from `WalkieService.onCreate`
     * once the foreground service is up.
     */
    fun initContext(context: android.content.Context): Boolean {
        return try {
            val ok = nativeInitContext(context.applicationContext)
            Log.i(TAG, "Native initContext: $ok")
            ok
        } catch (e: Exception) {
            Log.e(TAG, "initContext failed: ${e.message}")
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

    /**
     * Set the per-install unique id, mixed into the native sender identity so
     * two devices with the SAME display name never derive the same sender_id.
     * Without this, each side dropped the other's audio as its own echo and
     * never registered the peer (no audio + no roster row, toast still firing).
     * Call before transports connect (init path, alongside setDeviceName).
     */
    fun setInstallId(installId: String) {
        if (initialized && installId.isNotBlank()) {
            try {
                nativeSetInstallId(installId)
                Log.i(TAG, "Install id set (${installId.length} chars)")
            } catch (e: Exception) {
                Log.e(TAG, "setInstallId failed: ${e.message}")
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
                applySealedContext(qrJson)
            }
            ok
        } catch (e: Exception) {
            Log.e(TAG, "importSessionFromQR failed: ${e.message}")
            false
        }
    }

    /**
     * Populate the native sealed-sender context (32-byte session key + stable
     * per-install id) from a session QR/JSON so the relay's room/peer/device
     * params can be replaced by per-epoch blinded handles. The key is read from
     * the QR's "key" field and handed straight across JNI — it never lands in a
     * long-lived JVM field. No-op on a missing key/context. Blinding only takes
     * effect when the user enables Sealed Sender (setSealedSenderEnabled).
     */
    private fun applySealedContext(qrJson: String) {
        try {
            val keyB64 = org.json.JSONObject(qrJson).optString("key", "")
            val ctx = appContext
            if (keyB64.isNotEmpty() && ctx != null) {
                setSealedContext(keyB64, InstallId.get(ctx))
            }
        } catch (_: Throwable) { /* sealed context is best-effort defense-in-depth */ }
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
                    applySealedContext(json)
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

    fun removeUser(userId: String) {
        if (initialized) {
            try {
                nativeRemoveUser(userId)
            } catch (e: Exception) {
                Log.e(TAG, "removeUser failed: ${e.message}")
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
        // Capture the currently-active room id BEFORE we wipe the session,
        // so we can DELETE the /presence row that ties this install's FCM
        // token to that room. Otherwise the relay keeps firing wake pushes
        // to this token for the room we just left, until the FCM-side error
        // eventually evicts the row 30 days later.
        val activeRoom = try { getSessionId() } catch (_: Exception) { null }

        if (initialized) {
            try { nativeClearSession() } catch (e: Exception) {
                Log.e(TAG, "clearSession failed: ${e.message}")
            }
        }
        // Drop the sealed-sender context so a stale session key can't blind a
        // future room after this one is torn down.
        clearSealedContext()
        // Clear per-channel sessions and legacy session_json, but preserve cohort_history_v1.
        val prefs = sessionPrefs() ?: return
        val editor = prefs.edit()
        for (ch in 1..8) editor.remove("session_ch_$ch")
        editor.remove("session_json")
        editor.apply()

        // Fire-and-forget presence DELETE on a worker thread — must not block
        // the UI thread typically driving clearSession, and we don't want a
        // network hiccup to leave the local clear half-done.
        if (!activeRoom.isNullOrBlank()) {
            val ctx = appContext
            if (ctx != null) {
                Thread {
                    try { PresenceClient.remove(ctx, activeRoom) }
                    catch (t: Throwable) { Log.w(TAG, "presence DELETE on clearSession: ${t.message}") }
                }.apply { name = "presence-remove"; isDaemon = true }.start()
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

    // ── Hybrid post-quantum key exchange (path a: PSK-authenticated) ──
    //
    // The QR PSK authenticates the pairing; the ephemeral X25519 + ML-KEM-768
    // handshake adds forward secrecy + post-quantum protection. Negotiated via
    // the heartbeat capabilities bitmap ([localCapabilities] / CAP_HYBRID_PQC) —
    // only used when BOTH peers advertise support, else the classical path stands.

    /** This build's capability bitmap (heartbeat caps byte). Today: hybrid-PQC. */
    fun localCapabilities(): Int {
        if (!initialized) return 0
        return try { nativeLocalCapabilities() } catch (_: Exception) { 0 }
    }

    /**
     * Initiator: begin a hybrid handshake for [channel]. Returns the base64
     * initiator message to send to the peer, or null if the channel has no PSK.
     * Finish with [hybridHandshakeComplete] once the peer replies.
     */
    fun hybridHandshakeInit(channel: Int): String? {
        if (!initialized) return null
        return try {
            nativeHybridHandshakeInit(channel)?.ifEmpty { null }
        } catch (e: Exception) { Log.e(TAG, "hybridHandshakeInit failed: ${e.message}"); null }
    }

    /**
     * Responder: given the peer's base64 initiator message for [channel],
     * establish the session and return the base64 reply to send back (or null).
     */
    fun hybridHandshakeRespond(channel: Int, initB64: String): String? {
        if (!initialized) return null
        return try {
            nativeHybridHandshakeRespond(channel, initB64)?.ifEmpty { null }
        } catch (e: Exception) { Log.e(TAG, "hybridHandshakeRespond failed: ${e.message}"); null }
    }

    /** Initiator: complete with the peer's base64 reply, establishing the session. */
    fun hybridHandshakeComplete(respB64: String): Boolean {
        if (!initialized) return false
        return try {
            nativeHybridHandshakeComplete(respB64)
        } catch (e: Exception) { Log.e(TAG, "hybridHandshakeComplete failed: ${e.message}"); false }
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

    // ── Mic gain & squelch ──
    //
    // Gain is stored on the native side as `gain × 100` (so 100 = 1.0×).
    // Squelch is an integer dBFS threshold; 0 means disabled. Both default
    // to a no-op (gain 1.0, squelch off) so the existing user experience is
    // unchanged until the user opts into Settings adjustments.

    /** Set mic input gain (1.0 = no change). Clamped to [0.25, 4.0]. */
    fun setMicGain(gain: Float) {
        if (!initialized) return
        val x100 = (gain * 100f).toInt().coerceIn(25, 400)
        try { nativeSetMicGainX100(x100) } catch (e: Exception) {
            Log.e(TAG, "setMicGain failed: ${e.message}")
        }
    }

    /** Current mic input gain (1.0 = unity). */
    fun getMicGain(): Float {
        if (!initialized) return 1.0f
        return try { nativeGetMicGainX100() / 100f } catch (_: Exception) { 1.0f }
    }

    /** Set squelch threshold in dBFS. 0 disables squelch. Otherwise clamped to [-60, -10]. */
    fun setSquelchDbfs(dbfs: Int) {
        if (!initialized) return
        val clamped = if (dbfs == 0) 0 else dbfs.coerceIn(-60, -10)
        try { nativeSetSquelchDbfs(clamped) } catch (e: Exception) {
            Log.e(TAG, "setSquelchDbfs failed: ${e.message}")
        }
    }

    /** Current squelch threshold (0 = disabled). */
    fun getSquelchDbfs(): Int {
        if (!initialized) return 0
        return try { nativeGetSquelchDbfs() } catch (_: Exception) { 0 }
    }

    // ── Noise suppression (spectral-subtraction Wiener on the mic TX path) ──
    // Default OFF (zero cost when disabled). Toggle + aggressiveness exposed to
    // Settings; useful for trucks / wind / job sites.

    /** Enable/disable mic-path noise suppression. */
    fun setNoiseSuppressionEnabled(enabled: Boolean) {
        if (!initialized) return
        try { nativeSetNoiseSuppressionEnabled(enabled) } catch (e: Exception) {
            Log.e(TAG, "setNoiseSuppressionEnabled failed: ${e.message}")
        }
    }

    /** True if noise suppression is currently engaged. */
    fun getNoiseSuppressionEnabled(): Boolean {
        if (!initialized) return false
        return try { nativeGetNoiseSuppressionEnabled() } catch (_: Exception) { false }
    }

    /** Max attenuation in dB (aggressiveness). Clamped to [6, 30] on the Rust side. */
    fun setNoiseSuppressionAttenDb(db: Int) {
        if (!initialized) return
        try { nativeSetNoiseSuppressionAttenDb(db) } catch (e: Exception) {
            Log.e(TAG, "setNoiseSuppressionAttenDb failed: ${e.message}")
        }
    }

    // ── Sealed sender (metadata resistance) ──
    // When enabled with a context set, the cellular relay sees only per-epoch
    // blinded room/peer handles (no stable room id, no device name). Opt-in and
    // coordinated: every member must enable it and share the session key.

    /** Enable/disable sealed-sender connection blinding for the relay. */
    fun setSealedSenderEnabled(enabled: Boolean) {
        if (!initialized) return
        try { nativeSetSealedSenderEnabled(enabled) } catch (e: Exception) {
            Log.e(TAG, "setSealedSenderEnabled failed: ${e.message}")
        }
    }

    /**
     * Push the sealed context: the 32-byte session key (base64) + the stable
     * per-install peer id (see InstallId). Call after a session is imported.
     * Returns false if the key isn't a valid 32-byte base64 blob.
     */
    fun setSealedContext(sessionKeyB64: String, stablePeerId: String): Boolean {
        if (!initialized) return false
        return try { nativeSetSealedContext(sessionKeyB64, stablePeerId) } catch (e: Exception) {
            Log.e(TAG, "setSealedContext failed: ${e.message}")
            false
        }
    }

    /** Clear the sealed context (call on session clear). */
    fun clearSealedContext() {
        if (!initialized) return
        try { nativeClearSealedContext() } catch (_: Exception) {}
    }

    // ── v2.7.5: RX (playback) gain ──
    //
    // Multiplies decoded PCM samples on the receiver side before forwarding
    // them to AudioTrack. Independent of the system media volume slider.
    // Useful when peers are at low Opus loudness or speakerphone is off and
    // the user wants louder voice without raising the global volume.

    /** Set RX playback gain (1.0 = no change). Clamped to [0.25, 4.0]. */
    fun setRxGain(gain: Float) {
        if (!initialized) return
        val x100 = (gain * 100f).toInt().coerceIn(25, 400)
        try { nativeSetRxGainX100(x100) } catch (e: Exception) {
            Log.e(TAG, "setRxGain failed: ${e.message}")
        }
    }

    /** Current RX playback gain (1.0 = unity). */
    fun getRxGain(): Float {
        if (!initialized) return 1.0f
        return try { nativeGetRxGainX100() / 100f } catch (_: Exception) { 1.0f }
    }

    /**
     * Hard-mute RX playback (true half-duplex cut). Unlike [setRxGain] (which
     * floors at 0.25×), this fully silences incoming audio — used to cut RX
     * while the local user is transmitting so the remote stream can't feed back
     * into the hot mic. Preserves the user's configured gain (restored on unmute).
     */
    fun setRxMuted(muted: Boolean) {
        if (!initialized) return
        try { nativeSetRxMuted(muted) } catch (e: Exception) {
            Log.e(TAG, "setRxMuted failed: ${e.message}")
        }
    }

    // ── v2.7.5: Speakerphone vs earpiece routing ──

    /**
     * Force audio output to the loudspeaker (true) or the earpiece (false).
     * Engages Android's MODE_IN_COMMUNICATION + setSpeakerphoneOn under the
     * hood — see `audio_routing.rs`.
     * @return true if the routing was applied, false if AudioManager wasn't
     *         reachable (rare — usually means initContext wasn't called).
     */
    fun setSpeakerphone(on: Boolean): Boolean {
        if (!initialized) return false
        return try { nativeSetSpeakerphone(on) } catch (e: Exception) {
            Log.e(TAG, "setSpeakerphone failed: ${e.message}")
            false
        }
    }

    /** True if our COMM-mode override is currently engaged. */
    fun isCommModeActive(): Boolean {
        if (!initialized) return false
        return try { nativeIsCommModeActive() } catch (_: Exception) { false }
    }

    // ── v2.7.5: Live-mode jitter buffer size ──
    //
    // 3 = ~60ms (Low Latency)  · 5 = ~100ms (Balanced, default)  · 8 = ~160ms (Smooth)
    // Trades end-to-end PTT latency for jitter absorption on flaky links.

    fun setJitterPrebufferFrames(frames: Int) {
        if (!initialized) return
        try { nativeSetJitterPrebufferFrames(frames.coerceIn(2, 16)) } catch (e: Exception) {
            Log.e(TAG, "setJitterPrebufferFrames failed: ${e.message}")
        }
    }

    fun getJitterPrebufferFrames(): Int {
        if (!initialized) return 5
        return try { nativeGetJitterPrebufferFrames() } catch (_: Exception) { 5 }
    }

    /** Clear all cached audio, reset to Live mode */
    fun clearAudioCache() {
        if (initialized) {
            try { nativeClearAudioCache() } catch (e: Exception) {
                Log.e(TAG, "clearAudioCache failed: ${e.message}")
            }
        }
    }

    /**
     * Enable or disable client-side PCM mixing of 2..=6 concurrent speakers.
     * Disabled by default — overlap flips the cache to Queue (sequential
     * utterances, classic walkie-talkie). Enabled — overlap flips to Mix
     * (real-time summed audio with AGC + soft clip). Falls back to Queue
     * automatically above 6 simultaneous talkers.
     */
    fun setMixModeEnabled(enabled: Boolean) {
        if (!initialized) return
        try { nativeSetMixModeEnabled(enabled) } catch (e: Exception) {
            Log.e(TAG, "setMixModeEnabled failed: ${e.message}")
        }
    }

    /** Returns true if mix mode is currently engaged on the native side. */
    fun isMixModeEnabled(): Boolean {
        if (!initialized) return false
        return try { nativeIsMixModeEnabled() } catch (_: Exception) { false }
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
    @JvmStatic private external fun nativeInitContext(ctx: android.content.Context): Boolean
    @JvmStatic private external fun nativeShutdown()

    // PTT
    @JvmStatic private external fun nativePttStart()
    @JvmStatic private external fun nativePttStop()
    @JvmStatic private external fun nativeSetPttBufferMode(bufferMode: Boolean)
    @JvmStatic private external fun nativeGetPttBufferMode(): Boolean
    @JvmStatic private external fun nativeReplayById(utteranceId: Long): Boolean
    @JvmStatic private external fun nativeLastHistoryId(): Long
    @JvmStatic private external fun nativeSetChannel(channel: Byte)

    /**
     * Toggle whether the native RX thread invokes TranscriptionBridge.onAudioReceived
     * for every audio frame. Off by default — paying ~1.9KB short[] allocation +
     * JNI thread attach per 20ms frame is the difference between clean playback
     * and intermittent ~50-300ms underrun glitches when transcription is off.
     */
    @JvmStatic external fun nativeSetTranscriptionBridgeEnabled(enabled: Boolean)

    // Mic gain & squelch
    @JvmStatic private external fun nativeSetMicGainX100(gainX100: Int)
    @JvmStatic private external fun nativeGetMicGainX100(): Int
    @JvmStatic private external fun nativeSetSquelchDbfs(dbfs: Int)
    @JvmStatic private external fun nativeGetSquelchDbfs(): Int

    // Noise suppression
    @JvmStatic private external fun nativeSetNoiseSuppressionEnabled(enabled: Boolean)
    @JvmStatic private external fun nativeGetNoiseSuppressionEnabled(): Boolean
    @JvmStatic private external fun nativeSetNoiseSuppressionAttenDb(db: Int)

    // Sealed sender (metadata resistance)
    @JvmStatic private external fun nativeSetSealedSenderEnabled(enabled: Boolean)
    @JvmStatic private external fun nativeSetSealedContext(keyB64: String, peerId: String): Boolean
    @JvmStatic private external fun nativeClearSealedContext()

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
    @JvmStatic private external fun nativeRemoveUser(userId: String)

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
    @JvmStatic private external fun nativeLocalCapabilities(): Int
    @JvmStatic private external fun nativeHybridHandshakeInit(channel: Int): String?
    @JvmStatic private external fun nativeHybridHandshakeRespond(channel: Int, initB64: String): String?
    @JvmStatic private external fun nativeHybridHandshakeComplete(respB64: String): Boolean
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
    @JvmStatic private external fun nativeSetInstallId(installId: String)
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
    @JvmStatic private external fun nativeSetMixModeEnabled(enabled: Boolean)
    @JvmStatic private external fun nativeIsMixModeEnabled(): Boolean

    // v2.7.5 — RX gain, speakerphone, jitter buffer
    @JvmStatic private external fun nativeSetRxGainX100(x100: Int)
    @JvmStatic private external fun nativeGetRxGainX100(): Int
    @JvmStatic private external fun nativeSetRxMuted(muted: Boolean)
    @JvmStatic private external fun nativeSetSpeakerphone(on: Boolean): Boolean
    @JvmStatic private external fun nativeIsCommModeActive(): Boolean
    @JvmStatic private external fun nativeSetJitterPrebufferFrames(frames: Int)
    @JvmStatic private external fun nativeGetJitterPrebufferFrames(): Int

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
