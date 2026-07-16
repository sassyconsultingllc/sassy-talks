// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-5RQBU2NAY2I4
package com.sassyconsulting.sassytalkie.audio

import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor

/**
 * Wraps Android's hardware audio effects on the capture chain.
 *
 * Defaults for PTT:
 *   AEC = OFF   half-duplex use, AEC introduces unnecessary processing artifacts
 *   NS  = OFF   aggressive OEM NS (especially Motorola, low-end Samsung) erases
 *               quiet voices before they reach the encoder
 *   AGC = ON    helps level out distance-to-mic variation
 *
 * The PttAudioPipeline pulls a per-device override from DeviceQuirks.current()
 * before applying. Field-known bad devices can have all three disabled.
 *
 * After apply() use the returned AppliedState to surface in telemetry; some
 * devices report isAvailable()=true but enabled stays false. Don't trust the
 * config, trust the applied state.
 */
class AudioEffectsManager(private val audioSessionId: Int) {

    data class Config(
        val enableAec: Boolean = false,
        val enableNs: Boolean = false,
        val enableAgc: Boolean = true,
    )

    data class AppliedState(
        val aecActive: Boolean,
        val nsActive: Boolean,
        val agcActive: Boolean,
    )

    private var aec: AcousticEchoCanceler? = null
    private var ns: NoiseSuppressor? = null
    private var agc: AutomaticGainControl? = null

    fun apply(config: Config): AppliedState {
        release()
        val aecActive = if (config.enableAec && AcousticEchoCanceler.isAvailable()) {
            aec = AcousticEchoCanceler.create(audioSessionId).also { it?.enabled = true }
            aec?.enabled == true
        } else false

        val nsActive = if (config.enableNs && NoiseSuppressor.isAvailable()) {
            ns = NoiseSuppressor.create(audioSessionId).also { it?.enabled = true }
            ns?.enabled == true
        } else false

        val agcActive = if (config.enableAgc && AutomaticGainControl.isAvailable()) {
            agc = AutomaticGainControl.create(audioSessionId).also { it?.enabled = true }
            agc?.enabled == true
        } else false

        return AppliedState(aecActive, nsActive, agcActive)
    }

    fun release() {
        aec?.release(); aec = null
        ns?.release(); ns = null
        agc?.release(); agc = null
    }
}
