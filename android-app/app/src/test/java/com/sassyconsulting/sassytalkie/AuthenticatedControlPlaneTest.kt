// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AuthenticatedControlPlaneTest {
    private val key = ByteArray(32) { (it * 7 + 3).toByte() }

    @Test
    fun `authenticated frame round trips and binds sender`() {
        val sender = codec(sender = "device-a", epoch = 11)
        val receiver = codec(sender = "device-b", epoch = 22)
        val inner = ControlFrame.encodeRecvAck(11, 42, 1_700_000_000_000L)

        val verified = receiver.open(sender.seal(inner))!!

        assertEquals("device-a", verified.senderId)
        assertEquals(11L, verified.epoch)
        assertTrue(verified.sequence > 0)
        assertTrue(inner.contentEquals(verified.innerFrame))
    }

    @Test
    fun `forged ciphertext and wrong room fail closed`() {
        val sender = codec(sender = "device-a")
        val envelope = sender.seal(ControlFrame.encodePttStartV2(9, 1))
        envelope[envelope.lastIndex] = (envelope.last().toInt() xor 1).toByte()

        assertNull(codec(sender = "device-b").open(envelope))

        val valid = sender.seal(ControlFrame.encodePttStopV2(9, 2))
        assertNull(codec(sender = "device-b", room = "other-room").open(valid))
    }

    @Test
    fun `replay is rejected but bounded reordering is accepted`() {
        val sender = codec(sender = "device-a")
        val receiver = codec(sender = "device-b")
        val first = sender.seal(ControlFrame.encodeHeartbeat(1, 1, NOW, PresenceState.IDLE, 0))
        val second = sender.seal(ControlFrame.encodeHeartbeat(1, 2, NOW, PresenceState.IDLE, 0))

        assertTrue(receiver.open(second) != null)
        assertTrue(receiver.open(first) != null)
        assertNull(receiver.open(first))
    }

    @Test
    fun `stale and future envelopes are rejected`() {
        var senderNow = NOW - 121_000L
        val sender = AuthenticatedControlCodec(key.copyOf(), "room-1", "device-a", 11, { senderNow })
        val receiver = codec(sender = "device-b")
        assertNull(receiver.open(sender.seal(ControlFrame.encodeWake(11, senderNow))))

        senderNow = NOW + 31_000L
        assertNull(receiver.open(sender.seal(ControlFrame.encodeWake(11, senderNow))))
    }

    @Test(expected = IllegalArgumentException::class)
    fun `legacy control cannot be wrapped as privileged v2`() {
        codec(sender = "device-a").seal(ControlFrame.encodeLegacy(ControlFrame.OP_PTT_START))
    }

    private fun codec(
        sender: String,
        epoch: Long = 11,
        room: String = "room-1",
    ) = AuthenticatedControlCodec(key.copyOf(), room, sender, epoch, { NOW })

    companion object {
        private const val NOW = 1_700_000_000_000L
    }
}
