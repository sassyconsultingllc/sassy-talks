// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-73N5EZNS46TB
package com.sassyconsulting.sassytalkie.audio

import android.media.MediaRecorder
import android.os.Build

/**
 * Per-device overrides for known-bad OEM audio behavior. Pipeline consults
 * DeviceQuirks.current() at start. Add to the table as field data accumulates.
 *
 * Reporting: emit Build.MANUFACTURER + Build.MODEL + Build.DEVICE + appliedState
 * to telemetry on session start so you can see which profiles users hit.
 */
object DeviceQuirks {

    data class Profile(
        val effectsConfig: AudioEffectsManager.Config,
        val outputForceCommMode: Boolean,
        val preferredAudioSource: Int? = null,
        val sourceFallbackChain: List<Int> = defaultSourceChain,
        val recordBufferMultiplier: Int = 4,
        val notes: String = "",
    )

    val defaultSourceChain = listOf(
        MediaRecorder.AudioSource.VOICE_RECOGNITION,
        MediaRecorder.AudioSource.MIC,
        MediaRecorder.AudioSource.DEFAULT,
    )

    fun current(): Profile {
        val mfr = Build.MANUFACTURER.lowercase()
        val device = Build.DEVICE.lowercase()
        return when {
            mfr.contains("motorola") || device.startsWith("moto") -> motoProfile()
            mfr.contains("samsung") -> samsungProfile()
            mfr.contains("xiaomi") || mfr.contains("redmi") -> xiaomiProfile()
            else -> defaultProfile
        }
    }

    private fun motoProfile(): Profile = Profile(
        effectsConfig = AudioEffectsManager.Config(
            enableAec = false,
            enableNs = false,
            enableAgc = false,
        ),
        outputForceCommMode = true,
        preferredAudioSource = MediaRecorder.AudioSource.MIC,
        sourceFallbackChain = listOf(
            MediaRecorder.AudioSource.MIC,
            MediaRecorder.AudioSource.VOICE_RECOGNITION,
            MediaRecorder.AudioSource.DEFAULT,
        ),
        recordBufferMultiplier = 6,
        notes = "Moto: disable all effects (HAL too aggressive). " +
                "Force comm mode output to bypass Dolby Atmos. " +
                "Prefer MIC over VOICE_RECOGNITION (modem path is flaky on G series). " +
                "Larger record buffer to absorb HAL stalls."
    )

    private fun samsungProfile(): Profile = Profile(
        effectsConfig = AudioEffectsManager.Config(
            enableAec = false,
            enableNs = false,
            enableAgc = true,
        ),
        outputForceCommMode = true,
        notes = "Samsung: NS too aggressive on quiet speakers; AGC works fine. " +
                "Force comm-mode loudspeaker on RX (earpiece is not the default)."
    )

    private fun xiaomiProfile(): Profile = Profile(
        effectsConfig = AudioEffectsManager.Config(
            enableAec = false,
            enableNs = false,
            enableAgc = true,
        ),
        outputForceCommMode = true,
        notes = "Xiaomi: MIUI sound enhance mangles voice; force comm mode output."
    )

    val defaultProfile = Profile(
        effectsConfig = AudioEffectsManager.Config(),
        outputForceCommMode = true,
        notes = "Stock defaults. Force comm-mode loudspeaker on RX."
    )
}
