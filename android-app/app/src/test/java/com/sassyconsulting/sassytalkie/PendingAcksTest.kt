// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import com.sassyconsulting.sassytalkie.license.PendingAcks
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The billing connection may only end after every purchase acknowledgement
 * has called back. Ending it first drops the ack, and Play refunds an
 * unacknowledged purchase after 3 days (see [PendingAcks]).
 */
class PendingAcksTest {

    @Test
    fun `nothing to acknowledge finishes at seal`() {
        var done = 0
        val acks = PendingAcks { done++ }
        acks.seal()
        assertEquals(1, done)
    }

    @Test
    fun `does not finish while an ack is still in flight`() {
        // The old refresh() ended the connection here, before Play answered.
        var done = 0
        val acks = PendingAcks { done++ }
        val first = acks.begin()
        val second = acks.begin()
        acks.seal()
        assertEquals(0, done)
        first()
        assertEquals(0, done)
        second()
        assertEquals(1, done)
    }

    @Test
    fun `acks that call back before seal still finish once at seal`() {
        var done = 0
        val acks = PendingAcks { done++ }
        acks.begin()()
        assertEquals(0, done)
        acks.seal()
        assertEquals(1, done)
    }

    @Test
    fun `a callback fired twice counts once`() {
        var done = 0
        val acks = PendingAcks { done++ }
        val first = acks.begin()
        val second = acks.begin()
        acks.seal()
        first()
        first()
        assertEquals(0, done)
        second()
        assertEquals(1, done)
    }

    @Test
    fun `timeout finishes once and late callbacks do nothing`() {
        var done = 0
        val acks = PendingAcks { done++ }
        val late = acks.begin()
        acks.seal()
        acks.forceDone()
        assertEquals(1, done)
        late()
        acks.forceDone()
        assertEquals(1, done)
    }
}
