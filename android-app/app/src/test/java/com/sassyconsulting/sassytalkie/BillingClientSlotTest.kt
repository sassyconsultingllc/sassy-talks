// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import com.sassyconsulting.sassytalkie.license.BillingClientSlot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The paywall must never start a billing client it has already ended.
 *
 * Play Billing's endConnection() is terminal, and the gate used to end and
 * restart one client on every attempt, so the catalog never loaded (see
 * [BillingClientSlot]). The fake below fails the same way the real client
 * does: start() after end() is an error.
 */
class BillingClientSlotTest {

    private class FakeClient(val id: Int) {
        var ended = 0
        var started = false
        fun start() {
            check(ended == 0) { "Client was already closed and can't be reused" }
            started = true
        }
    }

    @Test
    fun `every attempt gets a client that can still start`() {
        var next = 0
        val slot = BillingClientSlot<FakeClient> { it.ended++ }
        val first = slot.fresh { FakeClient(next++) }
        first.start()
        // Retry: the old shape called end() then start() on `first` and threw here.
        val second = slot.fresh { FakeClient(next++) }
        second.start()
        assertNotSame(first, second)
        assertTrue(second.started)
        assertSame(second, slot.current)
    }

    @Test
    fun `the previous client is ended exactly once`() {
        val slot = BillingClientSlot<FakeClient> { it.ended++ }
        val first = slot.fresh { FakeClient(1) }
        val second = slot.fresh { FakeClient(2) }
        assertEquals(1, first.ended)
        assertEquals(0, second.ended)
        slot.close()
        slot.close()
        assertEquals(1, second.ended)
        assertNull(slot.current)
    }

    @Test
    fun `a throwing end does not block the next attempt`() {
        val slot = BillingClientSlot<FakeClient> { throw IllegalStateException("unbind failed") }
        slot.fresh { FakeClient(1) }
        val second = slot.fresh { FakeClient(2) }
        assertSame(second, slot.current)
        assertFalse(second.started)
        second.start()
        assertTrue(second.started)
    }
}
