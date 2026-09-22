// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import com.sassyconsulting.sassytalkie.license.TrialGate
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * When the radio screen warns that the trial is running out.
 *
 * The decision is a pure function on purpose. Its real inputs — remaining
 * sessions and entitlement — are both environment-dependent, and entitlement is
 * unconditionally true in debug builds, so a test that went through the live
 * environment would assert nothing at all. That trap already cost one round of
 * red tests on this feature; this is the shape that avoids it.
 */
class TrialWarningTest {

    @Test
    fun `no warning at the start of the trial`() {
        // Warning from session one would be nagging, not informing.
        assertFalse(TrialGate.warnThreshold(remaining = 5, entitled = false))
        assertFalse(TrialGate.warnThreshold(remaining = 4, entitled = false))
        assertFalse(TrialGate.warnThreshold(remaining = 3, entitled = false))
    }

    @Test
    fun `warns for the last two sessions`() {
        assertTrue(TrialGate.warnThreshold(remaining = 2, entitled = false))
        assertTrue(TrialGate.warnThreshold(remaining = 1, entitled = false))
    }

    /**
     * At zero the user is already at the paywall — a "0 free sessions left"
     * banner on the radio screen would mean the gate failed to fire.
     */
    @Test
    fun `no warning once the trial is spent`() {
        assertFalse(TrialGate.warnThreshold(remaining = 0, entitled = false))
    }

    /** A paying user has nothing to run out of. */
    @Test
    fun `never warns an entitled user`() {
        for (r in 0..5) {
            assertFalse("remaining=$r", TrialGate.warnThreshold(remaining = r, entitled = true))
        }
    }

    /** Defensive: a corrupt or negative count must not produce a banner. */
    @Test
    fun `negative remaining does not warn`() {
        assertFalse(TrialGate.warnThreshold(remaining = -1, entitled = false))
    }
}
