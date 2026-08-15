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
 * MODE_IN_COMMUNICATION defaults to the earpiece. RX therefore always pins
 * the loudspeaker unless a real BT/wired/USB headset is connected, or the
 * caller asked for earpiece.
 *
 * Side effect: while engaged, system media volume controls switch to voice-call
 * volume. UI should reflect that.
 *
 * Lifecycle: call engageCommMode() when receive session opens (safe to call
 * again to re-assert). release() on close restores prior mode/device.
 */
class AudioOutputRouter(context: Context) {

    private val am: AudioManager = context.getSystemService()
        ?: error("AudioManager unavailable")

    private var savedMode: Int = AudioManager.MODE_NORMAL
    private var savedSpeakerOn: Boolean = false
    private var savedCommDevice: AudioDeviceInfo? = null
    private var active = false

    fun engageCommMode(forceSpeaker: Boolean = true) {
        val first = !active
        if (first) {
            savedMode = am.mode
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                savedCommDevice = am.communicationDevice
            } else {
                @Suppress("DEPRECATION")
                savedSpeakerOn = am.isSpeakerphoneOn
            }
        }
        am.mode = AudioManager.MODE_IN_COMMUNICATION
        applyTarget(forceSpeaker)
        active = true
    }

    /** Re-apply the sticky sink without resetting saved restore state. */
    fun reassert(forceSpeaker: Boolean = true) {
        if (!active) {
            engageCommMode(forceSpeaker)
            return
        }
        am.mode = AudioManager.MODE_IN_COMMUNICATION
        applyTarget(forceSpeaker)
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

    private fun applyTarget(forceSpeaker: Boolean) {
        val types = outputTypes()
        val target = RxOutputPolicy.resolve(forceSpeaker, types)
        val current = currentType()
        if (!RxOutputPolicy.shouldApply(target, current)) return
        when (target) {
            RxOutputPolicy.Target.LOUDSPEAKER -> pinBuiltin(speaker = true)
            RxOutputPolicy.Target.EARPIECE -> pinBuiltin(speaker = false)
            RxOutputPolicy.Target.EXTERNAL -> clearBuiltinPin()
        }
    }

    private fun outputTypes(): IntArray =
        am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).map { it.type }.toIntArray()

    private fun currentType(): Int? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            return am.communicationDevice?.type
        }
        @Suppress("DEPRECATION")
        return if (am.isSpeakerphoneOn) {
            RxOutputPolicy.TYPE_BUILTIN_SPEAKER
        } else {
            RxOutputPolicy.TYPE_BUILTIN_EARPIECE
        }
    }

    private fun pinBuiltin(speaker: Boolean) {
        val wanted = if (speaker) {
            intArrayOf(
                RxOutputPolicy.TYPE_BUILTIN_SPEAKER,
                RxOutputPolicy.TYPE_BUILTIN_SPEAKER_SAFE,
            )
        } else {
            intArrayOf(RxOutputPolicy.TYPE_BUILTIN_EARPIECE)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val device = am.availableCommunicationDevices.firstOrNull { it.type in wanted }
                ?: am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).firstOrNull { it.type in wanted }
            if (device != null) {
                am.setCommunicationDevice(device)
            }
        }
        @Suppress("DEPRECATION")
        am.isSpeakerphoneOn = speaker
    }

    private fun clearBuiltinPin() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            am.clearCommunicationDevice()
        }
        @Suppress("DEPRECATION")
        am.isSpeakerphoneOn = false
    }
}
