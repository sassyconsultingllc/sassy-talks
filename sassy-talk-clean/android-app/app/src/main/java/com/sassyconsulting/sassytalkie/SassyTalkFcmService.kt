package com.sassyconsulting.sassytalkie

import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.os.Build
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Receives wake-pushes from the relay. The relay fires a data-only,
 * priority=HIGH FCM message of the form:
 *
 *   data: { kind: "wake", room: "<room_id>", ts: "<senderMs>" }
 *
 * Our job:
 *   1. Bring WalkieService into the foreground if it isn't already (relay WS
 *      lives on that service).
 *   2. Tell the service to re-attempt cellular relay attach for the room.
 *   3. Persist the room hint so the service handles the wake even if it has
 *      to cold-start (rare, but FCM can land on a fully-killed process).
 *
 * Also handles FCM token rotation: on onNewToken, re-upload to /presence so
 * the relay's stored token stays fresh.
 */
class SassyTalkFcmService : FirebaseMessagingService() {

    companion object {
        private const val TAG = "SassyTalkFcm"
        // Encrypted store filename. Deliberately NOT the old plaintext name so
        // EncryptedSharedPreferences never tries to open a pre-existing
        // plaintext file of the same name (that throws).
        const val WAKE_PREFS = "sassy_wake_state_enc"
        private const val LEGACY_WAKE_PREFS = "sassy_wake_state"
        const val WAKE_KEY_LAST_TOKEN = "fcm_token"
        const val WAKE_KEY_LAST_ROOM = "last_wake_room"
        const val WAKE_KEY_LAST_TS = "last_wake_ts"
    }

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    override fun onDestroy() {
        super.onDestroy()
        // Don't leak the IO scope past the service lifecycle.
        scope.cancel()
    }

    /**
     * Keystore-backed prefs for the FCM token + wake hints. The token addresses
     * wake-pushes to this device, so it must not sit in a plaintext file
     * (readable on a rooted device). Returns null if Keystore is unavailable —
     * these are write-only hints, so skipping a write is harmless.
     */
    private fun wakePrefs(): SharedPreferences? = try {
        // One-time cleanup of any legacy plaintext file from older builds.
        applicationContext.deleteSharedPreferences(LEGACY_WAKE_PREFS)
        val masterKey = MasterKey.Builder(applicationContext)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        EncryptedSharedPreferences.create(
            applicationContext,
            WAKE_PREFS,
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    } catch (e: Exception) {
        Log.w(TAG, "Encrypted wake prefs unavailable; not persisting: ${e.message}")
        null
    }

    override fun onNewToken(token: String) {
        super.onNewToken(token)
        Log.i(TAG, "FCM token refreshed (len=${token.length})")
        wakePrefs()?.edit()
            ?.putString(WAKE_KEY_LAST_TOKEN, token)
            ?.apply()

        // Re-upload to /presence if we know the current room.
        val roomId = currentRoomId()
        if (!roomId.isNullOrBlank()) {
            scope.launch {
                PresenceClient.upload(applicationContext, roomId, token)
            }
        }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        super.onMessageReceived(message)
        val data = message.data
        val kind = data["kind"]
        if (kind != "wake") {
            Log.d(TAG, "FCM message kind=$kind — ignoring")
            return
        }
        val room = data["room"] ?: ""
        Log.i(TAG, "WAKE push received room=$room ts=${data["ts"]}")

        // Persist so a cold-started service can act on it.
        wakePrefs()?.edit()
            ?.putString(WAKE_KEY_LAST_ROOM, room)
            ?.putLong(WAKE_KEY_LAST_TS, System.currentTimeMillis())
            ?.apply()

        // Two-pronged: cold-start MainActivity (which wires PttCoordinator +
        // cellular client via the existing AppNavigation flow), and ALSO send
        // an ACTION_WAKE to the already-running service for the warm case.
        // High-priority FCM messages get a temporary background-launch
        // allowlist on Android 12+, which makes both starts legal.
        //
        // The Activity path is the only one that actually bootstraps the
        // walkie stack from a fully-killed process — startForegroundService
        // with foregroundServiceType=microphone is rejected on Android 14+
        // when launched from background without a UI foreground promotion.
        val activityIntent = Intent(applicationContext, MainActivity::class.java).apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        }
        try {
            applicationContext.startActivity(activityIntent)
        } catch (t: Throwable) {
            Log.w(TAG, "Failed to start MainActivity on wake: ${t.message}")
        }

        // Best-effort warm-path nudge — fine if the service isn't running yet,
        // the Activity launch above will start it. startService (not
        // startForegroundService) avoids the A14+ microphone-fg-type trap;
        // the service stays foregrounded via the existing notification if it
        // was already promoted, or stays plain-bound if newly bound.
        val serviceIntent = Intent(applicationContext, WalkieService::class.java).apply {
            action = WalkieService.ACTION_WAKE
            putExtra(WalkieService.EXTRA_ROOM, room)
        }
        try {
            applicationContext.startService(serviceIntent)
        } catch (t: Throwable) {
            // If the service isn't running and the OS rejects a plain
            // startService from background, the Activity launch above is the
            // real bootstrap path — silent skip is correct.
            Log.d(TAG, "warm-path startService skipped: ${t.message}")
        }
    }

    /** Best-effort: read the current active session id (room) from native. */
    private fun currentRoomId(): String? {
        return try {
            SassyTalkNative.appContext = applicationContext
            SassyTalkNative.getSessionId()
        } catch (t: Throwable) {
            null
        }
    }
}
