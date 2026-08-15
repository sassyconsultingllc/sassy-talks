// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FloorArbitrationTest {
    @Test
    fun `busy channel blocks routine local talk`() {
        assertTrue(FloorArbitration.shouldBlockLocal(floorHeld = true, localEmergency = false))
        assertFalse(FloorArbitration.shouldBlockLocal(floorHeld = false, localEmergency = false))
    }

    @Test
    fun `emergency preempts routine floor`() {
        assertFalse(FloorArbitration.shouldBlockLocal(floorHeld = true, localEmergency = true))
        assertTrue(FloorArbitration.remoteWins(1, false, 99, true))
        assertFalse(FloorArbitration.remoteWins(99, true, 1, false))
    }

    @Test
    fun `simultaneous equal-priority talk has deterministic winner`() {
        assertTrue(FloorArbitration.remoteWins(localEpoch = 20, localEmergency = false, remoteEpoch = 10, remoteEmergency = false))
        assertFalse(FloorArbitration.remoteWins(localEpoch = 10, localEmergency = false, remoteEpoch = 20, remoteEmergency = false))
    }

    @Test
    fun `equal epoch tie-break is the same on both radios`() {
        assertTrue(
            FloorArbitration.remoteWins(
                localEpoch = 7,
                localEmergency = false,
                remoteEpoch = 7,
                remoteEmergency = false,
                localPeerId = "bbb",
                remotePeerId = "aaa",
            ),
        )
        assertFalse(
            FloorArbitration.remoteWins(
                localEpoch = 7,
                localEmergency = false,
                remoteEpoch = 7,
                remoteEmergency = false,
                localPeerId = "aaa",
                remotePeerId = "bbb",
            ),
        )
    }

    @Test
    fun `floor hold outlasts the UI LED and covers jitter drain`() {
        assertTrue(FloorArbitration.STALE_HOLD_MS > FloorArbitration.UI_SPEAKING_MS)
        assertTrue(FloorArbitration.STALE_HOLD_MS > 500L)
        assertEquals(300L, FloorArbitration.DRAIN_HOLD_MS)
    }
}
