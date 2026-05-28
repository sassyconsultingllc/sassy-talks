package com.sassyconsulting.sassytalkie

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.widget.Toast
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
    /**
     * Audio duration of a single Opus frame, in ms. Must stay in sync with
     * the encoder's `frame_size` (currently 960 samples at 48 kHz = 20 ms).
     * Used by `finalizeSpeechSegment` to derive the "spoke for X" duration
     * from frame count instead of wall-clock — see the comment there for
     * why wall-clock produced phantom durations during network gaps.
     */
    private const val FRAME_DURATION_MS = 20L

    /** Maximum number of entries kept in the feed to bound memory usage. */
    private const val MAX_ENTRIES = 200

    // ── State ──

    private val _entries = MutableStateFlow<List<TranscriptionEntry>>(emptyList())

    /** Observable feed of timeline entries for Compose UI. */
    val entries: StateFlow<List<TranscriptionEntry>> = _entries.asStateFlow()

    private val _incomingAudio = MutableStateFlow(false)
    /** True while a remote speaker is actively speaking. */
    val incomingAudio: StateFlow<Boolean> = _incomingAudio.asStateFlow()

    private val _activeSpeakerName = MutableStateFlow("")
    /** Display name of the currently active remote speaker. */
    val activeSpeakerName: StateFlow<String> = _activeSpeakerName.asStateFlow()

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
        // Mirror the flag into Rust so the RX thread skips the per-frame JNI
        // dispatch + short[] allocation when the timeline feature is off.
        // Without this, the callback's allocation cost causes intermittent
        // AudioTrack underruns whether or not anyone is listening for it.
        try { SassyTalkNative.nativeSetTranscriptionBridgeEnabled(enabled) }
        catch (_: Throwable) { /* JNI may be unavailable in tests */ }
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

        // Hold `lock` ONLY for the state mutation; do any UI side-effects
        // outside so we don't deadlock the main thread on a re-entrant
        // TranscriptionBridge call (e.g. clearEntries while a Toast is
        // pending). `startedSegment` is the signal.
        var startedSegment = false
        synchronized(lock) {
            if (isSpeech) {
                startedSegment = handleSpeechFrame(senderId, senderName, isFavorite, isMuted)
            } else {
                handleSilenceFrame()
            }
        }
        if (startedSegment) {
            onSpeechSegmentStarted(senderName, isMuted)
        }
    }

    /**
     * Called from the Rust RX thread the moment an utterance is committed
     * to the audio cache history. Replaces the previous best-effort
     * `lastHistoryId()` poll which raced against the 800 ms cache commit
     * timer and silently captured the wrong (or no) ID — the root cause
     * of "timeline play button does nothing."
     *
     * Behavior:
     *  - If a timeline entry for this sender already exists with
     *    `utteranceId == -1L` (Kotlin VAD finalized first), patch it in
     *    place so the replay button wires up correctly.
     *  - Otherwise stash the ID in `latestCommittedId` so the next
     *    `finalizeSpeechSegment` call for this sender can pick it up.
     */
    @JvmStatic
    fun onUtteranceCommitted(
        senderId: String,
        senderName: String,
        utteranceId: Long,
        durationMs: Long,
    ) {
        if (utteranceId < 0) return

        // All `_entries.value` mutations MUST happen under `lock`. Previously
        // this path mutated the StateFlow from the RX thread without holding
        // `lock`, while `addEntry` (from finalizeSpeechSegment) held it.
        // Concurrent read-modify-write on the backing list was possible.
        var patchedAny = false
        synchronized(lock) {
            val current = _entries.value
            val patched = current.toMutableList()
            // Patch the OLDEST un-tagged entry for this sender, not the most
            // recent. Previously the reversed-scan broke on the first match,
            // so two rapid un-tagged entries for the same sender would leave
            // the older one permanently stuck at utteranceId=-1. Now we walk
            // oldest-first AND only patch one — Rust commits arrive in the
            // same order they were finalized, so the oldest -1 entry is the
            // one that this commit corresponds to.
            for (i in patched.indices) {
                val e = patched[i]
                if (e.senderId == senderId && e.utteranceId < 0) {
                    patched[i] = e.copy(utteranceId = utteranceId)
                    patchedAny = true
                    break
                }
            }
            if (patchedAny) _entries.value = patched
        }
        if (patchedAny) {
            Log.d(TAG, "onUtteranceCommitted: patched existing entry sender=$senderId id=$utteranceId")
            return
        }

        // Otherwise stash so the next finalize for this sender picks it up.
        synchronized(latestCommittedLock) {
            latestCommittedId[senderId] = utteranceId
        }
        Log.d(TAG, "onUtteranceCommitted: stashed sender=$senderId id=$utteranceId (duration=${durationMs}ms)")
    }

    /** Most recently Rust-committed utterance ID per sender. Drained by `finalizeSpeechSegment`. */
    private val latestCommittedId = HashMap<String, Long>()
    private val latestCommittedLock = Any()

    // ── User status ──

    fun updateUserStatus(senderId: String, isFavorite: Boolean, isMuted: Boolean) {
        // Hold `lock` for the read-modify-write. Without this, concurrent
        // onUtteranceCommitted patches could be overwritten.
        synchronized(lock) {
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
    }

    private val clearing = java.util.concurrent.atomic.AtomicBoolean(false)

    fun clearEntries() {
        // CAS instead of check-then-act. The previous @Volatile + read+write
        // pattern was a TOCTOU — two simultaneous callers could both pass
        // the guard and both run the clear. CAS gives exactly-once semantics.
        if (!clearing.compareAndSet(false, true)) return
        try {
            synchronized(lock) {
                _entries.value = emptyList()
                resetSpeechState()
            }
            try { SassyTalkNative.clearAudioCache() } catch (_: Exception) {}
            Log.d(TAG, "Entries cleared")
        } finally {
            clearing.set(false)
        }
    }

    fun release() {
        clearEntries()
        initialized = false
        enabled = false
        Log.i(TAG, "Released")
    }

    // ── Internal helpers ──

    /**
     * Called from inside `synchronized(lock)` in `onAudioReceived`.
     * Returns `true` if this frame STARTED a new speech segment — caller
     * should perform any side-effects (Toast notification) AFTER releasing
     * the lock, to avoid blocking the main thread on `lock` if it tries to
     * call back into TranscriptionBridge while we hold it.
     */
    private fun handleSpeechFrame(
        senderId: String,
        senderName: String,
        isFavorite: Boolean,
        isMuted: Boolean,
    ): Boolean {
        silentFrameCount = 0

        if (!inSpeech) {
            inSpeech = true
            activeSenderId = senderId
            activeSenderName = senderName
            activeIsFavorite = isFavorite
            activeIsMuted = isMuted
            speechStartTime = System.currentTimeMillis()
            speechFrameCount = 0
            _incomingAudio.value = true
            _activeSpeakerName.value = senderName
            speechFrameCount++
            return true  // signal "new segment started, surface UX outside lock"
        }

        speechFrameCount++
        return false
    }

    /**
     * Side-effects deferred from `handleSpeechFrame`. Called from
     * `onAudioReceived` AFTER the `synchronized(lock)` block exits.
     * Posts the speaker-name Toast to the main thread; main never has
     * to wait on `lock` to do it.
     */
    private fun onSpeechSegmentStarted(senderName: String, isMuted: Boolean) {
        val ctx = appContext
        if (ctx == null || isMuted) return
        Handler(Looper.getMainLooper()).post {
            Toast.makeText(
                ctx,
                "$senderName is speaking on sassy-talk",
                Toast.LENGTH_SHORT
            ).show()
        }
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
        val isFav = activeIsFavorite
        val isMuted = activeIsMuted
        val ts = speechStartTime
        val frames = speechFrameCount
        // Duration MUST be derived from frame count, not wall-clock.
        //
        // The wall-clock version (`currentTimeMillis() - speechStartTime`)
        // produced phantom durations whenever a sender kept transmitting
        // through a network gap: receiver's VAD silence-detector finalised
        // mid-press, the wall-clock had accumulated, and the timeline showed
        // "spoke for 31s" / "spoke for 3 minutes" while the sender never
        // actually paused. Frame count is the ground truth — each frame is
        // exactly FRAME_DURATION_MS of audio the receiver actually played
        // (or buffered for play). Dropped frames are correctly EXCLUDED.
        val durationMs = (frames.toLong() * FRAME_DURATION_MS)

        resetSpeechState()

        if (frames < 2) return // skip sub-40ms blips

        // Get utterance ID for replay linkage. Single source of truth:
        // the Rust commit callback (`onUtteranceCommitted`). Two paths,
        // ONE outcome — no race, no wrong-sender attribution:
        //
        //   - If commit fired BEFORE this finalize: utteranceId is in
        //     the per-sender stash, we consume it here.
        //   - If commit fires AFTER this finalize: leave the entry's
        //     utteranceId = -1; `onUtteranceCommitted` will patch it
        //     in place the moment Rust dispatches the commit.
        //
        // The previous fallback to `SassyTalkNative.lastHistoryId()` was
        // a wrong-sender hazard: that returns the GLOBAL most-recent ID,
        // not per-sender. If Bob's utterance committed just before
        // Alice's finalize, Alice's entry could be tagged with Bob's
        // utteranceId and tapping replay would play Bob's audio.
        // Removed deliberately.
        val utteranceId = synchronized(latestCommittedLock) {
            latestCommittedId.remove(id)
        } ?: -1L

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
        _incomingAudio.value = false
        _activeSpeakerName.value = ""
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
