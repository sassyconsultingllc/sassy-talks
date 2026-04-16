package com.sassyconsulting.sassytalkie

import android.app.NotificationManager
import android.content.Context
import android.media.AudioManager
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.flow.MutableStateFlow

class PresenceSensor(private val ctx: Context) {
    private val audio = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val notif = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    val state = MutableStateFlow(PresenceState.LISTENING)
    var micMuted: Boolean = false

    fun refresh() {
        val lifecycle = try {
            ProcessLifecycleOwner.get().lifecycle.currentState
        } catch (_: Exception) {
            Lifecycle.State.RESUMED
        }
        val dnd = try {
            notif.currentInterruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL
        } catch (_: Exception) { false }
        val outputZero = try {
            audio.getStreamVolume(AudioManager.STREAM_VOICE_CALL) == 0
        } catch (_: Exception) { false }
        val isBackground = lifecycle.isAtLeast(Lifecycle.State.STARTED) &&
                           !lifecycle.isAtLeast(Lifecycle.State.RESUMED)

        state.value = when {
            micMuted     -> PresenceState.MUTED
            dnd          -> PresenceState.DND
            isBackground -> PresenceState.BACKGROUNDED
            outputZero   -> PresenceState.AWAY
            else         -> PresenceState.LISTENING
        }
    }
}
