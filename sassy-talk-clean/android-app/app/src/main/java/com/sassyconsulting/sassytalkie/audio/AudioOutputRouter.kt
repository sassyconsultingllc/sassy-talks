package com.sassyconsulting.sassytalkie.audio

import android.content.Context
import android.media.AudioManager
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
 * close. Saves and restores prior mode/speaker state.
 */
class AudioOutputRouter(context: Context) {

    private val am: AudioManager = context.getSystemService()
        ?: error("AudioManager unavailable")

    private var savedMode: Int = AudioManager.MODE_NORMAL
    private var savedSpeakerOn: Boolean = false
    private var active = false

    fun engageCommMode(forceSpeaker: Boolean = true) {
        if (active) return
        savedMode = am.mode
        savedSpeakerOn = am.isSpeakerphoneOn
        am.mode = AudioManager.MODE_IN_COMMUNICATION
        am.isSpeakerphoneOn = forceSpeaker
        active = true
    }

    fun release() {
        if (!active) return
        am.mode = savedMode
        am.isSpeakerphoneOn = savedSpeakerOn
        active = false
    }

    val isActive: Boolean get() = active
}
