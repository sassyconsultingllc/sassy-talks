package com.sassyconsulting.sassytalkie

import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.random.Random

enum class PresenceState(val byte: Byte) {
    IDLE(0), LISTENING(1), SPEAKING(2), MUTED(3),
    AWAY(4), BACKGROUNDED(5), DND(6);
    companion object {
        fun fromByte(b: Byte): PresenceState =
            values().firstOrNull { it.byte == b } ?: IDLE
    }
}

object SessionEpoch {
    @Volatile private var current: Long = 0
    fun generate(): Long {
        var v = Random.nextLong()
        while (v == 0L) v = Random.nextLong()
        current = v
        return v
    }
    fun current(): Long = current
}

data class DecodedFrame(val opcode: Byte, val payload: ByteArray) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is DecodedFrame) return false
        return opcode == other.opcode && payload.contentEquals(other.payload)
    }
    override fun hashCode(): Int = 31 * opcode.hashCode() + payload.contentHashCode()
}

data class HeartbeatPayload(
    val epoch: Long, val seq: Int, val tsMs: Long,
    val state: PresenceState, val rttMs: Int,
)

object ControlFrame {
    const val OP_PTT_START: Byte     = 0x01
    const val OP_PTT_STOP: Byte      = 0x02
    const val OP_READY_ACK: Byte     = 0x03
    const val OP_PING: Byte          = 0x04
    const val OP_CHANNEL_SYNC: Byte  = 0x05
    const val OP_HEARTBEAT: Byte     = 0x10
    const val OP_RECV_ACK: Byte      = 0x11
    const val OP_EOT_ACK: Byte       = 0x12
    const val OP_CAPABILITIES: Byte  = 0x13
    const val OP_PARTNER_OFFLINE: Byte = 0x14
    const val OP_PTT_START_V2: Byte  = 0x15
    const val OP_PTT_STOP_V2: Byte   = 0x16

    fun encodeLegacy(op: Byte): ByteArray = byteArrayOf(op)

    fun encodeTlv(op: Byte, payload: ByteArray): ByteArray {
        require(payload.size <= 0xFFFF) { "payload too big" }
        val buf = ByteBuffer.allocate(3 + payload.size).order(ByteOrder.LITTLE_ENDIAN)
        buf.put(op)
        buf.putShort(payload.size.toShort())
        buf.put(payload)
        return buf.array()
    }

    fun decode(bytes: ByteArray): DecodedFrame {
        if (bytes.isEmpty()) return DecodedFrame(0, ByteArray(0))
        val op = bytes[0]
        if (op.toInt() and 0xFF < 0x10) return DecodedFrame(op, ByteArray(0))
        if (bytes.size < 3) return DecodedFrame(op, ByteArray(0))
        val len = ByteBuffer.wrap(bytes, 1, 2).order(ByteOrder.LITTLE_ENDIAN).short.toInt() and 0xFFFF
        val payload = bytes.copyOfRange(3, minOf(3 + len, bytes.size))
        return DecodedFrame(op, payload)
    }

    fun encodeHeartbeat(epoch: Long, seq: Int, tsMs: Long,
                        state: PresenceState, rttMs: Int): ByteArray {
        val p = ByteBuffer.allocate(23).order(ByteOrder.LITTLE_ENDIAN)
        p.putLong(epoch); p.putInt(seq); p.putLong(tsMs)
        p.put(state.byte); p.putShort(rttMs.toShort())
        return encodeTlv(OP_HEARTBEAT, p.array())
    }

    fun parseHeartbeat(payload: ByteArray): HeartbeatPayload {
        val b = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN)
        return HeartbeatPayload(
            epoch = b.long, seq = b.int, tsMs = b.long,
            state = PresenceState.fromByte(b.get()),
            rttMs = b.short.toInt() and 0xFFFF,
        )
    }

    fun encodeRecvAck(epoch: Long, lastSeq: Int, tsMs: Long): ByteArray {
        val p = ByteBuffer.allocate(20).order(ByteOrder.LITTLE_ENDIAN)
        p.putLong(epoch); p.putInt(lastSeq); p.putLong(tsMs)
        return encodeTlv(OP_RECV_ACK, p.array())
    }

    fun parseRecvAck(payload: ByteArray): Triple<Long, Int, Long> {
        val b = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN)
        return Triple(b.long, b.int, b.long)
    }

    fun encodeEotAck(epoch: Long, upToSeq: Int): ByteArray {
        val p = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN)
        p.putLong(epoch); p.putInt(upToSeq)
        return encodeTlv(OP_EOT_ACK, p.array())
    }

    fun encodePartnerOffline(peerId: String): ByteArray {
        val idBytes = peerId.toByteArray(Charsets.UTF_8)
        val p = ByteArray(1 + idBytes.size)
        p[0] = idBytes.size.toByte()
        System.arraycopy(idBytes, 0, p, 1, idBytes.size)
        return encodeTlv(OP_PARTNER_OFFLINE, p)
    }

    fun encodePttStartV2(epoch: Long, startSeq: Int): ByteArray {
        val p = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN)
        p.putLong(epoch); p.putInt(startSeq)
        return encodeTlv(OP_PTT_START_V2, p.array())
    }

    fun encodePttStopV2(epoch: Long, endSeq: Int): ByteArray {
        val p = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN)
        p.putLong(epoch); p.putInt(endSeq)
        return encodeTlv(OP_PTT_STOP_V2, p.array())
    }
}
