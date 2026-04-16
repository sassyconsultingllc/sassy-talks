package com.sassyconsulting.sassytalkie

import java.nio.ByteBuffer
import java.nio.ByteOrder

data class AudioV2Decoded(val epoch: Long, val seq: Int, val payload: ByteArray) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is AudioV2Decoded) return false
        return epoch == other.epoch && seq == other.seq && payload.contentEquals(other.payload)
    }
    override fun hashCode(): Int = 31 * (31 * epoch.hashCode() + seq) + payload.contentHashCode()
}

object AudioFrameV2 {
    private const val HEADER_SIZE = 8 + 4 // epoch + seq

    /**
     * Encode a V2 audio frame: [length:4 LE][epoch:8 LE][seq:4 LE][payload]
     * Length field = HEADER_SIZE + payload.size
     */
    fun encode(epoch: Long, seq: Int, payload: ByteArray): ByteArray {
        val totalLen = HEADER_SIZE + payload.size
        val buf = ByteBuffer.allocate(4 + totalLen).order(ByteOrder.LITTLE_ENDIAN)
        buf.putInt(totalLen)
        buf.putLong(epoch)
        buf.putInt(seq)
        buf.put(payload)
        return buf.array()
    }

    /**
     * Decode a V2 audio frame. Returns null if too short.
     * Input is the full frame including the 4-byte length prefix.
     */
    fun decode(bytes: ByteArray): AudioV2Decoded? {
        if (bytes.size < 4 + HEADER_SIZE) return null
        val bb = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        val len = bb.int
        if (len < HEADER_SIZE || bytes.size < 4 + len) return null
        val epoch = bb.long
        val seq = bb.int
        val payloadSize = len - HEADER_SIZE
        val payload = ByteArray(payloadSize)
        if (payloadSize > 0) bb.get(payload)
        return AudioV2Decoded(epoch, seq, payload)
    }

    /**
     * Quick check: is this a V2 frame? V2 frames have length >= 12 (epoch+seq minimum).
     * V1 frames have length == actual audio payload size (typically < 12 for the length field
     * but usually audio is bigger, so we check length >= 12 AND total size matches).
     *
     * Heuristic: if length field >= 12 and the epoch bytes look valid, treat as V2.
     * For reliable detection, use the capability negotiation flag.
     */
    fun isV2(bytes: ByteArray): Boolean {
        if (bytes.size < 4 + HEADER_SIZE) return false
        val len = ByteBuffer.wrap(bytes, 0, 4).order(ByteOrder.LITTLE_ENDIAN).int
        return len >= HEADER_SIZE && bytes.size >= 4 + len
    }
}
