// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import android.media.AudioManager
import com.sassyconsulting.sassytalkie.RadioFocusPolicy.FocusAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RadioFocusPolicyTest {
    private val t = 1_700_000_000_000L

    @Test
    fun `a long transmission keeps RX hot while frames keep arriving`() {
        // 30 s into a burst, last frame 20 ms ago.
        assertFalse(RadioFocusPolicy.rxIdle(t, t - 20L))
    }

    @Test
    fun `RX goes idle after the stay-hot window with no frames`() {
        assertFalse(RadioFocusPolicy.rxIdle(t, t - RadioFocusPolicy.RX_STAY_HOT_MS + 1L))
        assertTrue(RadioFocusPolicy.rxIdle(t, t - RadioFocusPolicy.RX_STAY_HOT_MS))
    }

    @Test
    fun `focus is never abandoned while transmitting`() {
        assertFalse(RadioFocusPolicy.mayAbandon(rxActive = false, txActive = true))
        assertFalse(RadioFocusPolicy.mayAbandon(rxActive = true, txActive = false))
        assertTrue(RadioFocusPolicy.mayAbandon(rxActive = false, txActive = false))
    }

    @Test
    fun `a duck request lowers RX instead of ending TX`() {
        assertEquals(FocusAction.DUCK, RadioFocusPolicy.onFocusChange(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK))
        assertEquals(FocusAction.STOP_TX, RadioFocusPolicy.onFocusChange(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT))
        assertEquals(FocusAction.STOP_TX, RadioFocusPolicy.onFocusChange(AudioManager.AUDIOFOCUS_LOSS))
        assertEquals(FocusAction.REGAIN, RadioFocusPolicy.onFocusChange(AudioManager.AUDIOFOCUS_GAIN))
    }
}
