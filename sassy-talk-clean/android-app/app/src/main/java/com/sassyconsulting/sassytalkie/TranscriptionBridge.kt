package com.sassyconsulting.sassytalkie

import android.util.Log
import com.sassyconsulting.sassytalkie.ui.TranscriptionEntry
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlin.math.sqrt

/**
 * Bridges the Rust native RX audio thread to live transcription.
 *
 * Rust calls [onAudioReceived] via JNI with decoded PCM frames from remote
 * speakers. This object performs energy-based voice activity detection (VAD)
 * to segment speech, then emits [TranscriptionEntry] items for the Compose UI.
 *
 * TODO: Integrate a real STT engine (Vosk / Whisper) to replace placeholder text.
 */
object TranscriptionBridge {

    private const val TAG = "TranscriptionBridge"

    /** RMS amplitude below which a frame is considered silence. */
    private const val SILENCE_THRESHOLD = 500

    /**
     * Number of consecutive silent frames required to finalize a speech segment.
     * At 20 ms per frame this equals 400 ms of silence.
     */
    private const val SILENCE_FRAMES_TO_END = 20

    /** Maximum number of entries kept in the feed to bound memory usage. */
    private const val MAX_ENTRIES = 200

    // ── State ──

    private val _entries = MutableStateFlow<List<TranscriptionEntry>>(emptyList())

    /** Observable feed of transcription entries for Compose UI. */
    val entries: StateFlow<List<TranscriptionEntry>> = _entries.asStateFlow()

    @Volatile
    private var enabled = false

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

    fun initialize(@Suppress("UNUSED_PARAMETER") context: android.content.Context) {
        if (initialized) return
        initialized = true
        Log.i(TAG, "Initialized")
    }

    fun setEnabled(enabled: Boolean) {
        this.enabled = enabled
        Log.d(TAG, "Transcription enabled=$enabled")
    }

    fun isEnabled(): Boolean = enabled

    // ── JNI entry point ──

    /**
     * Called from the Rust native RX thread with a chunk of decoded PCM audio.
     *
     * @param senderId   unique identifier of the remote speaker
     * @param senderName human-readable display name
     * @param pcmSamples 16-bit mono PCM samples (typically one 20 ms frame)
     * @param isFavorite whether this sender is marked as a favorite
     * @param isMuted    whether this sender is muted
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

    /** Update favorite/muted flags for an existing sender across all entries. */
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

    fun clearEntries() {
        _entries.value = emptyList()
        synchronized(lock) { resetSpeechState() }
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
        }
    }

    private fun handleSilenceFrame() {
        if (!inSpeech) return

        silentFrameCount++
        if (silentFrameCount >= SILENCE_FRAMES_TO_END) {
            finalizeSpeechSegment()
        }
    }

    /** End the current speech segment and emit a [TranscriptionEntry]. */
    private fun finalizeSpeechSegment() {
        val id = activeSenderId ?: return
        val name = activeSenderName ?: return
        val durationMs = System.currentTimeMillis() - speechStartTime

        // TODO: Replace placeholder with real STT result (Vosk / Whisper)
        val entry = TranscriptionEntry(
            senderId = id,
            senderName = name,
            text = "[${name} spoke for ${durationMs}ms]",
            timestamp = speechStartTime,
            isFavorite = activeIsFavorite,
            isMuted = activeIsMuted,
        )

        addEntry(entry)
        resetSpeechState()
    }

    private fun addEntry(entry: TranscriptionEntry) {
        val current = _entries.value
        val updated = if (current.size >= MAX_ENTRIES) {
            current.drop(1) + entry
        } else {
            current + entry
        }
        _entries.value = updated
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
