// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-QZNA2Z2WYKWL
package com.sassyconsulting.sassytalkie

import org.junit.Assert.*
import org.junit.Test

class ControlFrameTest {
    @Test
    fun `legacy single byte opcode round trips`() {
        val encoded = ControlFrame.encodeLegacy(ControlFrame.OP_PTT_START)
        assertArrayEquals(byteArrayOf(0x01), encoded)
        val decoded = ControlFrame.decode(encoded)!!
        assertEquals(ControlFrame.OP_PTT_START, decoded.opcode)
        assertEquals(0, decoded.payload.size)
    }

    @Test
    fun `heartbeat tlv round trips`() {
        val hb = ControlFrame.encodeHeartbeat(
            epoch = 0x1234567890ABCDEFL,
            seq = 42,
            tsMs = 1_700_000_000_000L,
            state = PresenceState.LISTENING,
            rttMs = 18,
        )
        assertEquals(0x10, hb[0].toInt())
        val decoded = ControlFrame.decode(hb)!!
        assertEquals(ControlFrame.OP_HEARTBEAT, decoded.opcode)
        val parsed = ControlFrame.parseHeartbeat(decoded.payload)
        assertEquals(42, parsed.seq)
        assertEquals(PresenceState.LISTENING, parsed.state)
        assertEquals(18, parsed.rttMs)
        assertEquals(0x1234567890ABCDEFL, parsed.epoch)
    }

    @Test
    fun `recv ack round trips`() {
        val bytes = ControlFrame.encodeRecvAck(999L, 77, 5000L)
        val decoded = ControlFrame.decode(bytes)!!
        assertEquals(ControlFrame.OP_RECV_ACK, decoded.opcode)
        val (epoch, seq, ts) = ControlFrame.parseRecvAck(decoded.payload)
        assertEquals(999L, epoch)
        assertEquals(77, seq)
        assertEquals(5000L, ts)
    }

    @Test
    fun `eot ack round trips`() {
        val bytes = ControlFrame.encodeEotAck(888L, 55)
        val decoded = ControlFrame.decode(bytes)!!
        assertEquals(ControlFrame.OP_EOT_ACK, decoded.opcode)
    }

    @Test
    fun `partner offline round trips`() {
        val bytes = ControlFrame.encodePartnerOffline("alice-uuid-1234")
        val decoded = ControlFrame.decode(bytes)!!
        assertEquals(ControlFrame.OP_PARTNER_OFFLINE, decoded.opcode)
        val len = decoded.payload[0].toInt() and 0xFF
        val id = String(decoded.payload, 1, len, Charsets.UTF_8)
        assertEquals("alice-uuid-1234", id)
    }

    @Test
    fun `ptt v2 start and stop round trip`() {
        val start = ControlFrame.encodePttStartV2(123L, 1)
        val stop = ControlFrame.encodePttStopV2(123L, 50)
        assertEquals(ControlFrame.OP_PTT_START_V2, ControlFrame.decode(start)!!.opcode)
        assertEquals(ControlFrame.OP_PTT_STOP_V2, ControlFrame.decode(stop)!!.opcode)
    }

    @Test
    fun `epoch generator is stable per instance and unique across instances`() {
        val a = SessionEpoch.generate()
        val b = SessionEpoch.generate()
        assertNotEquals(a, b)
        assertTrue(a != 0L)
        assertTrue(b != 0L)
    }

    @Test
    fun `empty bytes decode returns null`() {
        assertNull(ControlFrame.decode(ByteArray(0)))
    }

    @Test
    fun `truncated TLV frame returns null`() {
        // Build a TLV header claiming 100 bytes but only provide 5
        val bytes = byteArrayOf(0x10, 100, 0, 1, 2)
        assertNull(ControlFrame.decode(bytes))
    }

    @Test
    fun `short TLV header returns null`() {
        // Only opcode, no length bytes
        assertNull(ControlFrame.decode(byteArrayOf(0x10)))
        assertNull(ControlFrame.decode(byteArrayOf(0x10, 0x00)))
    }
}
