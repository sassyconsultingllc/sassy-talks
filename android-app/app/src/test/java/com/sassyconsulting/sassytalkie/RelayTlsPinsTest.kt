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
    fun `a renewal from any Cloudflare CA still matches`() {
        // Regression: only GTS intermediates were pinned. OkHttp's cleaned
        // chain ends in the trust anchor, so the root key is what matches.
        val leaf = "7/dpi3KALI8vMVdv7i9w+UoowzRNzhes+sWWfWkba8g=" // never pinned
        val unknownIntermediate = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="
        val anchors = mapOf(
            "Let's Encrypt X1" to "C5+lpZ7tcVwmwQIMcRtPbsQtWLABXhQzejna0wHFr8M=",
            "Let's Encrypt X2" to "diGVwiVYbubAI3RW4hB9xU8e/CH2GnkuvVFZE8zmgzI=",
            "SSL.com RSA 2022" to "K89VOmb1cJAN3TK6bf4ezAbJGC1mLcG2Dh97dnwr3VQ=",
            "SSL.com ECC 2022" to "G/ANXI8TwJTdF+AFBM8IiIUPEv0Gf6H5LA/b9guG4yE=",
            "GTS Root R4" to "mEflZT5enoR1FuXLgYYGqnVEoZvmf9c2bVBpiOjYQ0c=",
        )
        for ((ca, root) in anchors) {
            assertTrue(ca, RelayTlsPins.pinMatch(listOf(leaf, unknownIntermediate, root)))
        }
        assertFalse(RelayTlsPins.pinMatch(listOf(leaf, unknownIntermediate)))
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
