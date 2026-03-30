package com.sassyconsulting.sassytalkie

import android.util.Log
import com.sassyconsulting.sassytalkie.ui.TranscriptionEntry
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlin.math.sqrt

/**
 * Bridges the Rust native RX audio thread to live transcription via Whisper.
 *
 * Rust calls [onAudioReceived] via JNI with decoded PCM frames from remote
 * speakers. This object performs energy-based voice activity detection (VAD),
 * buffers PCM during speech, then runs on-device Whisper inference when
 * speech ends. Results are emitted as [TranscriptionEntry] items for the
 * Compose UI.
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

    /** Maximum buffered frames (10 seconds at 20ms/frame = 500 frames). */
    private const val MAX_BUFFER_FRAMES = 500

    // ── State ──

    private val _entries = MutableStateFlow<List<TranscriptionEntry>>(emptyList())

    /** Observable feed of transcription entries for Compose UI. */
    val entries: StateFlow<List<TranscriptionEntry>> = _entries.asStateFlow()

    @Volatile
    private var enabled = false

    @Volatile
    private var initialized = false

    @Volatile
    var whisperReady = false
        private set

    // Background scope for whisper inference (doesn't block RX thread)
    private val transcriptionScope = CoroutineScope(
        Dispatchers.Default + SupervisorJob()
    )

    // ── Active speech tracking (guarded by [lock]) ──

    private val lock = Any()
    private var activeSenderId: String? = null
    private var activeSenderName: String? = null
    private var activeIsFavorite = false
    private var activeIsMuted = false
    private var speechStartTime = 0L
    private var silentFrameCount = 0
    private var inSpeech = false

    /** PCM frame buffer — accumulated during speech for whisper inference. */
    private val pcmBuffer = mutableListOf<ShortArray>()

    // ── Lifecycle ──

    fun initialize(@Suppress("UNUSED_PARAMETER") context: android.content.Context) {
        if (initialized) return
        initialized = true
        Log.i(TAG, "Initialized")
    }

    /** Call after model download completes. */
    fun initWhisper(modelPath: String): Boolean {
        whisperReady = SassyTalkNative.nativeInitWhisper(modelPath)
        Log.i(TAG, "Whisper init: ready=$whisperReady path=$modelPath")
        return whisperReady
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
     * @param pcmSamples 16-bit mono PCM samples (typically one 20 ms frame at 48kHz)
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
                handleSpeechFrame(senderId, senderName, pcmSamples, isFavorite, isMuted)
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
        transcriptionScope.cancel()
        initialized = false
        enabled = false
        whisperReady = false
        Log.i(TAG, "Released")
    }

    // ── Internal helpers ──

    private fun handleSpeechFrame(
        senderId: String,
        senderName: String,
        pcmSamples: ShortArray,
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
            pcmBuffer.clear()
        }

        // Buffer PCM for whisper inference (cap at MAX_BUFFER_FRAMES)
        if (pcmBuffer.size < MAX_BUFFER_FRAMES) {
            pcmBuffer.add(pcmSamples.copyOf())
        }
    }

    private fun handleSilenceFrame() {
        if (!inSpeech) return

        silentFrameCount++
        if (silentFrameCount >= SILENCE_FRAMES_TO_END) {
            finalizeSpeechSegment()
        }
    }

    /** End the current speech segment and run whisper inference. */
    private fun finalizeSpeechSegment() {
        val id = activeSenderId ?: return
        val name = activeSenderName ?: return
        val durationMs = System.currentTimeMillis() - speechStartTime
        val isFav = activeIsFavorite
        val isMuted = activeIsMuted
        val ts = speechStartTime

        // Concatenate all buffered frames into one contiguous PCM array
        val totalSamples = pcmBuffer.sumOf { it.size }
        val fullPcm = ShortArray(totalSamples)
        var offset = 0
        for (frame in pcmBuffer) {
            frame.copyInto(fullPcm, offset)
            offset += frame.size
        }
        pcmBuffer.clear()
        resetSpeechState()

        if (totalSamples < 960) return // skip sub-20ms utterances

        if (whisperReady) {
            // Run whisper inference on background thread (1-3s on weak phones)
            transcriptionScope.launch {
                val text = try {
                    SassyTalkNative.nativeTranscribe48k(fullPcm)
                } catch (e: Exception) {
                    Log.e(TAG, "Whisper inference failed: ${e.message}")
                    ""
                }

                val displayText = text.trim().ifEmpty {
                    "[${name} spoke for ${durationMs}ms]"
                }

                val entry = TranscriptionEntry(
                    senderId = id,
                    senderName = name,
                    text = displayText,
                    timestamp = ts,
                    isFavorite = isFav,
                    isMuted = isMuted,
                )
                addEntry(entry)
            }
        } else {
            // Whisper not loaded — use placeholder
            val entry = TranscriptionEntry(
                senderId = id,
                senderName = name,
                text = "[${name} spoke for ${durationMs}ms]",
                timestamp = ts,
                isFavorite = isFav,
                isMuted = isMuted,
            )
            addEntry(entry)
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
