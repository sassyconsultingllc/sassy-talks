// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RLYCATCHTST9
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class RelayCatchupTest {
    private val t = 1_700_000_000_000L

    @Test
    fun neverConnectedAsksForNothing() {
        assertNull(RelayCatchup.sinceMs(0L, t))
    }

    @Test
    fun shortDropReplaysOnlyTheGap() {
        assertEquals(t - 2_250L, RelayCatchup.sinceMs(t - 2_000L, t))
    }

    @Test
    fun longGapGetsNoLateBurst() {
        assertEquals(t - RelayCatchup.MAX_GAP_MS - 250L, RelayCatchup.sinceMs(t - RelayCatchup.MAX_GAP_MS, t))
        assertNull(RelayCatchup.sinceMs(t - RelayCatchup.MAX_GAP_MS - 1L, t))
    }

    /** Same answers as core::protocol::catchup_since_ms (desktop / iOS). */
    @Test
    fun matchesCoreForBackwardsClockStep() {
        assertEquals(t + 250L, RelayCatchup.sinceMs(t + 500L, t))
    }
}
