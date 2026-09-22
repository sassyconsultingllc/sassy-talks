// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TxSafetyPolicyTest {
    @Test
    fun `max TX duration is bounded`() {
        assertEquals(10_000L, TxSafetyPolicy.normalizeMaxTxMs(1L))
        assertEquals(60_000L, TxSafetyPolicy.normalizeMaxTxMs(60_000L))
        assertEquals(300_000L, TxSafetyPolicy.normalizeMaxTxMs(Long.MAX_VALUE))
    }

    @Test
    fun `transport loss stops only active TX without any data plane`() {
        assertTrue(TxSafetyPolicy.shouldForceForTransportLoss(true, false, 0))
        assertFalse(TxSafetyPolicy.shouldForceForTransportLoss(false, false, 0))
        assertFalse(TxSafetyPolicy.shouldForceForTransportLoss(true, true, 0))
        assertFalse(TxSafetyPolicy.shouldForceForTransportLoss(true, false, 1))
    }
}
