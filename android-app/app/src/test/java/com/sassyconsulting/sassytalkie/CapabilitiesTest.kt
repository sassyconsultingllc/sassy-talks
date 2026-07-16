// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-4LO4URULHFD7
package com.sassyconsulting.sassytalkie

import org.junit.Assert.*
import org.junit.Test

class CapabilitiesTest {
    @Test
    fun `capabilities encode and decode round trip`() {
        val caps = Capabilities(
            codec = "opus", sampleRate = 24000, mute = false,
            vol = 80, battery = 73, audioV2 = true, epoch = 1234L
        )
        val bytes = caps.toFrame()
        assertEquals(ControlFrame.OP_CAPABILITIES, bytes[0])

        val decoded = Capabilities.fromFrame(bytes)
        assertNotNull(decoded)
        assertEquals(caps, decoded)
    }

    @Test
    fun `parse handles missing fields with defaults`() {
        val json = """{"codec":"pcm"}""".toByteArray(Charsets.UTF_8)
        val caps = Capabilities.parse(json)
        assertEquals("pcm", caps.codec)
        assertEquals(24000, caps.sampleRate) // default
        assertFalse(caps.mute) // default
        assertEquals(100, caps.vol) // default
        assertFalse(caps.audioV2) // default
    }

    @Test
    fun `fromFrame returns null for wrong opcode`() {
        val bytes = ControlFrame.encodeLegacy(ControlFrame.OP_PTT_START)
        assertNull(Capabilities.fromFrame(bytes))
    }

    @Test
    fun `muted state round trips`() {
        val caps = Capabilities("opus", 16000, true, 0, 50, false, 999L)
        val decoded = Capabilities.fromFrame(caps.toFrame())!!
        assertTrue(decoded.mute)
        assertEquals(0, decoded.vol)
    }
}
