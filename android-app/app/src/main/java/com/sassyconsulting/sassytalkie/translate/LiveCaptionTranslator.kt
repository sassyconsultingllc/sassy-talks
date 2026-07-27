// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-D63U4X2SONQN
package com.sassyconsulting.sassytalkie.translate

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.util.Log
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.util.Locale

/**
 * Offline live-caption + translation of the LOCAL user's speech.
 *
 * Pipeline:  mic → Android [SpeechRecognizer] (on-device) → [TranslationManager]
 *
 * Speech-to-text uses the platform [SpeechRecognizer] with
 * [RecognizerIntent.EXTRA_PREFER_OFFLINE] = true so recognition runs on-device
 * (no audio leaves the phone) on devices that ship an offline recognition
 * model. Recognized partials + finals are piped through ML Kit translation and
 * exposed as [StateFlow]s for the Compose UI.
 *
 * ── Mic-contention caveat ───────────────────────────────────────────────
 * SpeechRecognizer opens its OWN AudioRecord session on the mic. The PTT
 * pipeline (PttAudioPipeline / WalkieService) ALSO uses the mic. Two
 * simultaneous capture sessions contend and on many devices one will fail
 * (ERROR_RECOGNIZER_BUSY / ERROR_AUDIO / silent capture). This class does NOT
 * try to share the mic with PTT. Intended usage:
 *
 *   • Run captioning when NOT actively PTT-transmitting (e.g. to caption what
 *     you're about to say, or while listening), OR
 *   • Let the caller gate start/stop around PTT press/release.
 *
 * start()/stop() are therefore EXPLICIT and idempotent. Prefer driving this
 * through [LiveTranslationBridge], which pauses around PTT and only runs while
 * a UI consumer (Main / Settings) is visible. SpeechRecognizer is mic-only: it
 * cannot transcribe the decoded PCM of REMOTE speakers.
 * ────────────────────────────────────────────────────────────────────────
 *
 * All recognition callbacks arrive on the main thread; translation is
 * dispatched to a background coroutine scope. Defensive throughout — never
 * crashes if the recognizer or permission is unavailable.
 */
class LiveCaptionTranslator(
    private val appContext: Context,
    private val translationManager: TranslationManager,
) {
    companion object {
        private const val TAG = "LiveCaptionTranslator"
        // Restart backoff. A session ends on every pause; in a silent room
        // ERROR_NO_MATCH / ERROR_SPEECH_TIMEOUT fire every few hundred ms, so a
        // zero-delay restart became an unbounded storm holding the mic. Back off
        // on consecutive empty results and give up after MAX_CONSECUTIVE_EMPTY.
        private const val RESTART_BASE_MS = 400L
        private const val RESTART_MAX_MS = 5_000L
        private const val MIC_BUSY_RETRY_MS = 700L
        private const val MAX_CONSECUTIVE_EMPTY = 8
    }

    /** Recognition / pipeline status for the UI. */
    enum class Status { IDLE, LISTENING, ERROR, UNAVAILABLE }

    private val _caption = MutableStateFlow("")
    /** Most recent recognized text (partial while speaking, final on pause). */
    val caption: StateFlow<String> = _caption.asStateFlow()

    private val _translation = MutableStateFlow("")
    /** Translation of [caption] into the current target language. */
    val translation: StateFlow<String> = _translation.asStateFlow()

    private val _status = MutableStateFlow(Status.IDLE)
    val status: StateFlow<Status> = _status.asStateFlow()

    private val _errorMessage = MutableStateFlow<String?>(null)
    /** Last human-readable error (null when none). */
    val errorMessage: StateFlow<String?> = _errorMessage.asStateFlow()

    // Background scope for translation work — recognition callbacks stay light.
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    // SpeechRecognizer must be created + driven on the main thread.
    private var recognizer: SpeechRecognizer? = null

    // Main-thread scheduler for backed-off restarts (recognizer is main-thread).
    private val mainHandler = Handler(Looper.getMainLooper())
    private var pendingRestart: Runnable? = null
    @Volatile private var consecutiveEmpty = 0

    /**
     * Optional gate the owner can set to report when PTT currently owns the mic
     * (e.g. `{ SassyTalkNative.isPttActive() }`). When it returns true, the
     * translator defers its restart instead of fighting PTT for the mic.
     */
    var onMicBusy: (() -> Boolean)? = null

    @Volatile private var running = false
    @Volatile private var srcLang = TranslationLangDefaults.DEFAULT_SOURCE
    @Volatile private var dstLang = TranslationLangDefaults.DEFAULT_TARGET
    @Volatile private var requireWifiForModels = true

    // Coalesce overlapping translation jobs so a fast stream of partials
    // doesn't pile up — only the latest in-flight translate matters.
    private var translateJob: Job? = null

    /**
     * Fired after a FINAL recognition result has been translated (caption +
     * translation). Used by [LiveTranslationBridge] for Timeline + TTS.
     */
    var onFinalUtterance: ((caption: String, translation: String) -> Unit)? = null

    /**
     * Fired when the recognizer gives up after too many empty/silent sessions.
     * The bridge should schedule a later restart so quiet radio standby doesn't
     * leave captioning permanently dead.
     */
    var onGaveUpListening: (() -> Unit)? = null

    /** True if this device exposes a usable on-device speech recognizer. */
    fun isRecognitionAvailable(): Boolean =
        try { SpeechRecognizer.isRecognitionAvailable(appContext) } catch (_: Throwable) { false }

    private fun hasMicPermission(): Boolean =
        ContextCompat.checkSelfPermission(appContext, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    /** Update the translation source/target. Safe to call while running. */
    fun setLanguages(src: String, dst: String) {
        srcLang = src
        dstLang = dst
        // Re-translate the current caption against the new target immediately.
        val current = _caption.value
        if (current.isNotBlank()) translateAsync(current)
    }

    /** Wi-Fi-only model downloads (default true). */
    fun setRequireWifi(requireWifi: Boolean) { requireWifiForModels = requireWifi }

    /**
     * Begin offline recognition. Idempotent — a second call while running is a
     * no-op. MUST be invoked from the main thread (SpeechRecognizer requirement).
     * Caller must ensure PTT is not actively transmitting (mic contention).
     */
    fun start() {
        if (running) return
        if (!hasMicPermission()) {
            Log.w(TAG, "start() denied: RECORD_AUDIO not granted")
            _status.value = Status.UNAVAILABLE
            _errorMessage.value = "Microphone permission required"
            return
        }
        if (!isRecognitionAvailable()) {
            Log.w(TAG, "start() denied: no speech recognizer on this device")
            _status.value = Status.UNAVAILABLE
            _errorMessage.value = "On-device speech recognition unavailable"
            return
        }

        try {
            val rec = recognizer ?: SpeechRecognizer.createSpeechRecognizer(appContext).also {
                it.setRecognitionListener(listener)
                recognizer = it
            }
            running = true
            consecutiveEmpty = 0
            _errorMessage.value = null
            _status.value = Status.LISTENING
            rec.startListening(buildRecognizerIntent())
        } catch (t: Throwable) {
            Log.e(TAG, "start() failed: ${t.message}", t)
            running = false
            _status.value = Status.ERROR
            _errorMessage.value = "Could not start recognition"
        }
    }

    /** Stop recognition and release the recognizer. Idempotent. Main thread. */
    fun stop() {
        running = false
        consecutiveEmpty = 0
        pendingRestart?.let { mainHandler.removeCallbacks(it) }
        pendingRestart = null
        translateJob?.cancel()
        try {
            recognizer?.stopListening()
            recognizer?.cancel()
            recognizer?.destroy()
        } catch (t: Throwable) {
            Log.w(TAG, "stop() cleanup error: ${t.message}")
        } finally {
            recognizer = null
            if (_status.value == Status.LISTENING) _status.value = Status.IDLE
        }
    }

    /** Full teardown including the translation scope. Call when disposing. */
    fun release() {
        stop()
        scope.coroutineContext[Job]?.cancel()
    }

    private fun buildRecognizerIntent(): Intent =
        Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(
                RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                RecognizerIntent.LANGUAGE_MODEL_FREE_FORM,
            )
            // Recognition language = the SOURCE (what the user speaks). ML Kit
            // codes ("en") map cleanly onto recognizer BCP-47 tags; pass a full
            // tag where we can.
            putExtra(RecognizerIntent.EXTRA_LANGUAGE, recognizerTag(srcLang))
            // Keep emitting partial hypotheses for a live-caption feel.
            putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            // The privacy-critical flag: force on-device recognition. On devices
            // without an offline model this yields ERROR_LANGUAGE_UNAVAILABLE,
            // which we surface rather than silently falling back to the cloud.
            putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
            putExtra(RecognizerIntent.EXTRA_CALLING_PACKAGE, appContext.packageName)
        }

    /** Map an ML Kit language code to a recognizer locale tag (best effort). */
    private fun recognizerTag(code: String): String =
        try { Locale.forLanguageTag(code).toLanguageTag() } catch (_: Throwable) { code }

    /** Fire-and-forget translation of [text], cancelling any prior in-flight job. */
    private fun translateAsync(text: String, isFinal: Boolean = false) {
        translateJob?.cancel()
        translateJob = scope.launch {
            val out = translationManager.translate(
                text = text,
                src = srcLang,
                dst = dstLang,
                requireWifi = requireWifiForModels,
            )
            _translation.value = out
            if (isFinal) {
                try {
                    onFinalUtterance?.invoke(text, out)
                } catch (t: Throwable) {
                    Log.w(TAG, "onFinalUtterance failed: ${t.message}")
                }
            }
        }
    }

    private val listener = object : RecognitionListener {
        override fun onReadyForSpeech(params: Bundle?) {
            _errorMessage.value = null
            _status.value = Status.LISTENING
        }

        override fun onPartialResults(partialResults: Bundle?) {
            val text = firstResult(partialResults) ?: return
            if (text.isBlank()) return
            _caption.value = text
            translateAsync(text)
        }

        override fun onResults(results: Bundle?) {
            val text = firstResult(results)
            if (!text.isNullOrBlank()) {
                _caption.value = text
                translateAsync(text, isFinal = true)
                // Real speech recognized — restart promptly, reset backoff.
                scheduleRestart(backoff = false)
            } else {
                // Session ended with nothing — treat as an empty result so the
                // restart backs off instead of spinning.
                scheduleRestart(backoff = true)
            }
        }

        override fun onError(error: Int) {
            val msg = errorText(error)
            Log.w(TAG, "recognizer error: $msg ($error)")
            when (error) {
                // Transient no-speech: restart with backoff (silent-room guard).
                SpeechRecognizer.ERROR_NO_MATCH,
                SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> scheduleRestart(backoff = true)
                // Mic busy / flaky client — almost always PTT contention or a
                // brief AudioRecord race. Keep trying with backoff instead of
                // sticking in ERROR until the user toggles translation off/on.
                SpeechRecognizer.ERROR_RECOGNIZER_BUSY,
                SpeechRecognizer.ERROR_AUDIO,
                SpeechRecognizer.ERROR_CLIENT -> {
                    _errorMessage.value = msg
                    scheduleRestart(backoff = true)
                }
                // Offline pack / permission — surface and stop; Settings deep-link
                // covers language packs. Health watchdog ignores UNAVAILABLE.
                SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED,
                SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE,
                SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> {
                    running = false
                    _errorMessage.value = msg
                    _status.value = Status.UNAVAILABLE
                }
                else -> {
                    _errorMessage.value = msg
                    scheduleRestart(backoff = true)
                }
            }
        }

        // Unused callbacks — keep them no-op but defined (interface requires all).
        override fun onBeginningOfSpeech() {}
        override fun onRmsChanged(rmsdB: Float) {}
        override fun onBufferReceived(buffer: ByteArray?) {}
        override fun onEndOfSpeech() {}
        override fun onEvent(eventType: Int, params: Bundle?) {}
    }

    private fun firstResult(bundle: Bundle?): String? =
        bundle?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()

    /**
     * Schedule a fresh recognition session with backoff. `backoff = true` after
     * an empty/no-match result increments the consecutive-empty counter and
     * delays exponentially; after [MAX_CONSECUTIVE_EMPTY] empties in a row it
     * gives up (goes IDLE) so a silent room can't hold the mic forever. When the
     * optional [onMicBusy] gate reports PTT owns the mic, it defers without
     * counting that as a failure. Main thread only (recognizer requirement).
     */
    private fun scheduleRestart(backoff: Boolean) {
        if (!running) return
        pendingRestart?.let { mainHandler.removeCallbacks(it) }

        val delay: Long = if (backoff) {
            consecutiveEmpty++
            if (consecutiveEmpty >= MAX_CONSECUTIVE_EMPTY) {
                Log.i(TAG, "captioning paused after $consecutiveEmpty empty results (silent mic?)")
                running = false
                pendingRestart = null
                if (_status.value == Status.LISTENING) _status.value = Status.IDLE
                try {
                    onGaveUpListening?.invoke()
                } catch (t: Throwable) {
                    Log.w(TAG, "onGaveUpListening failed: ${t.message}")
                }
                return
            }
            // 400ms, 800, 1600, 3200, capped at 5s.
            (RESTART_BASE_MS shl (consecutiveEmpty - 1).coerceAtMost(4)).coerceAtMost(RESTART_MAX_MS)
        } else {
            consecutiveEmpty = 0
            RESTART_BASE_MS
        }

        val runnable = object : Runnable {
            override fun run() {
                if (!running) return
                // Don't fight PTT for the mic — wait and retry without penalty.
                if (onMicBusy?.invoke() == true) {
                    mainHandler.postDelayed(this, MIC_BUSY_RETRY_MS)
                    return
                }
                try {
                    recognizer?.startListening(buildRecognizerIntent())
                } catch (t: Throwable) {
                    Log.w(TAG, "restart failed: ${t.message}")
                    _status.value = Status.ERROR
                    _errorMessage.value = "Recognition restart failed"
                }
            }
        }
        pendingRestart = runnable
        mainHandler.postDelayed(runnable, delay)
    }

    private fun errorText(error: Int): String = when (error) {
        SpeechRecognizer.ERROR_AUDIO -> "Audio recording error (mic busy?)"
        SpeechRecognizer.ERROR_CLIENT -> "Client error"
        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "Microphone permission required"
        SpeechRecognizer.ERROR_NETWORK -> "Network error"
        SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "Network timeout"
        SpeechRecognizer.ERROR_NO_MATCH -> "No speech recognized"
        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "Recognizer busy (PTT using mic?)"
        SpeechRecognizer.ERROR_SERVER -> "Server error"
        SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> "No speech detected"
        SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED -> "Language not supported offline"
        SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE -> "Offline language model not installed"
        else -> "Recognition error ($error)"
    }
}

/** Default source/target language codes shared by the translation feature. */
object TranslationLangDefaults {
    /** What the local user speaks (English by default). */
    const val DEFAULT_SOURCE = "en"
    /** What we translate captions into by default (Spanish). */
    const val DEFAULT_TARGET = "es"
}
