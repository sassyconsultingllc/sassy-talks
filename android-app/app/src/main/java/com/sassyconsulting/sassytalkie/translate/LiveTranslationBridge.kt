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
    /** Delay before restarting STT after a silent-room give-up. */
    private const val SILENT_RESUME_MS = 2_500L
    /** Periodic health check while captioning should be running. */
    private const val HEALTH_CHECK_MS = 5_000L

    private val mainHandler = Handler(Looper.getMainLooper())
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    @Volatile private var appContext: Context? = null
    private var prefs: SharedPreferences? = null
    private var manager: TranslationManager? = null
    private var translator: LiveCaptionTranslator? = null
    private var speaker: TranslationSpeaker? = null
    private var modelJob: Job? = null
    private var pendingResume: Runnable? = null
    private var pendingSilentResume: Runnable? = null
    private var healthWatchdog: Runnable? = null
    private val lifecycle = LiveTranslationLifecycle()
    /** Deduplicate consecutive identical finals (recognizer can re-emit). */
    @Volatile private var lastFinalKey: String = ""
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
        _enabled.value = p.getBoolean(KEY_ENABLED, false)
        _sourceLang.value = p.getString(KEY_SOURCE, TranslationLangDefaults.DEFAULT_SOURCE)
            ?: TranslationLangDefaults.DEFAULT_SOURCE
        _targetLang.value = p.getString(KEY_TARGET, TranslationLangDefaults.DEFAULT_TARGET)
            ?: TranslationLangDefaults.DEFAULT_TARGET
        _wifiOnlyModels.value = p.getBoolean(KEY_WIFI_ONLY, true)
        _ttsEnabled.value = p.getBoolean(KEY_TTS, false)
        _timelineEnabled.value = p.getBoolean(KEY_TIMELINE, true)

        val tm = TranslationManager()
        manager = tm
        speaker = TranslationSpeaker(app).also { it.setLanguage(_targetLang.value) }
        val cap = LiveCaptionTranslator(app, tm).apply {
            onMicBusy = { SassyTalkNative.isPttActive() || _pausedForPtt.value }
            setLanguages(_sourceLang.value, _targetLang.value)
            setRequireWifi(_wifiOnlyModels.value)
            onFinalUtterance = { caption, translation ->
                handleFinalUtterance(caption, translation)
            }
            onGaveUpListening = { scheduleSilentResume() }
        }
        translator = cap
        bindTranslatorFlows(cap)
        bindIncomingAudioDuck()
        registerProcessObserver()

        if (_enabled.value) {
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
        if (_enabled.value == on) return
        _enabled.value = on
        prefs?.edit()?.putBoolean(KEY_ENABLED, on)?.apply()
        cancelPendingResume()
        val action = lifecycle.setEnabled(on)
        _pausedForPtt.value = lifecycle.pausedForPtt
        if (on) {
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
            speaker?.stop()
            applyMicAction(action) // STOP from setEnabled(false)
            stopHealthWatchdog()
            lastFinalKey = ""
            _caption.value = ""
            _translation.value = ""
        }
    }

    fun setSourceLang(code: String) {
        val normalized = TranslationManager.normalizeLanguage(code) ?: code
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
        val normalized = TranslationManager.normalizeLanguage(code) ?: code
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
        if (!on) speaker?.stop()
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
            speaker?.stop()
        }
        applyMicAction(action)
    }

    /**
     * Yield the mic immediately when PTT keys up. Idempotent — safe to call from
     * both [com.sassyconsulting.sassytalkie.PttCoordinator] and
     * [SassyTalkNative] paths.
     */
    fun onPttStarted() {
        cancelPendingResume()
        val action = lifecycle.onPttStarted()
        _pausedForPtt.value = lifecycle.pausedForPtt
        if (action == LiveTranslationLifecycle.MicAction.STOP) {
            speaker?.stop()
            applyMicAction(action)
        }
    }

    /**
     * Resume captioning shortly after PTT releases the mic. Coalesces duplicate
     * calls from [SassyTalkNative.pttStop] + [com.sassyconsulting.sassytalkie.PttCoordinator]
     * so the delay is not reset repeatedly on the same release.
     */
    fun onPttReleased() {
        if (!lifecycle.enabled) {
            _pausedForPtt.value = false
            return
        }
        // Already scheduled — leave the original timer alone.
        if (pendingResume != null) return
        val resume = Runnable {
            pendingResume = null
            val pttActive = try { SassyTalkNative.isPttActive() } catch (_: Throwable) { false }
            val action = lifecycle.onPttResumeReady(pttActive)
            _pausedForPtt.value = lifecycle.pausedForPtt
            applyMicAction(action)
            syncRunningState()
        }
        pendingResume = resume
        mainHandler.postDelayed(resume, RESUME_AFTER_PTT_MS)
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
        val incoming = try {
            TranscriptionBridge.incomingAudio.value
        } catch (_: Throwable) {
            false
        }
        // Skip TTS when source==target (identity) — no new audio value, just echo.
        val identity = _sourceLang.value == _targetLang.value
        if (!identity &&
            LiveTranslationText.shouldSpeakTts(_ttsEnabled.value, _pausedForPtt.value, incoming)
        ) {
            val speakText = translation.ifBlank { caption }
            mainHandler.post { speaker?.speak(speakText) }
        }
    }

    /** Stop TTS the moment a peer starts speaking so read-back never talks over RX. */
    private fun bindIncomingAudioDuck() {
        scope.launch {
            TranscriptionBridge.incomingAudio.collect { incoming ->
                if (incoming) {
                    mainHandler.post { speaker?.stop() }
                }
            }
        }
    }

    private fun applyMicAction(action: LiveTranslationLifecycle.MicAction) {
        when (action) {
            LiveTranslationLifecycle.MicAction.START ->
                mainHandler.post {
                    if (lifecycle.shouldRun(
                            try { SassyTalkNative.isPttActive() } catch (_: Throwable) { false }
                        )
                    ) {
                        translator?.start()
                    }
                }
            LiveTranslationLifecycle.MicAction.STOP ->
                mainHandler.post { translator?.stop() }
            LiveTranslationLifecycle.MicAction.NONE -> Unit
        }
    }

    private fun syncRunningState() {
        val pttActive = try { SassyTalkNative.isPttActive() } catch (_: Throwable) { false }
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
        modelJob = scope.launch {
            tm.downloadModelsIfNeeded(src, dst, requireWifi = wifiOnly)
            _downloadedModels.value = tm.downloadedModels().sorted()
        }
    }

    private fun bindTranslatorFlows(cap: LiveCaptionTranslator) {
        scope.launch {
            cap.caption.collect { _caption.value = it }
        }
        scope.launch {
            cap.translation.collect { _translation.value = it }
        }
        scope.launch {
            cap.status.collect { _status.value = it }
        }
        scope.launch {
            cap.errorMessage.collect { _errorMessage.value = it }
        }
        val tm = manager ?: return
        scope.launch {
            tm.modelState.collect { _modelState.value = it }
        }
    }

    private fun cancelPendingResume() {
        pendingResume?.let { mainHandler.removeCallbacks(it) }
        pendingResume = null
        // Also drop silent-room restarts — otherwise a timer armed before
        // disable/PTT could fire and briefly reopen the mic.
        pendingSilentResume?.let { mainHandler.removeCallbacks(it) }
        pendingSilentResume = null
    }

    /**
     * After the recognizer quits on a quiet mic, wait briefly then restart if
     * captioning is still supposed to be active (radio standby is usually silent).
     */
    private fun scheduleSilentResume() {
        if (!lifecycle.enabled || lifecycle.pausedForPtt) return
        pendingSilentResume?.let { mainHandler.removeCallbacks(it) }
        val resume = Runnable {
            pendingSilentResume = null
            val pttActive = try { SassyTalkNative.isPttActive() } catch (_: Throwable) { false }
            if (!lifecycle.shouldRun(pttActive)) return@Runnable
            if (_status.value == LiveCaptionTranslator.Status.LISTENING) return@Runnable
            Log.i(TAG, "restarting captioning after silent-room pause")
            applyMicAction(LiveTranslationLifecycle.MicAction.START)
        }
        pendingSilentResume = resume
        mainHandler.postDelayed(resume, SILENT_RESUME_MS)
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
                val pttActive = try { SassyTalkNative.isPttActive() } catch (_: Throwable) { false }
                if (lifecycle.shouldRun(pttActive) &&
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
