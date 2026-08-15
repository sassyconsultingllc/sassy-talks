// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-LTB7K9M2QXWP
package com.sassyconsulting.sassytalkie.translate

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * App-scoped live translation session — Settings configures it; Main / PiP
 * observe captions while it runs. Persists prefs, yields the mic to PTT,
 * records finals into Timeline, and optionally speaks translations.
 *
 * Mic ownership: screen consumers ([acquireUi]) plus a single process-foreground
 * consumer so captions keep working across in-app navigation without holding the
 * mic while the app is backgrounded (battery). WalkieService only mirrors the
 * latest line into the notification — it does not own the recognizer.
 *
 * Pipeline: local mic → [LiveCaptionTranslator] → [TranslationManager] (ML Kit).
 * Captions the LOCAL user only (platform SpeechRecognizer cannot ingest remote
 * PCM). Init once from [MainActivity]; release is process-lifetime.
 *
 * Speak translation: deferred across PTT — snapshots the last captioned line
 * at key-up and reads it back after release (STT cannot hear during TX).
 * TTS ducks for incoming peer audio and holds the recognizer so it does not
 * caption its own read-back.
 */
object LiveTranslationBridge {

    private const val TAG = "LiveTranslationBridge"
    private const val PREFS = "sassy_settings"
    private const val KEY_ENABLED = "live_translation_enabled"
    private const val KEY_SOURCE = "live_translation_source"
    private const val KEY_TARGET = "live_translation_target"
    private const val KEY_WIFI_ONLY = "live_translation_wifi_only"
    private const val KEY_TTS = "live_translation_tts"
    private const val KEY_TIMELINE = "live_translation_timeline"
    /** Delay after PTT release before reclaiming the mic for STT. */
    private const val RESUME_AFTER_PTT_MS = 350L
    /** Extra beat after PTT before deferred TTS so TX audio fully settles. */
    private const val POST_PTT_TTS_MS = 180L
    /** Delay before restarting STT after a silent-room give-up. */
    private const val SILENT_RESUME_MS = 2_500L
    /** Periodic health check while captioning should be running. */
    private const val HEALTH_CHECK_MS = 5_000L
    /** Safety: never hold STT forever if TTS onDone never fires. */
    private const val TTS_HOLD_TIMEOUT_MS = 12_000L

    private val mainHandler = Handler(Looper.getMainLooper())
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    @Volatile private var appContext: Context? = null
    private var prefs: SharedPreferences? = null
    private var manager: TranslationManager? = null
    private var translator: LiveCaptionTranslator? = null
    private var speaker: TranslationSpeaker? = null
    private var modelJob: Job? = null
    private val resourceBindingJobs = mutableListOf<Job>()
    private val handlerLock = Any()
    @Volatile private var pendingResume: Runnable? = null
    @Volatile private var pendingSilentResume: Runnable? = null
    @Volatile private var pendingTtsFlush: Runnable? = null
    @Volatile private var ttsHoldTimeout: Runnable? = null
    @Volatile private var healthWatchdog: Runnable? = null
    private val lifecycle = LiveTranslationLifecycle()
    /** Deduplicate consecutive identical finals (recognizer can re-emit). */
    @Volatile private var lastFinalKey: String = ""
    /** Last translation queued for post-PTT / post-RX TTS read-back. */
    @Volatile private var pendingTtsText: String = ""
    /** Avoid re-speaking the same deferred line. */
    @Volatile private var lastSpokenKey: String = ""
    /** True while post-PTT TTS owns the speaker path — STT stays stopped. */
    @Volatile private var holdingMicForTts = false
    /** Latest TTS utterance id; stale onDone from a ducked/stopped speak is ignored. */
    @Volatile private var activeTtsId: String? = null
    @Volatile private var processObserverRegistered = false
    @Volatile private var processForegroundHeld = false

    private val processObserver = object : DefaultLifecycleObserver {
        override fun onStart(owner: LifecycleOwner) {
            if (!_enabled.value || processForegroundHeld) return
            processForegroundHeld = true
            acquireUi()
        }

        override fun onStop(owner: LifecycleOwner) {
            if (!processForegroundHeld) return
            processForegroundHeld = false
            releaseUi()
        }
    }

    private val _enabled = MutableStateFlow(false)
    val enabled: StateFlow<Boolean> = _enabled.asStateFlow()

    private val _sourceLang = MutableStateFlow(TranslationLangDefaults.DEFAULT_SOURCE)
    val sourceLang: StateFlow<String> = _sourceLang.asStateFlow()

    private val _targetLang = MutableStateFlow(TranslationLangDefaults.DEFAULT_TARGET)
    val targetLang: StateFlow<String> = _targetLang.asStateFlow()

    private val _wifiOnlyModels = MutableStateFlow(true)
    val wifiOnlyModels: StateFlow<Boolean> = _wifiOnlyModels.asStateFlow()

    private val _ttsEnabled = MutableStateFlow(false)
    val ttsEnabled: StateFlow<Boolean> = _ttsEnabled.asStateFlow()

    private val _timelineEnabled = MutableStateFlow(true)
    val timelineEnabled: StateFlow<Boolean> = _timelineEnabled.asStateFlow()

    private val _pausedForPtt = MutableStateFlow(false)
    /** True while PTT owns the mic and captioning is deferred. */
    val pausedForPtt: StateFlow<Boolean> = _pausedForPtt.asStateFlow()

    private val _caption = MutableStateFlow("")
    val caption: StateFlow<String> = _caption.asStateFlow()

    private val _translation = MutableStateFlow("")
    val translation: StateFlow<String> = _translation.asStateFlow()

    private val _status = MutableStateFlow(LiveCaptionTranslator.Status.IDLE)
    val status: StateFlow<LiveCaptionTranslator.Status> = _status.asStateFlow()

    private val _errorMessage = MutableStateFlow<String?>(null)
    val errorMessage: StateFlow<String?> = _errorMessage.asStateFlow()

    private val _modelState = MutableStateFlow(TranslationManager.ModelState.UNKNOWN)
    val modelState: StateFlow<TranslationManager.ModelState> = _modelState.asStateFlow()

    private val _downloadedModels = MutableStateFlow<List<String>>(emptyList())
    /** BCP-47 codes currently on-device (for Settings model management). */
    val downloadedModels: StateFlow<List<String>> = _downloadedModels.asStateFlow()

    /** Call once from activity/application startup. Idempotent. */
    fun init(context: Context) {
        if (appContext != null) return
        val app = context.applicationContext
        appContext = app
        prefs = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val p = prefs!!
        _enabled.value = p.getBoolean(KEY_ENABLED, false) &&
            com.sassyconsulting.sassytalkie.ManagedConfig.translationAllowed(app)
        _sourceLang.value = persistableLang(
            p.getString(KEY_SOURCE, TranslationLangDefaults.DEFAULT_SOURCE),
            TranslationLangDefaults.DEFAULT_SOURCE,
        )
        _targetLang.value = persistableLang(
            p.getString(KEY_TARGET, TranslationLangDefaults.DEFAULT_TARGET),
            TranslationLangDefaults.DEFAULT_TARGET,
        )
        _wifiOnlyModels.value = p.getBoolean(KEY_WIFI_ONLY, true)
        // Sensible default: on for first install so post-PTT read-back is discoverable.
        _ttsEnabled.value = p.getBoolean(KEY_TTS, true)
        _timelineEnabled.value = p.getBoolean(KEY_TIMELINE, true)

        bindIncomingAudioDuck()
        registerProcessObserver()

        if (_enabled.value) {
            ensureTranslationResources()
            applyMicAction(lifecycle.setEnabled(true))
            ensureModels()
            refreshDownloadedModels()
            syncRunningState()
            startHealthWatchdog()
            // If already foreground at init, take the process consumer now.
            mainHandler.post {
                if (_enabled.value &&
                    ProcessLifecycleOwner.get().lifecycle.currentState
                        .isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)
                ) {
                    processObserver.onStart(ProcessLifecycleOwner.get())
                }
            }
        }
        Log.i(TAG, "init enabled=${_enabled.value} ${_sourceLang.value}→${_targetLang.value}")
    }

    fun setEnabled(on: Boolean) {
        val allowed = appContext?.let {
            com.sassyconsulting.sassytalkie.ManagedConfig.translationAllowed(it)
        } ?: true
        val next = on && allowed
        if (_enabled.value == next) return
        _enabled.value = next
        prefs?.edit()?.putBoolean(KEY_ENABLED, next)?.apply()
        cancelPendingResume()
        val action = lifecycle.setEnabled(next)
        _pausedForPtt.value = lifecycle.pausedForPtt
        if (next) {
            ensureTranslationResources()
            ensureSpeaker()
            ensureModels()
            refreshDownloadedModels()
            applyMicAction(action)
            syncRunningState()
            startHealthWatchdog()
            mainHandler.post {
                if (_enabled.value &&
                    ProcessLifecycleOwner.get().lifecycle.currentState
                        .isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)
                ) {
                    processObserver.onStart(ProcessLifecycleOwner.get())
                }
            }
        } else {
            // Drop process consumer first so refcount can't restart the mic,
            // then force STOP regardless of leftover Main/Settings acquires.
            if (processForegroundHeld) {
                processForegroundHeld = false
                lifecycle.releaseUi()
            }
            clearPendingTts()
            stopTtsPlayback()
            speaker?.release()
            speaker = null
            modelJob?.cancel()
            modelJob = null
            holdingMicForTts = false
            applyMicAction(action) // STOP from setEnabled(false)
            stopHealthWatchdog()
            lastFinalKey = ""
            lastSpokenKey = ""
            _caption.value = ""
            _translation.value = ""
            releaseTranslationResources()
        }
    }

    fun setSourceLang(code: String) {
        val normalized = TranslationManager.normalizeLanguage(code) ?: return
        if (_sourceLang.value == normalized) return
        _sourceLang.value = normalized
        prefs?.edit()?.putString(KEY_SOURCE, normalized)?.apply()
        manager?.clearCache()
        translator?.setLanguages(normalized, _targetLang.value)
        ensureModels()
        if (_enabled.value && !_pausedForPtt.value) {
            mainHandler.post {
                translator?.stop()
                syncRunningState()
            }
        }
    }

    fun setTargetLang(code: String) {
        val normalized = TranslationManager.normalizeLanguage(code) ?: return
        if (_targetLang.value == normalized) return
        _targetLang.value = normalized
        prefs?.edit()?.putString(KEY_TARGET, normalized)?.apply()
        manager?.clearCache()
        translator?.setLanguages(_sourceLang.value, normalized)
        speaker?.setLanguage(normalized)
        ensureModels()
    }

    /**
     * Swap speak/translate languages in one shot so the mic + model pair stay
     * consistent (avoids a transient source==target window from two setters).
     */
    fun swapLanguages() {
        val src = _sourceLang.value
        val tgt = _targetLang.value
        if (src == tgt) return
        _sourceLang.value = tgt
        _targetLang.value = src
        prefs?.edit()
            ?.putString(KEY_SOURCE, tgt)
            ?.putString(KEY_TARGET, src)
            ?.apply()
        manager?.clearCache()
        translator?.setLanguages(tgt, src)
        speaker?.setLanguage(src)
        ensureModels()
        if (_enabled.value && !_pausedForPtt.value) {
            mainHandler.post {
                translator?.stop()
                syncRunningState()
            }
        }
        _caption.value = ""
        _translation.value = ""
        lastFinalKey = ""
    }

    fun setWifiOnlyModels(wifiOnly: Boolean) {
        if (_wifiOnlyModels.value == wifiOnly) return
        _wifiOnlyModels.value = wifiOnly
        prefs?.edit()?.putBoolean(KEY_WIFI_ONLY, wifiOnly)?.apply()
        translator?.setRequireWifi(wifiOnly)
        if (_enabled.value) ensureModels()
    }

    fun setTtsEnabled(on: Boolean) {
        if (_ttsEnabled.value == on) return
        _ttsEnabled.value = on
        prefs?.edit()?.putBoolean(KEY_TTS, on)?.apply()
        if (!on) {
            clearPendingTts()
            stopTtsPlayback()
            finishTtsHoldAndMaybeResumeMic()
        }
    }

    fun setTimelineEnabled(on: Boolean) {
        if (_timelineEnabled.value == on) return
        _timelineEnabled.value = on
        prefs?.edit()?.putBoolean(KEY_TIMELINE, on)?.apply()
    }

    /** Re-attempt model download for the current language pair. */
    fun retryModelDownload() {
        ensureModels()
    }

    /**
     * Re-speak a Timeline caption/translation row via on-device TTS.
     * Used by Activity playback when the entry has no PCM cache (local:self).
     */
    fun speakTimelineText(text: String): Boolean {
        val speakable = LiveTranslationText.speakableFromTimeline(text)
        if (speakable.isEmpty()) return false
        ensureSpeaker()
        val engine = speaker ?: return false
        engine.setLanguage(_targetLang.value)
        mainHandler.post { engine.speak(speakable) }
        return true
    }

    /** Stop Timeline / translation TTS (e.g. leaving Activity). */
    fun stopSpeaking() {
        mainHandler.post { stopTtsPlayback() }
    }

    fun refreshDownloadedModels() {
        val tm = manager ?: return
        scope.launch {
            _downloadedModels.value = tm.downloadedModels().sorted()
        }
    }

    fun deleteLanguageModel(code: String) {
        val tm = manager ?: return
        scope.launch {
            tm.deleteModel(code)
            _downloadedModels.value = tm.downloadedModels().sorted()
            // If we deleted a language still in use, kick a re-download.
            if (code == _sourceLang.value || code == _targetLang.value) {
                ensureModels()
            }
        }
    }

    /**
     * Main / Settings / PiP / WalkieService call this while they want captions
     * alive. Recognition only holds the mic when at least one consumer is active.
     */
    fun acquireUi() {
        applyMicAction(lifecycle.acquireUi())
        syncRunningState()
    }

    fun releaseUi() {
        val action = lifecycle.releaseUi()
        if (action == LiveTranslationLifecycle.MicAction.STOP) {
            cancelPendingResume()
            clearPendingTts()
            stopTtsPlayback()
            holdingMicForTts = false
        }
        applyMicAction(action)
    }

    /**
     * Yield the mic immediately when PTT keys up. Idempotent — safe to call from
     * both [com.sassyconsulting.sassytalkie.PttCoordinator] and
     * [SassyTalkNative] paths.
     *
     * Snapshots the current translation for post-PTT TTS read-back (STT cannot
     * hear during TX — this is the last captioned line before / at key-up).
     */
    fun onPttStarted() {
        cancelPendingResume()
        cancelPendingTtsFlush()
        // Snapshot before STOP so post-PTT read-back has something to speak.
        if (_ttsEnabled.value && _sourceLang.value != _targetLang.value) {
            val snap = LiveTranslationText.speakableUtterance(_caption.value, _translation.value)
            if (snap.isNotBlank()) pendingTtsText = snap
        }
        val action = lifecycle.onPttStarted()
        _pausedForPtt.value = lifecycle.pausedForPtt
        if (action == LiveTranslationLifecycle.MicAction.STOP) {
            // If we cut off an in-progress read-back, allow the same line after release.
            if (holdingMicForTts) lastSpokenKey = ""
            holdingMicForTts = false
            cancelTtsHoldTimeout()
            stopTtsPlayback()
            applyMicAction(action)
        }
    }

    /**
     * Resume captioning shortly after PTT releases the mic. Coalesces duplicate
     * calls from [SassyTalkNative.pttStop] + [com.sassyconsulting.sassytalkie.PttCoordinator]
     * so the delay is not reset repeatedly on the same release.
     *
     * When Speak translation is on, deferred TTS plays first and STT stays
     * stopped until the utterance ends (so the recognizer does not caption TTS).
     */
    fun onPttReleased() {
        if (!lifecycle.enabled) {
            _pausedForPtt.value = false
            return
        }
        // Already scheduled — leave the original timer alone.
        val resume = object : Runnable {
            override fun run() {
                synchronized(handlerLock) {
                    if (pendingResume !== this) return
                    pendingResume = null
                }
                val pttActive = currentPttActive()
                val action = lifecycle.onPttResumeReady(pttActive)
                _pausedForPtt.value = lifecycle.pausedForPtt
                if (pttActive || lifecycle.pausedForPtt) {
                    applyMicAction(action)
                    return
                }
                val incoming = currentIncomingAudio()
                // Prefer deferred read-back before reclaiming the mic for STT,
                // unless a peer is already talking (incoming-end will flush).
                if (_ttsEnabled.value && pendingTtsText.isNotBlank() && !incoming) {
                    holdingMicForTts = true
                    applyMicAction(LiveTranslationLifecycle.MicAction.STOP)
                    scheduleDeferredTtsFlush(POST_PTT_TTS_MS)
                } else {
                    applyMicAction(action)
                    syncRunningState()
                }
            }
        }
        synchronized(handlerLock) {
            if (pendingResume != null) return
            pendingResume = resume
            mainHandler.postDelayed(resume, RESUME_AFTER_PTT_MS)
        }
    }

    /**
     * Best-effort deep-link into system / Google voice settings so the user can
     * install an offline speech pack. Returns true if an activity was launched.
     */
    fun openOfflineSpeechSettings(context: Context): Boolean {
        val flags = Intent.FLAG_ACTIVITY_NEW_TASK
        val candidates = listOf(
            Intent(Settings.ACTION_VOICE_INPUT_SETTINGS).addFlags(flags),
            Intent(Intent.ACTION_MAIN).addFlags(flags).setComponent(
                ComponentName(
                    "com.google.android.googlequicksearchbox",
                    "com.google.android.apps.gsa.settingsui.VoiceSearchPreferences",
                ),
            ),
            Intent(Intent.ACTION_MAIN).addFlags(flags).setComponent(
                ComponentName(
                    "com.google.android.googlequicksearchbox",
                    "com.google.android.voicesearch.VoiceSearchPreferences",
                ),
            ),
            Intent(Settings.ACTION_SETTINGS).addFlags(flags),
        )
        for (intent in candidates) {
            try {
                context.startActivity(intent)
                return true
            } catch (_: Throwable) {
                // try next
            }
        }
        return false
    }

    private fun registerProcessObserver() {
        if (processObserverRegistered) return
        processObserverRegistered = true
        mainHandler.post {
            try {
                ProcessLifecycleOwner.get().lifecycle.addObserver(processObserver)
            } catch (t: Throwable) {
                Log.w(TAG, "process observer failed: ${t.message}")
                processObserverRegistered = false
            }
        }
    }

    private fun handleFinalUtterance(caption: String, translation: String) {
        if (!_enabled.value) return
        val key = LiveTranslationText.normalizeKey("$caption|$translation")
        if (key.isEmpty() || key == lastFinalKey) return
        lastFinalKey = key

        if (_timelineEnabled.value) {
            val name = try {
                SassyTalkNative.getDeviceName().ifBlank { "You" }
            } catch (_: Throwable) {
                "You"
            }
            try {
                TranscriptionBridge.recordLocalCaption(caption, translation, name)
            } catch (t: Throwable) {
                Log.w(TAG, "timeline record failed: ${t.message}")
            }
        }
        // Skip TTS when source==target (identity) — no new audio value, just echo.
        val identity = _sourceLang.value == _targetLang.value
        if (identity) return

        val speakText = LiveTranslationText.speakableUtterance(caption, translation)
        if (speakText.isBlank()) return

        val incoming = currentIncomingAudio()
        val paused = _pausedForPtt.value || holdingMicForTts || lifecycle.pausedForIncoming
        when {
            LiveTranslationText.shouldSpeakTts(_ttsEnabled.value, paused, incoming) ->
                speakNow(speakText)
            LiveTranslationText.shouldQueueTts(_ttsEnabled.value, paused, incoming) ->
                pendingTtsText = speakText
        }
    }

    /** Stop TTS the moment a peer starts speaking so read-back never talks over RX. */
    private fun bindIncomingAudioDuck() {
        scope.launch {
            TranscriptionBridge.incomingAudio.collect { incoming ->
                mainHandler.post {
                    if (incoming) {
                        stopTtsPlayback()
                        if (holdingMicForTts) {
                            holdingMicForTts = false
                            cancelTtsHoldTimeout()
                        }
                        applyMicAction(lifecycle.onIncomingStarted())
                    } else if (lifecycle.pausedForIncoming) {
                        val action = lifecycle.onIncomingEnded()
                        if (_ttsEnabled.value &&
                            pendingTtsText.isNotBlank() &&
                            !_pausedForPtt.value &&
                            lifecycle.enabled
                        ) {
                            holdingMicForTts = true
                            applyMicAction(LiveTranslationLifecycle.MicAction.STOP)
                            scheduleDeferredTtsFlush(POST_PTT_TTS_MS)
                        } else {
                            applyMicAction(action)
                            syncRunningState()
                        }
                    }
                }
            }
        }
    }

    private fun currentIncomingAudio(): Boolean = try {
        TranscriptionBridge.incomingAudio.value
    } catch (_: Throwable) {
        false
    }

    private fun persistableLang(raw: String?, fallback: String): String {
        val value = raw ?: fallback
        return TranslationManager.normalizeLanguage(value) ?: fallback
    }

    private fun currentPttActive(): Boolean = try {
        SassyTalkNative.isPttActive()
    } catch (_: Throwable) {
        false
    }

    private fun ensureSpeaker() {
        val ctx = appContext ?: return
        if (speaker != null) return
        speaker = TranslationSpeaker(ctx).also { sp ->
            sp.setLanguage(_targetLang.value)
            sp.onUtteranceDone = { id -> mainHandler.post { onTtsUtteranceFinished(id) } }
        }
    }

    /** Build heavy recognizer/ML Kit clients only while the feature is enabled. */
    private fun ensureTranslationResources() {
        if (manager != null && translator != null) return
        val app = appContext ?: return
        val tm = TranslationManager()
        val cap = LiveCaptionTranslator(app, tm).apply {
            onMicBusy = {
                currentPttActive() || _pausedForPtt.value || lifecycle.pausedForIncoming
            }
            setLanguages(_sourceLang.value, _targetLang.value)
            setRequireWifi(_wifiOnlyModels.value)
            onFinalUtterance = { caption, translation ->
                handleFinalUtterance(caption, translation)
            }
            onGaveUpListening = { scheduleSilentResume() }
        }
        manager = tm
        translator = cap
        bindTranslatorFlows(cap)
    }

    private fun releaseTranslationResources() {
        resourceBindingJobs.forEach { it.cancel() }
        resourceBindingJobs.clear()
        try { translator?.release() } catch (_: Throwable) {}
        translator = null
        try { manager?.release() } catch (_: Throwable) {}
        manager = null
        _modelState.value = TranslationManager.ModelState.UNKNOWN
    }

    private fun speakNow(text: String) {
        val key = LiveTranslationText.normalizeKey(text)
        if (key.isEmpty() || key == lastSpokenKey) return
        if (holdingMicForTts || activeTtsId != null) {
            pendingTtsText = text
            return
        }
        ensureSpeaker()
        val engine = speaker ?: return
        lastSpokenKey = key
        pendingTtsText = ""
        mainHandler.post {
            if (!LiveTranslationText.canSpeakNow(
                    ttsEnabled = _ttsEnabled.value,
                    featureEnabled = _enabled.value,
                    pausedForPtt = _pausedForPtt.value,
                    incomingAudio = currentIncomingAudio(),
                    pttActive = currentPttActive(),
                )
            ) {
                pendingTtsText = text
                lastSpokenKey = ""
                finishTtsHoldAndMaybeResumeMic()
                return@post
            }
            // Hold STT while we speak so the recognizer does not caption TTS.
            holdingMicForTts = true
            applyMicAction(LiveTranslationLifecycle.MicAction.STOP)
            val startedId = engine.speak(text)
            if (startedId != null) {
                activeTtsId = startedId
                armTtsHoldTimeout()
            } else {
                pendingTtsText = text
                lastSpokenKey = ""
                finishTtsHoldAndMaybeResumeMic()
            }
        }
    }

    /**
     * Speak deferred post-PTT / post-RX translation. Holds STT until utterance
     * completes so the recognizer does not caption the read-back.
     */
    private fun scheduleDeferredTtsFlush(delayMs: Long) {
        val flush = object : Runnable {
            override fun run() {
                synchronized(handlerLock) {
                    if (pendingTtsFlush !== this) return
                    pendingTtsFlush = null
                }
                flushPendingTts()
            }
        }
        synchronized(handlerLock) {
            pendingTtsFlush?.let { mainHandler.removeCallbacks(it) }
            pendingTtsFlush = flush
            mainHandler.postDelayed(flush, delayMs)
        }
    }

    private fun flushPendingTts() {
        if (!_ttsEnabled.value || !_enabled.value) {
            finishTtsHoldAndMaybeResumeMic()
            return
        }
        if (_pausedForPtt.value || currentPttActive()) {
            finishTtsHoldAndMaybeResumeMic()
            return
        }
        val incoming = currentIncomingAudio()
        val pttActive = currentPttActive()
        if (incoming || pttActive) {
            finishTtsHoldAndMaybeResumeMic()
            return
        }

        val text = pendingTtsText.trim()
        if (text.isEmpty()) {
            finishTtsHoldAndMaybeResumeMic()
            return
        }
        val key = LiveTranslationText.normalizeKey(text)
        if (key == lastSpokenKey) {
            pendingTtsText = ""
            finishTtsHoldAndMaybeResumeMic()
            return
        }

        if (!LiveTranslationText.canSpeakNow(
                ttsEnabled = _ttsEnabled.value,
                featureEnabled = _enabled.value,
                pausedForPtt = _pausedForPtt.value,
                incomingAudio = incoming,
                pttActive = pttActive,
            )
        ) {
            finishTtsHoldAndMaybeResumeMic()
            return
        }

        ensureSpeaker()
        val engine = speaker
        if (engine == null) {
            finishTtsHoldAndMaybeResumeMic()
            return
        }

        holdingMicForTts = true
        applyMicAction(LiveTranslationLifecycle.MicAction.STOP)
        lastSpokenKey = key
        pendingTtsText = ""
        val startedId = engine.speak(text)
        if (startedId != null) {
            activeTtsId = startedId
            armTtsHoldTimeout()
        } else {
            // Not ready yet — restore pending and resume mic; next final can retry.
            pendingTtsText = text
            lastSpokenKey = ""
            finishTtsHoldAndMaybeResumeMic()
        }
    }

    private fun onTtsUtteranceFinished(utteranceId: String?) {
        if (utteranceId != null && utteranceId != activeTtsId) return
        activeTtsId = null
        reassertRxRoute()
        finishTtsHoldAndMaybeResumeMic()
    }

    private fun stopTtsPlayback() {
        activeTtsId = null
        try { speaker?.stop() } catch (_: Throwable) {}
    }

    private fun finishTtsHoldAndMaybeResumeMic() {
        cancelTtsHoldTimeout()
        val wasHolding = holdingMicForTts
        holdingMicForTts = false
        if (!wasHolding) return
        if (!_enabled.value || _pausedForPtt.value || lifecycle.pausedForIncoming) return
        val pttActive = currentPttActive()
        if (pttActive) return
        syncRunningState()
    }

    private fun armTtsHoldTimeout() {
        val timeout = object : Runnable {
            override fun run() {
                synchronized(handlerLock) {
                    if (ttsHoldTimeout !== this) return
                    ttsHoldTimeout = null
                }
                Log.w(TAG, "TTS hold timed out — resuming captioning")
                finishTtsHoldAndMaybeResumeMic()
            }
        }
        synchronized(handlerLock) {
            ttsHoldTimeout?.let { mainHandler.removeCallbacks(it) }
            ttsHoldTimeout = timeout
            mainHandler.postDelayed(timeout, TTS_HOLD_TIMEOUT_MS)
        }
    }

    private fun cancelTtsHoldTimeout() {
        synchronized(handlerLock) {
            ttsHoldTimeout?.let { mainHandler.removeCallbacks(it) }
            ttsHoldTimeout = null
        }
    }

    private fun cancelPendingTtsFlush() {
        synchronized(handlerLock) {
            pendingTtsFlush?.let { mainHandler.removeCallbacks(it) }
            pendingTtsFlush = null
        }
    }

    private fun clearPendingTts() {
        cancelPendingTtsFlush()
        cancelTtsHoldTimeout()
        pendingTtsText = ""
    }

    private fun applyMicAction(action: LiveTranslationLifecycle.MicAction) {
        when (action) {
            LiveTranslationLifecycle.MicAction.START ->
                mainHandler.post {
                    if (holdingMicForTts) return@post
                    if (lifecycle.shouldRun(currentPttActive())) {
                        translator?.start()
                        reassertRxRoute()
                    }
                }
            LiveTranslationLifecycle.MicAction.STOP ->
                mainHandler.post {
                    translator?.stop()
                    reassertRxRoute()
                }
            LiveTranslationLifecycle.MicAction.NONE -> Unit
        }
    }

    private fun reassertRxRoute() {
        try { SassyTalkNative.reassertRxRoute() } catch (_: Throwable) {}
    }

    private fun syncRunningState() {
        if (holdingMicForTts) return
        val pttActive = currentPttActive()
        if (pttActive && lifecycle.enabled) {
            applyMicAction(lifecycle.onPttStarted())
            _pausedForPtt.value = lifecycle.pausedForPtt
            return
        }
        if (lifecycle.shouldRun(pttActive = false)) {
            applyMicAction(LiveTranslationLifecycle.MicAction.START)
        } else if (!lifecycle.enabled || lifecycle.uiConsumers <= 0) {
            applyMicAction(LiveTranslationLifecycle.MicAction.STOP)
        }
    }

    private fun ensureModels() {
        val tm = manager ?: return
        val src = _sourceLang.value
        val dst = _targetLang.value
        val wifiOnly = _wifiOnlyModels.value
        modelJob?.cancel()
        // Only flip the banner when we don't already know both models are present.
        val cached = _downloadedModels.value
        val alreadyOnDevice = src in cached && dst in cached
        if (!alreadyOnDevice) {
            _modelState.value = TranslationManager.ModelState.DOWNLOADING
        }
        modelJob = scope.launch {
            try {
                tm.downloadModelsIfNeeded(src, dst, requireWifi = wifiOnly)
                _downloadedModels.value = tm.downloadedModels().sorted()
            } catch (_: kotlinx.coroutines.CancellationException) {
                // Superseded by another language pick — new job owns state.
            }
        }
    }

    private fun bindTranslatorFlows(cap: LiveCaptionTranslator) {
        resourceBindingJobs += scope.launch {
            cap.caption.collect { _caption.value = it }
        }
        resourceBindingJobs += scope.launch {
            cap.translation.collect { _translation.value = it }
        }
        resourceBindingJobs += scope.launch {
            cap.status.collect { _status.value = it }
        }
        resourceBindingJobs += scope.launch {
            cap.errorMessage.collect { _errorMessage.value = it }
        }
        val tm = manager ?: return
        resourceBindingJobs += scope.launch {
            tm.modelState.collect { _modelState.value = it }
        }
    }

    private fun cancelPendingResume() {
        synchronized(handlerLock) {
            pendingResume?.let { mainHandler.removeCallbacks(it) }
            pendingResume = null
        }
        cancelPendingTtsFlush()
        synchronized(handlerLock) {
            pendingSilentResume?.let { mainHandler.removeCallbacks(it) }
            pendingSilentResume = null
        }
    }

    /**
     * After the recognizer quits on a quiet mic, wait briefly then restart if
     * captioning is still supposed to be active (radio standby is usually silent).
     */
    private fun scheduleSilentResume() {
        if (!lifecycle.enabled || lifecycle.pausedForPtt || lifecycle.pausedForIncoming ||
            holdingMicForTts || currentIncomingAudio()
        ) return
        val resume = object : Runnable {
            override fun run() {
                synchronized(handlerLock) {
                    if (pendingSilentResume !== this) return
                    pendingSilentResume = null
                }
                if (holdingMicForTts || currentIncomingAudio()) return
                val pttActive = currentPttActive()
                if (!lifecycle.shouldRun(pttActive)) return
                if (_status.value == LiveCaptionTranslator.Status.LISTENING) return
                Log.i(TAG, "restarting captioning after silent-room pause")
                applyMicAction(LiveTranslationLifecycle.MicAction.START)
            }
        }
        synchronized(handlerLock) {
            pendingSilentResume?.let { mainHandler.removeCallbacks(it) }
            pendingSilentResume = resume
            mainHandler.postDelayed(resume, SILENT_RESUME_MS)
        }
    }

    private fun startHealthWatchdog() {
        if (healthWatchdog != null) return
        if (!_enabled.value) return
        val tick = object : Runnable {
            override fun run() {
                if (!_enabled.value) {
                    healthWatchdog = null
                    return
                }
                healthWatchdog = this
                val pttActive = currentPttActive()
                if (lifecycle.shouldRun(pttActive) &&
                    !holdingMicForTts &&
                    _status.value != LiveCaptionTranslator.Status.LISTENING &&
                    _status.value != LiveCaptionTranslator.Status.UNAVAILABLE
                ) {
                    // Recognizer idle/error while we still want captions — nudge it.
                    if (pendingSilentResume == null) scheduleSilentResume()
                }
                mainHandler.postDelayed(this, HEALTH_CHECK_MS)
            }
        }
        healthWatchdog = tick
        mainHandler.postDelayed(tick, HEALTH_CHECK_MS)
    }

    private fun stopHealthWatchdog() {
        healthWatchdog?.let { mainHandler.removeCallbacks(it) }
        healthWatchdog = null
    }
}
