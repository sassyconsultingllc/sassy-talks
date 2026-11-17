// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.Context
import android.os.Build
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/**
 * Radio start-up that must happen once per process before any transport can
 * run: native init, the post-update reset, session restore, identity and the
 * saved audio settings.
 *
 * It used to live in the Compose start-up of AppNavigation, so only an open
 * screen could bring the radio up. When Android killed the process, the sticky
 * WalkieService came back with no native session and no transports and sat in
 * "Radio standby" until the user opened the app. The service and the UI now
 * share this, and whichever runs first does the work.
 */
object RadioBootstrap {
    private const val TAG = "RadioBootstrap"

    /**
     * @param nativeReady the native library is initialised
     * @param sessionActive a session is installed right now (read live, not cached)
     * @param profileSet the user finished profile setup
     */
    data class State(
        val nativeReady: Boolean,
        val sessionActive: Boolean,
        val profileSet: Boolean,
    )

    private val mutex = Mutex()
    @Volatile private var started = false

    /**
     * Run the one-time start-up if this process has not yet, then report the
     * current state. Safe to call from any thread and any number of times.
     * Requires RECORD_AUDIO: the native init creates the audio engine.
     */
    suspend fun ensure(context: Context): State = mutex.withLock {
        val app = context.applicationContext
        withContext(Dispatchers.IO) {
            if (!started || !SassyTalkNative.isInitialized()) {
                started = startOnce(app)
            }
            State(
                nativeReady = started && SassyTalkNative.isInitialized(),
                sessionActive = started && !SassyTalkNative.getSessionId().isNullOrBlank(),
                profileSet = profileSet(app),
            )
        }
    }

    /** Forget the start-up after [SassyTalkNative.shutdown]; the next [ensure] redoes it. */
    fun reset() {
        started = false
    }

    /**
     * May the service bring the radio up with no UI open? Only when a launch of
     * the app would have gone straight to the radio screen anyway.
     */
    fun mayResumeWithoutUi(
        micGranted: Boolean,
        entitled: Boolean,
        state: State,
    ): Boolean = micGranted && entitled && state.nativeReady && state.profileSet && state.sessionActive

    private fun profileSet(app: Context): Boolean =
        app.getSharedPreferences("sassy_profile", Context.MODE_PRIVATE)
            .getBoolean(com.sassyconsulting.sassytalkie.ui.KEY_PROFILE_SET, false)

    private fun startOnce(app: Context): Boolean {
        SassyTalkNative.appContext = app
        val initOk = SassyTalkNative.isInitialized() || SassyTalkNative.init()
        if (!initOk) {
            Log.e(TAG, "Native init failed")
            return false
        }
        // Lets the native side reach AudioManager for routing. Idempotent.
        try { SassyTalkNative.initContext(app) } catch (_: Throwable) {}

        // Before restoring anything: an in-place update keeps state that the
        // new binary may not expect. See UpdateReset.
        UpdateReset.runIfUpdated(app)

        SassyTalkNative.restoreSession()
        SassyTalkNative.restoreCohortHistory()

        // The install id goes in before the name: the native sender identity
        // is derived from both, and identical names alone made two devices
        // drop each other's audio as self-echo.
        SassyTalkNative.setInstallId(InstallId.get(app))
        SassyTalkNative.setDeviceName(
            if (profileSet(app)) com.sassyconsulting.sassytalkie.ui.getSavedProfileName(app) else Build.MODEL
        )

        // Activity is session-scoped: no disk history across process restarts.
        TranscriptionBridge.initialize(app)
        TranscriptionBridge.discardPersistedTimeline()
        TranscriptionBridge.clearEntries()
        TranscriptionBridge.setEnabled(true)

        // Saved audio choices; defaults keep first-run behaviour unchanged.
        val prefs = app.getSharedPreferences("sassy_settings", Context.MODE_PRIVATE)
        SassyTalkNative.setMicGain(prefs.getFloat("mic_gain", 1.0f))
        SassyTalkNative.setSquelchDbfs(prefs.getInt("squelch_dbfs", 0))
        SassyTalkNative.setMixModeEnabled(prefs.getBoolean("enable_mix_mode", false))
        SassyTalkNative.setNoiseSuppressionEnabled(prefs.getBoolean("noise_suppression", false))
        // The sealed context itself comes from restoreSession(); this re-arms the toggle.
        SassyTalkNative.setSealedSenderEnabled(prefs.getBoolean("sealed_sender", false))
        SassyTalkNative.setRxGain(prefs.getFloat("rx_gain", 1.0f))
        SassyTalkNative.setSpeakerphonePreference(prefs.getBoolean("speakerphone_on", true))
        SassyTalkNative.setJitterPrebufferFrames(prefs.getInt("jitter_prebuffer_frames", 5))
        Log.i(TAG, "Radio start-up complete")
        return true
    }
}
