package com.sassyconsulting.sassytalkie

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.lifecycle.Lifecycle
import com.sassyconsulting.sassytalkie.ui.ActivityEntry
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlin.math.sqrt

/**
 * Bridges the Rust native RX audio thread to the activity log and notifications.
 *
 * Rust calls [onAudioReceived] via JNI with decoded PCM frames from remote
 * speakers. This object performs energy-based voice activity detection (VAD)
 * to detect speech start/end, then emits [ActivityEntry] items (who spoke,
 * when, how long) for the Compose UI and fires notifications when backgrounded.
 */
object TranscriptionBridge {

    private const val TAG = "TranscriptionBridge"
    private const val ACTIVITY_CHANNEL_ID = "sassytalkie_activity"
    private const val ACTIVITY_NOTIFICATION_BASE_ID = 5000

    private var appContext: Context? = null
    private var notificationIdCounter = ACTIVITY_NOTIFICATION_BASE_ID

    /** RMS amplitude below which a frame is considered silence. */
    private const val SILENCE_THRESHOLD = 500

    /** Consecutive silent frames to finalize speech (400ms at 20ms/frame). */
    private const val SILENCE_FRAMES_TO_END = 20

    /** Maximum entries kept in the feed. */
    private const val MAX_ENTRIES = 200

    // ── State ──

    private val _entries = MutableStateFlow<List<ActivityEntry>>(emptyList())
    val entries: StateFlow<List<ActivityEntry>> = _entries.asStateFlow()

    /** Observable flag for UI: true when any remote speaker is actively talking. */
    private val _incomingAudio = MutableStateFlow(false)
    val incomingAudio: StateFlow<Boolean> = _incomingAudio.asStateFlow()

    /** Name of the currently active speaker (for UI indicator). */
    private val _activeSpeakerName = MutableStateFlow("")
    val activeSpeakerName: StateFlow<String> = _activeSpeakerName.asStateFlow()

    @Volatile
    private var initialized = false

    // ── Active speech tracking (guarded by [lock]) ──

    private val lock = Any()
    private var activeSenderId: String? = null
    private var activeSenderName: String? = null
    private var activeIsFavorite = false
    private var activeIsMuted = false
    private var speechStartTime = 0L
    private var silentFrameCount = 0
    private var inSpeech = false

    // ── Lifecycle ──

    fun initialize(context: Context) {
        if (initialized) return
        appContext = context.applicationContext
        createNotificationChannel(context.applicationContext)
        initialized = true
        Log.i(TAG, "Initialized")
    }

    private fun createNotificationChannel(context: Context) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                ACTIVITY_CHANNEL_ID,
                "Incoming Voice",
                NotificationManager.IMPORTANCE_HIGH
            ).apply {
                description = "Notifications when someone speaks while the app is in the background"
            }
            val nm = context.getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(channel)
        }
    }

    private fun isAppInForeground(): Boolean {
        return try {
            ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)
        } catch (e: Exception) {
            true
        }
    }

    // ── JNI entry point ──

    /**
     * Called from the Rust native RX thread with a chunk of decoded PCM audio.
     */
    @JvmStatic
    fun onAudioReceived(
        senderId: String,
        senderName: String,
        pcmSamples: ShortArray,
        isFavorite: Boolean,
        isMuted: Boolean,
    ) {
        if (!initialized) return

        val rms = computeRms(pcmSamples)
        val isSpeech = rms >= SILENCE_THRESHOLD

        synchronized(lock) {
            if (isSpeech) {
                handleSpeechFrame(senderId, senderName, isFavorite, isMuted)
            } else {
                handleSilenceFrame()
            }
        }
    }

    // ── User status ──

    fun updateUserStatus(senderId: String, isFavorite: Boolean, isMuted: Boolean) {
        val current = _entries.value
        val updated = current.map { entry ->
            if (entry.senderId == senderId) {
                entry.copy(isFavorite = isFavorite, isMuted = isMuted)
            } else {
                entry
            }
        }
        _entries.value = updated
    }

    @Volatile
    private var clearing = false

    fun clearEntries() {
        if (clearing) return
        clearing = true
        _entries.value = emptyList()
        synchronized(lock) { resetSpeechState() }
        try { SassyTalkNative.clearAudioCache() } catch (_: Exception) {}
        clearing = false
        Log.d(TAG, "Entries cleared")
    }

    fun release() {
        clearEntries()
        initialized = false
        Log.i(TAG, "Released")
    }

    // ── Internal helpers ──

    private fun handleSpeechFrame(
        senderId: String,
        senderName: String,
        isFavorite: Boolean,
        isMuted: Boolean,
    ) {
        silentFrameCount = 0

        if (!inSpeech) {
            inSpeech = true
            activeSenderId = senderId
            activeSenderName = senderName
            activeIsFavorite = isFavorite
            activeIsMuted = isMuted
            speechStartTime = System.currentTimeMillis()

            // Update incoming audio indicator
            _incomingAudio.value = true
            _activeSpeakerName.value = senderName
        }
    }

    private fun handleSilenceFrame() {
        if (!inSpeech) return

        silentFrameCount++
        if (silentFrameCount >= SILENCE_FRAMES_TO_END) {
            finalizeSpeechSegment()
        }
    }

    /** End the current speech segment — log who spoke and for how long. */
    private fun finalizeSpeechSegment() {
        val id = activeSenderId ?: return
        val name = activeSenderName ?: return
        val durationMs = System.currentTimeMillis() - speechStartTime
        val isFav = activeIsFavorite
        val isMuted = activeIsMuted
        val ts = speechStartTime

        resetSpeechState()

        // Update incoming audio indicator
        _incomingAudio.value = false
        _activeSpeakerName.value = ""

        if (durationMs < 200) return // skip sub-200ms blips

        // Get utterance ID for replay linkage
        val utteranceId = run {
            Thread.sleep(50)
            SassyTalkNative.lastHistoryId()
        }

        val durationSec = "%.1f".format(durationMs / 1000.0)

        val entry = ActivityEntry(
            senderId = id,
            senderName = name,
            durationText = "${durationSec}s",
            timestamp = ts,
            isFavorite = isFav,
            isMuted = isMuted,
            utteranceId = utteranceId,
        )
        addEntry(entry)
    }

    private fun addEntry(entry: ActivityEntry) {
        val current = _entries.value
        val updated = if (current.size >= MAX_ENTRIES) {
            current.drop(1) + entry
        } else {
            current + entry
        }
        _entries.value = updated

        // Fire notification if app is backgrounded
        showActivityNotification(entry)
    }

    private fun showActivityNotification(entry: ActivityEntry) {
        val ctx = appContext ?: return
        if (isAppInForeground()) return
        if (entry.isMuted) return

        val launchIntent = Intent(ctx, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra("open_activity", true)
        }
        val pendingIntent = PendingIntent.getActivity(
            ctx, 0, launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val timeStr = java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.getDefault())
            .format(java.util.Date(entry.timestamp))

        val notification = NotificationCompat.Builder(ctx, ACTIVITY_CHANNEL_ID)
            .setContentTitle("${entry.senderName} spoke")
            .setContentText("${entry.durationText} at $timeStr")
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setWhen(entry.timestamp)
            .setShowWhen(true)
            .build()

        try {
            val nm = ctx.getSystemService(NotificationManager::class.java)
            nm.notify(notificationIdCounter++, notification)
            if (notificationIdCounter > ACTIVITY_NOTIFICATION_BASE_ID + 50) {
                notificationIdCounter = ACTIVITY_NOTIFICATION_BASE_ID
            }
        } catch (e: Exception) {
            Log.w(TAG, "Failed to show activity notification: ${e.message}")
        }
    }

    private fun resetSpeechState() {
        inSpeech = false
        activeSenderId = null
        activeSenderName = null
        activeIsFavorite = false
        activeIsMuted = false
        speechStartTime = 0L
        silentFrameCount = 0
    }

    private fun computeRms(samples: ShortArray): Double {
        if (samples.isEmpty()) return 0.0
        var sumSquares = 0.0
        for (sample in samples) {
            val s = sample.toDouble()
            sumSquares += s * s
        }
        return sqrt(sumSquares / samples.size)
    }
}
