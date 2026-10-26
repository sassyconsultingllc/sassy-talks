// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.media.AudioManager

/** Pure radio audio-focus decisions for [WalkieService], shared with JVM tests. */
object RadioFocusPolicy {
    /**
     * RX stays hot this long after the last inbound frame. Longer than a
     * natural pause between quick replies (~2 s), shorter than a break in
     * conversation, so the phone-line-busy signal doesn't linger.
     */
    const val RX_STAY_HOT_MS = 5_000L

    /** RX gain multiplier while another app (navigation, a chime) asks us to duck. */
    const val DUCK_GAIN = 0.3f

    enum class FocusAction { REGAIN, DUCK, STOP_TX }

    fun onFocusChange(change: Int): FocusAction? = when (change) {
        AudioManager.AUDIOFOCUS_GAIN,
        AudioManager.AUDIOFOCUS_GAIN_TRANSIENT,
        AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK -> FocusAction.REGAIN
        // A notification chime or navigation prompt. Our speech track is not
        // auto-ducked, so lower it ourselves; ending the user's TX is wrong.
        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> FocusAction.DUCK
        AudioManager.AUDIOFOCUS_LOSS,
        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> FocusAction.STOP_TX
        else -> null
    }

    /** No remote frame for the stay-hot window: the burst is over (or the sender vanished). */
    fun rxIdle(nowMs: Long, lastRemoteFrameMs: Long): Boolean =
        nowMs - lastRemoteFrameMs >= RX_STAY_HOT_MS

    /** RX and TX share one focus request, so only drop it when neither needs it. */
    fun mayAbandon(rxActive: Boolean, txActive: Boolean): Boolean = !rxActive && !txActive
}
