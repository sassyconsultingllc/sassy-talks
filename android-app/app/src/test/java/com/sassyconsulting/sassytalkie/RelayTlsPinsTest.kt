// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RelayTlsPinsTest {
    @Test
    fun `known intermediate matches pin-set`() {
        assertTrue(RelayTlsPins.pinMatch(listOf(RelayTlsPins.SPKI_SHA256_B64[0])))
        assertTrue(RelayTlsPins.pinMatch(listOf(RelayTlsPins.SPKI_SHA256_B64[1])))
        assertTrue(RelayTlsPins.pinMatch(listOf(RelayTlsPins.SPKI_SHA256_B64[2])))
        assertTrue(RelayTlsPins.pinMatch(listOf(RelayTlsPins.SPKI_SHA256_B64[3])))
    }

    @Test
    fun `mismatch fails closed`() {
        assertFalse(RelayTlsPins.pinMatch(listOf("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")))
        assertFalse(RelayTlsPins.pinMatch(emptyList()))
        assertFalse(RelayTlsPins.pinMatch(listOf(RelayTlsPins.SPKI_SHA256_B64[0]), emptyList()))
    }

    @Test
    fun `production default is on because backup pins exist`() {
        assertTrue(RelayTlsPins.pinsComplete())
        assertTrue(RelayTlsPins.productionDefaultEnabled)
        assertTrue(RelayTlsPins.SPKI_SHA256_B64.size >= 2)
    }
}
