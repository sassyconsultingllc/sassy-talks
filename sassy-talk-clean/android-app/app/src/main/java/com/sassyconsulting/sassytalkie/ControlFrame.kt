package com.sassyconsulting.sassytalkie

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicLong
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
    private val atomicCurrent = AtomicLong(0)
    fun generate(): Long {
        var v = Random.nextLong()
        while (v == 0L) v = Random.nextLong()
        atomicCurrent.set(v)
        return v
    }
    fun current(): Long = atomicCurrent.get()
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
    /** Capability bitmap (e.g. CAP_HYBRID_PQC=0x01). 0 for peers that predate
     *  the caps byte — the heartbeat appends it, and parsing tolerates its
     *  absence, so this is a backward-compatible wire extension. */
    val caps: Int = 0,
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
    /**
     * Wake beacon. Sent by a peer about to transmit when LivenessTracker
     * marks any other peer STALE — tells receivers to immediately emit a
     * fresh heartbeat (bypassing their own outbound interval) and re-warm
     * any transport that's gone idle, so the imminent audio isn't lost on
     * the first ~1.5s of the talk burst.
     *
     * Receivers that are already HEALTHY treat OP_WAKE as a no-op (still
     * cheap — they just emit one extra HB). Receivers whose WS has fully
     * disconnected won't see this frame at all; that case needs FCM (a
     * Phase-2 follow-up). See [encodeWake] / [parseWake].
     */
    const val OP_WAKE: Byte          = 0x17

    /** Hybrid PQC handshake (path a). INIT carries the initiator message
     *  (X25519 pub || ML-KEM encaps key), RESP carries the responder message
     *  (X25519 pub || ML-KEM ciphertext). Payload: [channel:u8][message bytes].
     *  These are large (~1.2 KB) — send over a transport that carries big frames
     *  (cellular relay / RFCOMM), not a small-MTU BLE GATT write. */
    const val OP_HYBRID_INIT: Byte   = 0x1B
    const val OP_HYBRID_RESP: Byte   = 0x1C

    /** Capability bit advertised in the heartbeat caps byte: hybrid-PQC support. */
    const val CAP_HYBRID_PQC: Int    = 0x01

    fun encodeLegacy(op: Byte): ByteArray = byteArrayOf(op)

    fun encodeTlv(op: Byte, payload: ByteArray): ByteArray {
        require(payload.size <= 0xFFFF) { "payload too big" }
        val buf = ByteBuffer.allocate(3 + payload.size).order(ByteOrder.LITTLE_ENDIAN)
        buf.put(op)
        buf.putShort(payload.size.toShort())
        buf.put(payload)
        return buf.array()
    }

    fun decode(bytes: ByteArray): DecodedFrame? {
        if (bytes.isEmpty()) return null
        val op = bytes[0]
        if (op.toInt() and 0xFF < 0x10) return DecodedFrame(op, ByteArray(0))
        if (bytes.size < 3) return null
        val len = ByteBuffer.wrap(bytes, 1, 2).order(ByteOrder.LITTLE_ENDIAN).short.toInt() and 0xFFFF
        if (3 + len > bytes.size) return null  // reject truncated frames
        val payload = bytes.copyOfRange(3, 3 + len)
        return DecodedFrame(op, payload)
    }

    fun encodeHeartbeat(epoch: Long, seq: Int, tsMs: Long,
                        state: PresenceState, rttMs: Int, caps: Int = 0): ByteArray {
        // 24 bytes: the original 23 (epoch|seq|tsMs|state|rtt) + a trailing caps
        // byte. Appending keeps it readable by 23-byte-only peers (they ignore
        // the extra) while letting upgraded peers advertise capabilities.
        val p = ByteBuffer.allocate(24).order(ByteOrder.LITTLE_ENDIAN)
        p.putLong(epoch); p.putInt(seq); p.putLong(tsMs)
        p.put(state.byte); p.putShort(rttMs.toShort())
        p.put((caps and 0xFF).toByte())
        return encodeTlv(OP_HEARTBEAT, p.array())
    }

    fun parseHeartbeat(payload: ByteArray): HeartbeatPayload {
        val b = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN)
        val epoch = b.long; val seq = b.int; val tsMs = b.long
        val state = PresenceState.fromByte(b.get())
        val rttMs = b.short.toInt() and 0xFFFF
        // Caps byte is optional — older peers send a 23-byte payload without it.
        val caps = if (payload.size >= 24) payload[23].toInt() and 0xFF else 0
        return HeartbeatPayload(epoch, seq, tsMs, state, rttMs, caps)
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
        require(idBytes.size <= 255) { "peer ID too long: ${idBytes.size} bytes" }
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

    /** Hybrid handshake frame: [channel:u8][message bytes]. Used for both
     *  OP_HYBRID_INIT (initiator msg) and OP_HYBRID_RESP (responder msg). */
    fun encodeHybridFrame(op: Byte, channel: Int, msg: ByteArray): ByteArray {
        val p = ByteArray(1 + msg.size)
        p[0] = (channel and 0xFF).toByte()
        System.arraycopy(msg, 0, p, 1, msg.size)
        return encodeTlv(op, p)
    }

    /** Parse a hybrid handshake payload → (channel, messageBytes), or null. */
    fun parseHybridFrame(payload: ByteArray): Pair<Int, ByteArray>? {
        if (payload.isEmpty()) return null
        val channel = payload[0].toInt() and 0xFF
        return channel to payload.copyOfRange(1, payload.size)
    }

    /** Wake beacon payload: [epoch:u64][senderTsMs:u64]. 16 bytes. */
    fun encodeWake(epoch: Long, senderTsMs: Long): ByteArray {
        val p = ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN)
        p.putLong(epoch); p.putLong(senderTsMs)
        return encodeTlv(OP_WAKE, p.array())
    }

    /** Returns (senderEpoch, senderTsMs). */
    fun parseWake(payload: ByteArray): Pair<Long, Long> {
        val b = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN)
        return b.long to b.long
    }
}
