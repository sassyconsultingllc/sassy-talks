// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.util.Base64
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.LinkedHashMap
import java.util.concurrent.atomic.AtomicLong
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Versioned authenticated envelope for every v2 control frame.
 *
 * The room PSK authenticates membership and the encrypted body binds the
 * protocol version, opcode, room, claimed sender, process epoch, monotonic
 * sequence and issue time. A group member can still claim another member's
 * sender id because SassyTalkie does not yet have enrolled per-device signing
 * keys; callers must not treat [VerifiedControl.senderId] as non-repudiation.
 */
internal class AuthenticatedControlCodec(
    private val key: ByteArray,
    roomId: String,
    private val senderId: String,
    private val epoch: Long,
    private val clockMs: () -> Long = System::currentTimeMillis,
    private val random: SecureRandom = SecureRandom(),
) {
    init {
        require(key.size == 32) { "control key must be 32 bytes" }
        require(senderId.toByteArray(Charsets.UTF_8).size in 1..MAX_SENDER_BYTES) {
            "sender id must be 1..$MAX_SENDER_BYTES UTF-8 bytes"
        }
    }

    private val roomBinding = MessageDigest.getInstance("SHA-256")
        .digest(roomId.toByteArray(Charsets.UTF_8))
        .copyOf(ROOM_BINDING_BYTES)
    private val txSequence = AtomicLong(0)
    private val replay = ReplayWindow()

    fun seal(innerFrame: ByteArray): ByteArray {
        val decoded = ControlFrame.decode(innerFrame)
            ?: throw IllegalArgumentException("malformed control frame")
        require((decoded.opcode.toInt() and 0xff) >= 0x10) {
            "legacy controls are not valid in the authenticated v2 envelope"
        }
        require(innerFrame.size <= MAX_INNER_BYTES) { "control frame too large" }

        val sender = senderId.toByteArray(Charsets.UTF_8)
        val seq = txSequence.incrementAndGet()
        check(seq > 0) { "control sequence exhausted" }
        val body = ByteBuffer.allocate(
            MAGIC.size + 1 + ROOM_BINDING_BYTES + 1 + sender.size + 8 + 8 + 8 + 2 + innerFrame.size,
        ).order(ByteOrder.LITTLE_ENDIAN)
            .put(MAGIC)
            .put(decoded.opcode)
            .put(roomBinding)
            .put(sender.size.toByte())
            .put(sender)
            .putLong(epoch)
            .putLong(seq)
            .putLong(clockMs())
            .putShort(innerFrame.size.toShort())
            .put(innerFrame)
            .array()

        val nonce = ByteArray(NONCE_BYTES).also(random::nextBytes)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(TAG_BITS, nonce))
        cipher.updateAAD(outerAad(decoded.opcode))
        val ciphertext = cipher.doFinal(body)
        val payload = ByteBuffer.allocate(2 + nonce.size + ciphertext.size)
            .put(VERSION)
            .put(decoded.opcode) // relay-visible wake hint; authenticated by GCM
            .put(nonce)
            .put(ciphertext)
            .array()
        return ControlFrame.encodeTlv(ControlFrame.OP_AUTHENTICATED, payload)
    }

    fun open(envelope: ByteArray): VerifiedControl? {
        val outer = ControlFrame.decode(envelope) ?: return null
        if (outer.opcode != ControlFrame.OP_AUTHENTICATED) return null
        if (outer.payload.size < 2 + NONCE_BYTES + TAG_BYTES || outer.payload[0] != VERSION) return null

        val opcodeHint = outer.payload[1]
        val nonce = outer.payload.copyOfRange(2, 2 + NONCE_BYTES)
        val ciphertext = outer.payload.copyOfRange(2 + NONCE_BYTES, outer.payload.size)
        val plaintext = try {
            Cipher.getInstance("AES/GCM/NoPadding").run {
                init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(TAG_BITS, nonce))
                updateAAD(outerAad(opcodeHint))
                doFinal(ciphertext)
            }
        } catch (_: Exception) {
            return null
        }

        val b = ByteBuffer.wrap(plaintext).order(ByteOrder.LITTLE_ENDIAN)
        if (b.remaining() < MIN_BODY_BYTES) return null
        val magic = ByteArray(MAGIC.size).also(b::get)
        if (!magic.contentEquals(MAGIC)) return null
        val boundOpcode = b.get()
        val boundRoom = ByteArray(ROOM_BINDING_BYTES).also(b::get)
        if (!MessageDigest.isEqual(boundRoom, roomBinding)) return null
        val senderLength = b.get().toInt() and 0xff
        if (senderLength !in 1..MAX_SENDER_BYTES || b.remaining() < senderLength + 8 + 8 + 8 + 2) return null
        val sender = ByteArray(senderLength).also(b::get).toString(Charsets.UTF_8)
        val senderEpoch = b.long
        val sequence = b.long
        val issuedAtMs = b.long
        val innerLength = b.short.toInt() and 0xffff
        if (innerLength == 0 || innerLength > MAX_INNER_BYTES || b.remaining() != innerLength) return null
        val inner = ByteArray(innerLength).also(b::get)
        val decoded = ControlFrame.decode(inner) ?: return null
        if (decoded.opcode != boundOpcode || decoded.opcode != opcodeHint ||
            decoded.opcode == ControlFrame.OP_AUTHENTICATED
        ) return null

        val age = clockMs() - issuedAtMs
        if (age > MAX_AGE_MS || age < -MAX_FUTURE_SKEW_MS) return null
        if (!replay.accept(sender, senderEpoch, sequence)) return null
        return VerifiedControl(sender, senderEpoch, sequence, issuedAtMs, inner)
    }

    fun destroy() {
        key.fill(0)
        replay.clear()
    }

    private class ReplayWindow {
        private data class State(var highest: Long, var bitmap: Long)
        private val states = object : LinkedHashMap<String, State>(16, 0.75f, true) {
            override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, State>?): Boolean =
                size > MAX_REPLAY_SENDERS
        }

        @Synchronized
        fun accept(sender: String, epoch: Long, sequence: Long): Boolean {
            if (epoch == 0L || sequence <= 0L) return false
            val key = "$sender\u0000$epoch"
            val state = states[key]
            if (state == null) {
                states[key] = State(sequence, 1L)
                return true
            }
            if (sequence > state.highest) {
                val shift = sequence - state.highest
                state.bitmap = if (shift >= REPLAY_BITS) 1L else (state.bitmap shl shift.toInt()) or 1L
                state.highest = sequence
                return true
            }
            val behind = state.highest - sequence
            if (behind >= REPLAY_BITS) return false
            val bit = 1L shl behind.toInt()
            if (state.bitmap and bit != 0L) return false
            state.bitmap = state.bitmap or bit
            return true
        }

        @Synchronized fun clear() = states.clear()
    }

    companion object {
        private val MAGIC = byteArrayOf('S'.code.toByte(), 'T'.code.toByte(), 'C'.code.toByte(), 'P'.code.toByte())
        private val OUTER_AAD = "sassytalkie-control-envelope-v1".toByteArray(Charsets.US_ASCII)
        private fun outerAad(opcode: Byte): ByteArray = OUTER_AAD + byteArrayOf(VERSION, opcode)
        private const val VERSION: Byte = 1
        private const val NONCE_BYTES = 12
        private const val TAG_BYTES = 16
        private const val TAG_BITS = 128
        private const val ROOM_BINDING_BYTES = 16
        private const val MAX_SENDER_BYTES = 64
        private const val MAX_INNER_BYTES = 8 * 1024
        private const val MAX_REPLAY_SENDERS = 256
        private const val REPLAY_BITS = 64L
        private const val MAX_AGE_MS = 2 * 60 * 1000L
        private const val MAX_FUTURE_SKEW_MS = 30 * 1000L
        private const val MIN_BODY_BYTES = 4 + 1 + ROOM_BINDING_BYTES + 1 + 1 + 8 + 8 + 8 + 2
    }
}

internal data class VerifiedControl(
    val senderId: String,
    val epoch: Long,
    val sequence: Long,
    val issuedAtMs: Long,
    val innerFrame: ByteArray,
)

/**
 * Process-wide control-plane key lifecycle. The key is loaded from
 * Keystore-backed session preferences and replaced/zeroized on room changes.
 */
internal object AuthenticatedControlPlane {
    private data class Context(
        val roomId: String,
        val senderId: String,
        val keyFingerprint: String,
        val epoch: Long,
    )

    private var context: Context? = null
    private var codec: AuthenticatedControlCodec? = null

    @Synchronized
    fun seal(frame: ByteArray): ByteArray? = currentCodec()?.runCatching { seal(frame) }?.getOrNull()

    @Synchronized
    fun open(frame: ByteArray): VerifiedControl? = currentCodec()?.open(frame)

    @Synchronized
    fun clear() {
        codec?.destroy()
        codec = null
        context = null
    }

    private fun currentCodec(): AuthenticatedControlCodec? {
        val app = SassyTalkNative.appContext ?: return null
        val roomId = SassyTalkNative.getSessionId()?.takeIf(String::isNotBlank) ?: return null
        val channel = runCatching { SassyTalkNative.getChannel() }.getOrDefault(1)
        val json = SassyTalkNative.getChannelSessionJson(channel)
        val keyB64 = runCatching { org.json.JSONObject(json).optString("key") }.getOrDefault("")
        val key = runCatching { Base64.decode(keyB64, Base64.DEFAULT) }.getOrNull()
            ?.takeIf { it.size == 32 } ?: return null
        val senderId = InstallId.get(app)
        val fingerprint = MessageDigest.getInstance("SHA-256").digest(key).take(8)
            .joinToString("") { "%02x".format(it) }
        val epoch = SessionEpoch.current().takeIf { it != 0L } ?: SessionEpoch.generate()
        val next = Context(roomId, senderId, fingerprint, epoch)
        if (next != context || codec == null) {
            codec?.destroy()
            codec = AuthenticatedControlCodec(key.copyOf(), roomId, senderId, epoch)
            context = next
        }
        key.fill(0)
        return codec
    }
}
