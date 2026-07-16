// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-DKEMHNHGNCEJ
package com.sassyconsulting.sassytalkie.audio

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import androidx.core.content.getSystemService

/**
 * Routes PTT received audio through the voice-call output stream so it bypasses
 * the OEM media processing chain.
 *
 * Why: Motorola's Dolby Atmos, Samsung Adapt Sound, and similar post-processing
 * stacks run on STREAM_MUSIC. They expect music-shaped content and will
 * compress / spectrally shape voice packets into mush, especially low-amplitude
 * speech. STREAM_VOICE_CALL with MODE_IN_COMMUNICATION skips that chain.
 *
 * Side effect: while engaged, system media volume controls switch to voice-call
 * volume. UI should reflect that.
 *
 * Lifecycle: call engageCommMode() when receive session opens, release() on
 * close. Saves and restores prior mode/output-device state.
 */
class AudioOutputRouter(context: Context) {

    private val am: AudioManager = context.getSystemService()
        ?: error("AudioManager unavailable")

    private var savedMode: Int = AudioManager.MODE_NORMAL
    private var savedSpeakerOn: Boolean = false
    private var savedCommDevice: AudioDeviceInfo? = null
    private var active = false

    fun engageCommMode(forceSpeaker: Boolean = true) {
        if (active) return
        savedMode = am.mode
        am.mode = AudioManager.MODE_IN_COMMUNICATION
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            // isSpeakerphoneOn is deprecated in API 31 and increasingly a no-op
            // on newer OEM builds; setCommunicationDevice is the reliable way to
            // force the built-in speaker under MODE_IN_COMMUNICATION.
            savedCommDevice = am.communicationDevice
            if (forceSpeaker) {
                am.availableCommunicationDevices
                    .firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
                    ?.let { am.setCommunicationDevice(it) }
            }
        } else {
            @Suppress("DEPRECATION")
            savedSpeakerOn = am.isSpeakerphoneOn
            @Suppress("DEPRECATION")
            am.isSpeakerphoneOn = forceSpeaker
        }
        active = true
    }

    fun release() {
        if (!active) return
        am.mode = savedMode
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val prev = savedCommDevice
            if (prev != null) am.setCommunicationDevice(prev) else am.clearCommunicationDevice()
            savedCommDevice = null
        } else {
            @Suppress("DEPRECATION")
            am.isSpeakerphoneOn = savedSpeakerOn
        }
        active = false
    }

    val isActive: Boolean get() = active
}
