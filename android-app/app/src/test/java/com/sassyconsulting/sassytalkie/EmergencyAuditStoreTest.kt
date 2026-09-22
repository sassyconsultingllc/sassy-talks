// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class EmergencyAuditStoreTest {
    @Test
    fun `disclaimer is explicit`() {
        assertTrue(EmergencyAuditStore.DISCLAIMER.contains("not a legal chain of custody"))
        assertTrue(EmergencyAuditStore.DISCLAIMER.contains("not court-certified evidence"))
    }

    @Test
    fun `redact strips hex secrets and psk tokens`() {
        val d = EmergencyAuditStore.redact("reason=ok psk=0123456789abcdef0123456789abcdef key=AAAA")
        assertTrue(d.contains("[redacted]"))
        assertFalse(d.contains("0123456789abcdef0123456789abcdef"))
    }

    @Test
    fun `utc is zulu`() {
        assertEquals("1970-01-01T00:00:00.000Z", EmergencyAuditStore.utc(0L))
        assertTrue(EmergencyAuditStore.utc(1_700_000_000_000L).endsWith("Z"))
    }

    @Test
    fun `event hash is stable`() {
        val h = EmergencyAuditStore.sha256("a|1|tx_requested|ok")
        assertEquals(64, h.length)
        assertEquals(h, EmergencyAuditStore.sha256("a|1|tx_requested|ok"))
    }
}
