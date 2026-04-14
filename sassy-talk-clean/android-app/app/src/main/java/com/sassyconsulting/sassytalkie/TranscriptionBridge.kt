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
import com.sassyconsulting.sassytalkie.ui.TranscriptionEntry
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlin.math.sqrt

/**
 * Speech Timeline Bridge — records WHO spoke and for HOW LONG.
 *
 * Rust calls [onAudioReceived] via JNI with decoded PCM frames from remote
 * speakers. This object performs energy-based voice activity detection (VAD)
 * to detect speech start/end, then records a timeline entry with the speaker
 * name, timestamp, and duration. No transcription or Whisper inference.
 *
 * Results are emitted as [TranscriptionEntry] items for the Compose UI
 * (the data class is reused; the `text` field now contains duration info).
 */
object TranscriptionBridge {

    private const val TAG = "TranscriptionBridge"
    private const val TIMELINE_CHANNEL_ID = "sassytalkie_timeline"
    private const val TIMELINE_NOTIFICATION_BASE_ID = 5000

    private var appContext: Context? = null
    private var notificationIdCounter = TIMELINE_NOTIFICATION_BASE_ID

    /** RMS amplitude below which a frame is considered silence. */
    private const val SILENCE_THRESHOLD = 500

    /**
     * Number of consecutive silent frames required to finalize a speech segment.
     * At 20 ms per frame this equals 800 ms of silence — matches the Rust
     * AudioCache SPEECH_GAP_MS for consistent utterance boundaries.
     */
    private const val SILENCE_FRAMES_TO_END = 40

    /** Maximum number of entries kept in the feed to bound memory usage. */
    private const val MAX_ENTRIES = 200

    // ── State ──

    private val _entries = MutableStateFlow<List<TranscriptionEntry>>(emptyList())

    /** Observable feed of timeline entries for Compose UI. */
    val entries: StateFlow<List<TranscriptionEntry>> = _entries.asStateFlow()

    @Volatile
    private var enabled = false

    @Volatile
    private var initialized = false

    // Whisper fields removed — timeline only
    @Volatile
    var whisperReady = false  // kept for API compat, always false

    // ── Active speech tracking (guarded by [lock]) ──

    private val lock = Any()
    private var activeSenderId: String? = null
    private var activeSenderName: String? = null
    private var activeIsFavorite = false
    private var activeIsMuted = false
    private var speechStartTime = 0L
    private var silentFrameCount = 0
    private var inSpeech = false
    private var speechFrameCount = 0

    // ── Lifecycle ──

    fun initialize(context: Context) {
        if (initialized) return
        appContext = context.applicationContext
        createNotificationChannel(context.applicationContext)
        initialized = true
        Log.i(TAG, "Initialized (timeline mode)")
    }

    private fun createNotificationChannel(context: Context) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                TIMELINE_CHANNEL_ID,
                "Speech Timeline",
                NotificationManager.IMPORTANCE_DEFAULT
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

    private fun showTimelineNotification(entry: TranscriptionEntry) {
        val ctx = appContext ?: return
        if (isAppInForeground()) return
        if (entry.isMuted) return

        val launchIntent = Intent(ctx, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra("open_transcription", true)
        }
        val pendingIntent = PendingIntent.getActivity(
            ctx, 0, launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val notification = NotificationCompat.Builder(ctx, TIMELINE_CHANNEL_ID)
            .setContentTitle(entry.senderName)
            .setContentText(entry.text)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .build()

        try {
            val nm = ctx.getSystemService(NotificationManager::class.java)
            nm.notify(notificationIdCounter++, notification)
            if (notificationIdCounter > TIMELINE_NOTIFICATION_BASE_ID + 50) {
                notificationIdCounter = TIMELINE_NOTIFICATION_BASE_ID
            }
        } catch (e: Exception) {
            Log.w(TAG, "Failed to show timeline notification: ${e.message}")
        }
    }

    /** No-op — Whisper removed. Kept for API compatibility. */
    fun initWhisper(modelPath: String): Boolean {
        Log.i(TAG, "Whisper disabled — timeline mode only")
        whisperReady = false
        return false
    }

    fun setEnabled(enabled: Boolean) {
        this.enabled = enabled
        Log.d(TAG, "Timeline enabled=$enabled")
    }

    fun isEnabled(): Boolean = enabled

    // ── JNI entry point ──

    /**
     * Called from the Rust native RX thread with a chunk of decoded PCM audio.
     *
     * IMPORTANT: This method must return quickly — it runs on the audio RX thread.
     * No blocking calls, no Thread.sleep, no heavy computation.
     */
    @JvmStatic
    fun onAudioReceived(
        senderId: String,
        senderName: String,
        pcmSamples: ShortArray,
        isFavorite: Boolean,
        isMuted: Boolean,
    ) {
        if (!enabled || !initialized) return

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
        enabled = false
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
            // New speech segment begins
            inSpeech = true
            activeSenderId = senderId
            activeSenderName = senderName
            activeIsFavorite = isFavorite
            activeIsMuted = isMuted
            speechStartTime = System.currentTimeMillis()
            speechFrameCount = 0
        }

        speechFrameCount++
    }

    private fun handleSilenceFrame() {
        if (!inSpeech) return

        silentFrameCount++
        if (silentFrameCount >= SILENCE_FRAMES_TO_END) {
            finalizeSpeechSegment()
        }
    }

    /** End the current speech segment — record timeline entry (no Whisper). */
    private fun finalizeSpeechSegment() {
        val id = activeSenderId ?: return
        val name = activeSenderName ?: return
        val durationMs = System.currentTimeMillis() - speechStartTime
        val isFav = activeIsFavorite
        val isMuted = activeIsMuted
        val ts = speechStartTime
        val frames = speechFrameCount

        resetSpeechState()

        if (frames < 2) return // skip sub-40ms blips

        // Get utterance ID for replay linkage — non-blocking, no sleep needed
        val utteranceId = try {
            SassyTalkNative.lastHistoryId()
        } catch (_: Exception) {
            -1L
        }

        // Format duration for display
        val durationText = formatDuration(durationMs)
        val entry = TranscriptionEntry(
            senderId = id,
            senderName = name,
            text = "spoke for $durationText",
            timestamp = ts,
            isFavorite = isFav,
            isMuted = isMuted,
            utteranceId = utteranceId,
        )
        addEntry(entry)
    }

    private fun formatDuration(ms: Long): String {
        return when {
            ms < 1000 -> "${ms}ms"
            ms < 60_000 -> {
                val seconds = ms / 1000
                val remainder = (ms % 1000) / 100
                if (remainder > 0) "${seconds}.${remainder}s" else "${seconds}s"
            }
            else -> {
                val minutes = ms / 60_000
                val seconds = (ms % 60_000) / 1000
                "${minutes}m ${seconds}s"
            }
        }
    }

    private fun addEntry(entry: TranscriptionEntry) {
        val current = _entries.value
        val updated = if (current.size >= MAX_ENTRIES) {
            current.drop(1) + entry
        } else {
            current + entry
        }
        _entries.value = updated

        showTimelineNotification(entry)
    }

    private fun resetSpeechState() {
        inSpeech = false
        activeSenderId = null
        activeSenderName = null
        activeIsFavorite = false
        activeIsMuted = false
        speechStartTime = 0L
        silentFrameCount = 0
        speechFrameCount = 0
    }

    /** Compute root-mean-square amplitude for a PCM sample buffer. */
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
