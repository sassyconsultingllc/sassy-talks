// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RXROUTEKP01
package com.sassyconsulting.sassytalkie.audio

import android.bluetooth.BluetoothHeadset
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.sassyconsulting.sassytalkie.SassyTalkNative

/**
 * Re-asserts sticky RX loudspeaker routing after the OS (or TTS/STT) moves
 * the voice-call stream back to the earpiece. Does not steal a connected
 * Bluetooth / wired / USB headset.
 *
 * Debounced so AudioDeviceCallback storms do not flap speaker ↔ earpiece.
 */
class RxAudioRouteKeeper(private val context: Context) {

    companion object {
        private const val TAG = "RxAudioRoute"
        private const val PREFS = "sassy_settings"
        const val KEY_SPEAKERPHONE = "speakerphone_on"
        private const val DEBOUNCE_MS = 250L

        fun userWantsSpeaker(context: Context): Boolean =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getBoolean(KEY_SPEAKERPHONE, true)

        /** Apply the sticky preference through native routing. */
        fun reassert(context: Context) {
            val on = userWantsSpeaker(context)
            try {
                if (!SassyTalkNative.reassertRxRoute()) {
                    SassyTalkNative.setSpeakerphone(on)
                }
            } catch (t: Throwable) {
                Log.w(TAG, "reassert failed: ${t.message}")
            }
        }
    }

    private val app = context.applicationContext
    private val handler = Handler(Looper.getMainLooper())
    private val am: AudioManager? =
        app.getSystemService(Context.AUDIO_SERVICE) as? AudioManager

    private val debounce = Runnable { reassert(app) }

    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) {
            schedule()
        }
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) {
            schedule()
        }
    }

    private val routeReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            schedule()
        }
    }

    @Volatile private var started = false

    /**
     * Register route monitoring only while verified radio audio is active.
     * Idle standby must not continuously react to unrelated headset changes.
     */
    fun setActive(active: Boolean) {
        if (active) start() else stop()
    }

    private fun start() {
        if (started) return
        started = true
        try {
            am?.registerAudioDeviceCallback(deviceCallback, handler)
        } catch (t: Throwable) {
            Log.w(TAG, "registerAudioDeviceCallback: ${t.message}")
        }
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_HEADSET_PLUG)
            addAction(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
            addAction(AudioManager.ACTION_SCO_AUDIO_STATE_UPDATED)
            addAction(BluetoothHeadset.ACTION_CONNECTION_STATE_CHANGED)
            addAction(BluetoothHeadset.ACTION_AUDIO_STATE_CHANGED)
        }
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                app.registerReceiver(routeReceiver, filter, Context.RECEIVER_EXPORTED)
            } else {
                @Suppress("DEPRECATION")
                app.registerReceiver(routeReceiver, filter)
            }
        } catch (t: Throwable) {
            Log.w(TAG, "registerReceiver: ${t.message}")
        }
        reassert(app)
        Log.i(TAG, "started (speaker=${userWantsSpeaker(app)})")
    }

    fun stop() {
        if (!started) return
        started = false
        handler.removeCallbacks(debounce)
        try { am?.unregisterAudioDeviceCallback(deviceCallback) } catch (_: Throwable) {}
        try { app.unregisterReceiver(routeReceiver) } catch (_: Throwable) {}
    }

    private fun schedule() {
        if (!started) return
        handler.removeCallbacks(debounce)
        handler.postDelayed(debounce, DEBOUNCE_MS)
    }
}
