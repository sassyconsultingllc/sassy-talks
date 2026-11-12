// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class L2capOfferTest {
    @Test
    fun `offer round trips including a negative nonce`() {
        val offer = L2capOffer(psm = 0x00C5, nonce = -7_000_000_000_000_000L)
        val wire = offer.encode()
        assertEquals(L2capOffer.WIRE_BYTES, wire.size)
        assertEquals(offer, L2capOffer.decode(wire))
    }

    @Test
    fun `short or missing value is not an offer`() {
        assertNull(L2capOffer.decode(null))
        assertNull(L2capOffer.decode(ByteArray(9)))
    }

    @Test
    fun `psm zero means not listening and anything else is dialable`() {
        assertFalse(L2capOffer(0, 1).dialable)
        assertFalse(L2capOffer.decode(L2capOffer(0, 1).encode())!!.dialable)
        assertTrue(L2capOffer(0x0080, 1).dialable)
        assertTrue(L2capOffer(0x00FF, 1).dialable)
        assertTrue(L2capOffer.decode(L2capOffer(0xFFFF, 1).encode())!!.dialable)
    }

    @Test
    fun `exactly one side dials for any pair of distinct nonces`() {
        val pairs = listOf(1L to 2L, -1L to 1L, Long.MIN_VALUE to Long.MAX_VALUE, -5L to -4L, 0L to -1L)
        for ((a, b) in pairs) {
            val aDials = L2capOffer.localDials(a, b)
            val bDials = L2capOffer.localDials(b, a)
            assertTrue("a=$a b=$b", aDials != bDials)
        }
    }

    @Test
    fun `nonces compare unsigned so a negative value is the larger one`() {
        assertTrue(L2capOffer.localDials(localNonce = 1L, peerNonce = -1L))
        assertFalse(L2capOffer.localDials(localNonce = -1L, peerNonce = 1L))
    }
}
