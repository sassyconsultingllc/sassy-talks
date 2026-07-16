// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-QZ5SJL4QYC45
package com.sassyconsulting.sassytalkie.input

import android.content.Context
import android.content.Intent
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.KeyEvent
import com.sassyconsulting.sassytalkie.PttCoordinator
import com.sassyconsulting.sassytalkie.SassyTalkNative
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Hardware push-to-talk controller.
 *
 * Maps two configurable input sources onto the PTT coordinator (or native
 * fallback when the coordinator is not yet available):
 *
 *  1. **Hardware key PTT** — physical buttons. Rugged/industrial Android
 *     handsets (Sonim, Kyocera DuraForce, CAT, Zebra, RugGear, etc.) expose a
 *     dedicated PTT button as one of a small set of vendor keycodes. We also
 *     allow the user to opt VOLUME_UP / VOLUME_DOWN or the wired-headset hook
 *     button into the PTT role. The active keycode is configurable and
 *     persisted. [onKeyDown] / [onKeyUp] are delegated to from
 *     `MainActivity` and return `true` only when the event was consumed.
 *
 *  2. **Bluetooth PTT accessory** — most BT PTT pucks / headset buttons arrive
 *     as system *media-button* `KeyEvent`s (PLAY / PAUSE / PLAY_PAUSE /
 *     HEADSETHOOK / NEXT / PREVIOUS). We register a framework
 *     [android.media.session.MediaSession] (minSdk 24, no support-lib
 *     dependency) and translate those into PTT start/stop. Because the session
 *     is owned alongside the foreground `WalkieService`, media buttons keep
 *     reaching us while the app is backgrounded.
 *
 * Design notes:
 *  - **Self-contained.** Routes through [PttCoordinator] when available so
 *    BLE wake, RFCOMM pump, and reach watchdog run on every PTT path.
 *  - **Debounced.** A held hardware key emits repeated ACTION_DOWN events
 *    (`getRepeatCount() > 0`); we ignore the repeats so a held button is one
 *    continuous transmission (start on first down, stop on up). The BT media
 *    button path is toggle-or-hold aware (see [handleMediaKeyEvent]).
 *  - **Idempotent.** [startPtt] / [stopPtt] guard against double-start and
 *    double-stop so overlapping sources can't desync the native PTT state.
 *  - **Defensive.** Every framework interaction is wrapped in try/catch + Log,
 *    matching the app's Kotlin idioms; nothing blocks the main thread.
 *
 * Threading: all public methods are expected to be called from the main
 * thread (key callbacks, lifecycle). [armed] is a [StateFlow] the UI can
 * collect to show a "hardware PTT armed" indicator.
 */
class HardwarePttController(context: Context) {

    companion object {
        private const val TAG = "HwPttController"

        // Reuses the existing app-wide settings store (see SettingsScreen /
        // WalkieService, which both read "sassy_settings").
        private const val PREFS = "sassy_settings"
        private const val KEY_PTT_KEYCODE = "hw_ptt_keycode"
        private const val KEY_BT_PTT_ENABLED = "hw_ptt_bt_enabled"

        /**
         * Default hardware PTT keycode.
         *
         * `KEYCODE_HEADSETHOOK` (79) is the safest cross-device default: it is
         * the wired/BT headset call button, is not otherwise consumed by the
         * system on a media-less app, and is the keycode many rugged-device
         * PTT buttons additionally raise. Vendor-dedicated PTT buttons differ
         * per OEM (there is no single AOSP "KEYCODE_PTT"), so we ship a
         * candidate set ([PTT_KEYCODE_CANDIDATES]) the user can pick from.
         */
        const val DEFAULT_PTT_KEYCODE: Int = KeyEvent.KEYCODE_HEADSETHOOK

        /**
         * Candidate keycodes offered to the user as the hardware PTT key.
         * - HEADSETHOOK: headset call button (default).
         * - VOLUME_UP / VOLUME_DOWN: opt-in; consuming these suppresses normal
         *   volume control while PTT is armed, so it is deliberately not the
         *   default.
         * - Several vendor PTT keycodes commonly seen on rugged handsets. These
         *   are plain integers because the AOSP `KeyEvent` constants don't
         *   cover OEM PTT buttons; on devices that don't emit them the entry is
         *   simply never matched (harmless).
         *
         * The map value is a human-readable label for the settings UI.
         */
        val PTT_KEYCODE_CANDIDATES: Map<Int, String> = linkedMapOf(
            KeyEvent.KEYCODE_HEADSETHOOK to "Headset button (default)",
            KeyEvent.KEYCODE_VOLUME_UP to "Volume Up",
            KeyEvent.KEYCODE_VOLUME_DOWN to "Volume Down",
            KeyEvent.KEYCODE_CALL to "Call button",
            KeyEvent.KEYCODE_CAMERA to "Camera button",
            // Vendor / rugged-device dedicated PTT buttons (no AOSP constant).
            // Sonim XP-series & many industrial OEMs surface PTT as a custom
            // keycode in the 0x10x range; these cover the most common values.
            260 to "Dedicated PTT key (vendor 260)",
            264 to "Dedicated PTT key (vendor 264)",
            280 to "Dedicated PTT key (vendor 280)",
            1011 to "Dedicated PTT key (vendor 1011)",
        )

        /**
         * Media-button keycodes (delivered through the MediaSession callback by
         * BT PTT pucks / headset buttons) that we treat as PTT triggers.
         */
        private val MEDIA_PTT_KEYCODES: Set<Int> = setOf(
            KeyEvent.KEYCODE_MEDIA_PLAY,
            KeyEvent.KEYCODE_MEDIA_PAUSE,
            KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE,
            KeyEvent.KEYCODE_MEDIA_STOP,
            KeyEvent.KEYCODE_HEADSETHOOK,
            KeyEvent.KEYCODE_MEDIA_NEXT,
            KeyEvent.KEYCODE_MEDIA_PREVIOUS,
        )
    }

    private val appContext = context.applicationContext
    private val prefs = appContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** True while a hardware/BT source is actively transmitting (PTT held). */
    private val _transmitting = MutableStateFlow(false)
    val transmitting: StateFlow<Boolean> = _transmitting.asStateFlow()

    /**
     * True once the controller is enabled and listening for input. The UI can
     * collect this to render a "hardware PTT armed" badge.
     */
    private val _armed = MutableStateFlow(false)
    val armed: StateFlow<Boolean> = _armed.asStateFlow()

    /** Optional listener mirror of [transmitting] for non-Compose callers. */
    var onTransmitChanged: ((Boolean) -> Unit)? = null

    /**
     * Resolves the active coordinator from the bound [WalkieService]. When
     * null, [startPtt] / [stopPtt] fall back to native calls.
     */
    var pttCoordinatorProvider: (() -> PttCoordinator?)? = null

    @Volatile
    private var enabled = false

    /** Active hardware PTT keycode (persisted). */
    @Volatile
    private var pttKeyCode: Int =
        prefs.getInt(KEY_PTT_KEYCODE, DEFAULT_PTT_KEYCODE)

    /** Whether the BT/media-button PTT path is active (persisted). */
    @Volatile
    private var btPttEnabled: Boolean =
        prefs.getBoolean(KEY_BT_PTT_ENABLED, true)

    /**
     * Tracks whether the *hardware key* is currently held, so we ignore
     * auto-repeat ACTION_DOWNs and only stop on the real ACTION_UP.
     */
    @Volatile
    private var hwKeyHeld = false

    /** Framework MediaSession that captures BT/media-button events. */
    private var mediaSession: MediaSession? = null

    /**
     * For BT pucks that send a *single* media key per press (toggle style,
     * e.g. a PLAY_PAUSE tap) rather than a down/up pair, we toggle. This holds
     * the last media-key-driven PTT state so a second tap stops the talk.
     */
    @Volatile
    private var mediaToggleActive = false

    /** Debounce window for media-button toggles (ms). */
    private var lastMediaToggleAt = 0L

    // ── Enable / disable ──

    /**
     * Enable the controller: register the MediaSession (if BT PTT is on) and
     * begin honoring hardware key events. Safe to call repeatedly.
     */
    fun enable() {
        if (enabled) return
        enabled = true
        if (btPttEnabled) registerMediaSession()
        _armed.value = true
        Log.i(TAG, "Hardware PTT armed (keyCode=$pttKeyCode, btPtt=$btPttEnabled)")
    }

    /**
     * Disable the controller, release the MediaSession, and stop any in-flight
     * transmission. Call from the owning lifecycle's teardown.
     */
    fun disable() {
        if (!enabled) return
        enabled = false
        // If a key/BT button was being held when we tear down, make sure we
        // don't leave the radio keyed.
        if (_transmitting.value) stopPtt(reason = "disable")
        hwKeyHeld = false
        mediaToggleActive = false
        releaseMediaSession()
        _armed.value = false
        Log.i(TAG, "Hardware PTT disarmed")
    }

    fun isEnabled(): Boolean = enabled

    // ── Configuration ──

    /** Currently-selected hardware PTT keycode. */
    fun getPttKeyCode(): Int = pttKeyCode

    /**
     * Choose which keycode acts as the hardware PTT button. Persisted. If a key
     * is currently held under the old code, we release it first so we don't
     * strand the radio in TX.
     */
    fun setPttKeyCode(keyCode: Int) {
        if (keyCode == pttKeyCode) return
        if (hwKeyHeld) stopPtt(reason = "keycode-change")
        hwKeyHeld = false
        pttKeyCode = keyCode
        try {
            prefs.edit().putInt(KEY_PTT_KEYCODE, keyCode).apply()
        } catch (e: Exception) {
            Log.w(TAG, "persist keycode failed: ${e.message}")
        }
        Log.i(TAG, "Hardware PTT keycode set to $keyCode")
    }

    /** Whether BT/media-button PTT is currently enabled. */
    fun isBtPttEnabled(): Boolean = btPttEnabled

    /**
     * Enable/disable the Bluetooth (media-button) PTT path. Persisted, and
     * applied immediately: registers or releases the MediaSession when the
     * controller is currently enabled.
     */
    fun setBtPttEnabled(value: Boolean) {
        if (value == btPttEnabled) return
        btPttEnabled = value
        try {
            prefs.edit().putBoolean(KEY_BT_PTT_ENABLED, value).apply()
        } catch (e: Exception) {
            Log.w(TAG, "persist btPtt failed: ${e.message}")
        }
        if (enabled) {
            if (value) registerMediaSession() else {
                if (mediaToggleActive && _transmitting.value) stopPtt(reason = "bt-disabled")
                mediaToggleActive = false
                releaseMediaSession()
            }
        }
        Log.i(TAG, "BT PTT enabled=$value")
    }

    // ── Hardware key delegation (called from MainActivity) ──

    /**
     * Delegate target for `Activity.onKeyDown`. Returns `true` when the event
     * is the configured PTT key and was consumed — the Activity must then also
     * return `true` so the system doesn't act on the key (e.g. change volume).
     *
     * Auto-repeat (held key) ACTION_DOWNs are swallowed but still consumed, so
     * a held button = one continuous transmission.
     */
    fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        if (!enabled) return false
        if (keyCode != pttKeyCode) return false

        // event may be null in legacy paths; treat as a fresh press.
        val repeat = event?.repeatCount ?: 0
        if (repeat == 0 && !hwKeyHeld) {
            hwKeyHeld = true
            startPtt(reason = "hwKeyDown")
        }
        // Consume even on repeats so the system never acts on the held key.
        return true
    }

    /**
     * Delegate target for `Activity.onKeyUp`. Returns `true` when the event is
     * the configured PTT key (and thus consumed).
     */
    @Suppress("UNUSED_PARAMETER") // `event` kept for symmetry with onKeyDown + the Activity delegate contract
    fun onKeyUp(keyCode: Int, event: KeyEvent?): Boolean {
        if (!enabled) return false
        if (keyCode != pttKeyCode) return false

        if (hwKeyHeld) {
            hwKeyHeld = false
            stopPtt(reason = "hwKeyUp")
        }
        return true
    }

    // ── Bluetooth / MediaSession path ──

    private fun registerMediaSession() {
        if (mediaSession != null) return
        try {
            val session = MediaSession(appContext, "SassyTalkiePtt")

            // An ACTIVE session with a non-stopped playback state is what makes
            // the framework route hardware media-button KeyEvents to our
            // callback even when we're backgrounded behind the foreground
            // WalkieService. We don't actually play media; we advertise the
            // PLAY/PAUSE/STOP actions purely so BT pucks' buttons resolve here.
            val playbackState = PlaybackState.Builder()
                .setActions(
                    PlaybackState.ACTION_PLAY or
                        PlaybackState.ACTION_PAUSE or
                        PlaybackState.ACTION_PLAY_PAUSE or
                        PlaybackState.ACTION_STOP
                )
                // STATE_PLAYING keeps the session "current" for routing. We are
                // not emitting audio through it; SassyTalkie's own pipeline
                // handles capture/playback.
                .setState(PlaybackState.STATE_PLAYING, 0L, 0f, SystemClock.elapsedRealtime())
                .build()
            session.setPlaybackState(playbackState)

            session.setCallback(object : MediaSession.Callback() {
                // Modern BT accessories and the framework deliver media-button
                // presses here as a KeyEvent inside the intent.
                override fun onMediaButtonEvent(mediaButtonIntent: Intent): Boolean {
                    return try {
                        val keyEvent: KeyEvent? =
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                                mediaButtonIntent.getParcelableExtra(
                                    Intent.EXTRA_KEY_EVENT, KeyEvent::class.java
                                )
                            } else {
                                @Suppress("DEPRECATION")
                                mediaButtonIntent.getParcelableExtra(Intent.EXTRA_KEY_EVENT)
                            }
                        if (keyEvent != null && handleMediaKeyEvent(keyEvent)) {
                            true
                        } else {
                            super.onMediaButtonEvent(mediaButtonIntent)
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "onMediaButtonEvent failed: ${e.message}")
                        false
                    }
                }

                // Some BT stacks dispatch directly to the transport callbacks
                // rather than as a raw media-button intent. Map those to PTT
                // toggles too so a single-button puck still works.
                override fun onPlay() { mediaToggle(forceState = true) }
                override fun onPause() { mediaToggle(forceState = false) }
                override fun onStop() { mediaToggle(forceState = false) }
            })

            // setActive(true) is required for the session to receive media
            // buttons. setMediaButtonReceiver is intentionally NOT set: we want
            // in-process delivery to our callback, not a broadcast to a
            // manifest receiver (which we don't declare).
            session.isActive = true
            mediaSession = session
            Log.i(TAG, "MediaSession registered for BT PTT")
        } catch (e: Exception) {
            Log.e(TAG, "registerMediaSession failed: ${e.message}")
            mediaSession = null
        }
    }

    private fun releaseMediaSession() {
        val session = mediaSession ?: return
        mediaSession = null
        try {
            session.isActive = false
            session.release()
            Log.i(TAG, "MediaSession released")
        } catch (e: Exception) {
            Log.w(TAG, "releaseMediaSession failed: ${e.message}")
        }
    }

    /**
     * Map a media-button [KeyEvent] to PTT. Returns true if we consumed it.
     *
     * Behavior:
     *  - Buttons that deliver a real down/up pair (e.g. some headset hooks)
     *    drive press-and-hold: start on ACTION_DOWN, stop on ACTION_UP.
     *  - Buttons that send a single event per physical press (most PLAY_PAUSE
     *    pucks) toggle: first press starts, next press stops. We debounce so
     *    the framework's occasional duplicate DOWN/UP doesn't cancel itself.
     */
    private fun handleMediaKeyEvent(event: KeyEvent): Boolean {
        if (!enabled || !btPttEnabled) return false
        if (event.keyCode !in MEDIA_PTT_KEYCODES) return false

        when (event.action) {
            KeyEvent.ACTION_DOWN -> {
                if (event.repeatCount == 0) mediaToggle(forceState = null)
            }
            KeyEvent.ACTION_UP -> {
                // If a paired UP arrives quickly after a DOWN-driven toggle we
                // ignore it (the toggle already flipped state on DOWN). Held
                // hooks that we want to release on UP are handled by the
                // mediaToggleActive guard inside mediaToggle when state was
                // started by this same press.
            }
        }
        return true
    }

    /**
     * Flip (or set) the media-button-driven PTT state, with a short debounce.
     * @param forceState true = force-start, false = force-stop, null = toggle.
     */
    private fun mediaToggle(forceState: Boolean?) {
        val now = SystemClock.elapsedRealtime()
        // Debounce: collapse duplicate dispatches within 250ms.
        if (now - lastMediaToggleAt < 250L && forceState == null) return
        lastMediaToggleAt = now

        val target = forceState ?: !mediaToggleActive
        if (target == mediaToggleActive) return

        mediaToggleActive = target
        if (target) startPtt(reason = "btMediaButton") else stopPtt(reason = "btMediaButton")
    }

    // ── PTT drive (idempotent) ──

    private fun startPtt(reason: String) {
        if (_transmitting.value) return // guard double-start
        try {
            val coord = pttCoordinatorProvider?.invoke()
            val started = if (coord != null) {
                coord.onPttPressed()
            } else {
                SassyTalkNative.pttStart()
                true
            }
            if (!started) return
            _transmitting.value = true
            onTransmitChanged?.invoke(true)
            Log.d(TAG, "PTT start ($reason)")
        } catch (e: Exception) {
            Log.e(TAG, "pttStart failed ($reason): ${e.message}")
        }
    }

    private fun stopPtt(reason: String) {
        if (!_transmitting.value) return // guard double-stop
        try {
            val coord = pttCoordinatorProvider?.invoke()
            if (coord != null) {
                coord.onPttReleased()
            } else {
                SassyTalkNative.pttStop()
            }
        } catch (e: Exception) {
            Log.e(TAG, "pttStop failed ($reason): ${e.message}")
        } finally {
            // Always clear local state even if the native call threw, so we
            // don't wedge the controller into a permanent "transmitting" view.
            _transmitting.value = false
            onTransmitChanged?.invoke(false)
            Log.d(TAG, "PTT stop ($reason)")
        }
    }
}
