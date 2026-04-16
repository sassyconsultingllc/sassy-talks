package com.sassyconsulting.sassytalkie

import org.junit.Assert.*
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class AudioFrameV2Test {
    @Test
    fun `v2 encodes length epoch seq and payload`() {
        val payload = byteArrayOf(1, 2, 3, 4, 5)
        val f = AudioFrameV2.encode(epoch = 0xAA11L, seq = 5, payload = payload)
        // length field = 8 + 4 + 5 = 17
        val len = ByteBuffer.wrap(f, 0, 4).order(ByteOrder.LITTLE_ENDIAN).int
        assertEquals(17, len)
        // Total size = 4 + 17 = 21
        assertEquals(21, f.size)
    }

    @Test
    fun `round trip encode decode`() {
        val original = byteArrayOf(10, 20, 30)
        val encoded = AudioFrameV2.encode(epoch = 0xAA11L, seq = 5, payload = original)
        val decoded = AudioFrameV2.decode(encoded)!!
        assertEquals(0xAA11L, decoded.epoch)
        assertEquals(5, decoded.seq)
        assertArrayEquals(original, decoded.payload)
    }

    @Test
    fun `decode returns null for too-short input`() {
        assertNull(AudioFrameV2.decode(ByteArray(10)))
    }

    @Test
    fun `empty payload round trips`() {
        val encoded = AudioFrameV2.encode(epoch = 1L, seq = 0, payload = ByteArray(0))
        val decoded = AudioFrameV2.decode(encoded)!!
        assertEquals(1L, decoded.epoch)
        assertEquals(0, decoded.seq)
        assertEquals(0, decoded.payload.size)
    }

    @Test
    fun `large seq number round trips`() {
        val encoded = AudioFrameV2.encode(epoch = Long.MAX_VALUE, seq = Int.MAX_VALUE,
            payload = byteArrayOf(0xFF.toByte()))
        val decoded = AudioFrameV2.decode(encoded)!!
        assertEquals(Long.MAX_VALUE, decoded.epoch)
        assertEquals(Int.MAX_VALUE, decoded.seq)
    }

    @Test
    fun `isV2 detects v2 frames`() {
        val v2 = AudioFrameV2.encode(1L, 1, byteArrayOf(1, 2, 3))
        assertTrue(AudioFrameV2.isV2(v2))
    }

    @Test
    fun `isV2 rejects short frames`() {
        assertFalse(AudioFrameV2.isV2(ByteArray(5)))
    }
}
