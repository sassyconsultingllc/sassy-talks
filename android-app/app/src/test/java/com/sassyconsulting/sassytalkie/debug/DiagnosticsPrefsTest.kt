// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie.debug

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticsPrefsTest {
    @Test
    fun `release hides HUD unless diagnostics are on and radio UI is showing`() {
        assertFalse(DiagnosticsPrefs.shouldShowOverlay(false, false, true))
        assertFalse(DiagnosticsPrefs.shouldShowOverlay(true, true, false))
        assertTrue(DiagnosticsPrefs.shouldShowOverlay(true, false, true))
        assertTrue(DiagnosticsPrefs.shouldShowOverlay(false, true, true))
        assertFalse(DiagnosticsPrefs.shouldShowOverlay(true, true, true, false))
    }
}
