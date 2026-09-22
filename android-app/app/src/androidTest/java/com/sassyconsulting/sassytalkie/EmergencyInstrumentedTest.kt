// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * End-to-end emergency-beacon tests against the REAL native library.
 *
 * These run on-device because the whole point is the JNI boundary and the
 * AEAD sealing — a JVM unit test would exercise neither. Between them they
 * cover the full life-safety path short of the radio itself: raise → wire
 * frame → decode, sealed and unsealed, plus stand-down and cadence.
 */
@RunWith(AndroidJUnit4::class)
class EmergencyInstrumentedTest {

    private fun opcodeOf(frame: ByteArray): Byte = frame[0]

    /** TLV length field is u16 LE at bytes 1..2; total frame is 3 + len. */
    private fun declaredLen(frame: ByteArray): Int =
        (frame[1].toInt() and 0xFF) or ((frame[2].toInt() and 0xFF) shl 8)

    @Before
    fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        SassyTalkNative.appContext = ctx
        SassyTalkNative.init()
        SassyTalkNative.clearSession()
        // Leave no beacon running between tests.
        SassyTalkNative.emergencyClear()
    }

    // ── Framing ─────────────────────────────────────────────────────────────

    @Test
    fun sos_activate_emits_a_wellformed_emergency_frame() {
        val frame = SassyTalkNative.emergencyActivate(
            kind = SassyTalkNative.EMERGENCY_KIND_SOS,
            note = "leg pinned",
        )
        assertNotNull("activate must return a frame", frame)
        frame!!
        assertEquals("opcode must be OP_EMERGENCY", ControlFrame.OP_EMERGENCY, opcodeOf(frame))
        assertEquals("TLV length must match payload", frame.size - 3, declaredLen(frame))
        assertTrue(SassyTalkNative.emergencyIsActive())
    }

    @Test
    fun mandown_activate_uses_the_mandown_opcode() {
        val frame = SassyTalkNative.emergencyActivate(
            kind = SassyTalkNative.EMERGENCY_KIND_MANDOWN,
        )
        assertNotNull(frame)
        assertEquals(ControlFrame.OP_MANDOWN, opcodeOf(frame!!))
    }

    /**
     * The collision regression, checked against real emitted bytes rather than
     * constants: a man-down beacon must never carry the hybrid-PQC INIT opcode,
     * and a stand-down must never carry HYBRID_RESP. This is the failure that
     * would have shipped a life-safety frame into the key-exchange handler.
     */
    @Test
    fun emergency_frames_never_carry_hybrid_pqc_opcodes() {
        val sos = SassyTalkNative.emergencyActivate(SassyTalkNative.EMERGENCY_KIND_SOS)!!
        val mandown = SassyTalkNative.emergencyActivate(SassyTalkNative.EMERGENCY_KIND_MANDOWN)!!
        val clear = SassyTalkNative.emergencyClear()!!
        for (f in listOf(sos, mandown, clear)) {
            assertFalse(
                "0x${opcodeOf(f).toString(16)} collides with hybrid PQC",
                opcodeOf(f) == ControlFrame.OP_HYBRID_INIT ||
                    opcodeOf(f) == ControlFrame.OP_HYBRID_RESP,
            )
        }
    }

    // ── Round trip ──────────────────────────────────────────────────────────

    @Test
    fun activate_then_decode_round_trips_sender_and_kind() {
        val frame = SassyTalkNative.emergencyActivate(
            kind = SassyTalkNative.EMERGENCY_KIND_SOS,
            note = "help",
        )!!
        val json = SassyTalkNative.emergencyDecode(frame)
        assertNotNull("emitted frame must decode", json)
        val o = JSONObject(json!!)
        assertEquals("emergency", o.getString("op"))
        assertEquals("sos", o.getString("kind"))
        assertEquals("help", o.optString("note"))
        assertTrue("sender must be populated", o.getString("sender").isNotEmpty())
    }

    @Test
    fun clear_emits_a_standdown_that_decodes_as_clear() {
        SassyTalkNative.emergencyActivate(SassyTalkNative.EMERGENCY_KIND_SOS)
        assertTrue(SassyTalkNative.emergencyIsActive())

        val clear = SassyTalkNative.emergencyClear()
        assertNotNull("clear must return a stand-down frame", clear)
        assertEquals(ControlFrame.OP_EMERGENCY_CLEAR, opcodeOf(clear!!))

        val o = JSONObject(SassyTalkNative.emergencyDecode(clear)!!)
        assertEquals("clear", o.getString("op"))
        assertFalse("beacon must stop after clear", SassyTalkNative.emergencyIsActive())
    }

    @Test
    fun clear_without_an_active_emergency_returns_null() {
        assertFalse(SassyTalkNative.emergencyIsActive())
        assertNull(SassyTalkNative.emergencyClear())
    }

    // ── Privacy: coordinates must never ride unsealed ────────────────────────

    /**
     * With no session key the beacon still goes out — a distress call is not
     * gated on key state — but WITHOUT coordinates, because control frames
     * reach the relay in the clear and a GPS fix there is a live location
     * feed for whoever is in trouble.
     */
    @Test
    fun coordinates_are_dropped_when_there_is_no_session_key() {
        SassyTalkNative.clearSession()
        assertFalse("precondition: no crypto session", SassyTalkNative.isEncrypted())

        val frame = SassyTalkNative.emergencyActivate(
            kind = SassyTalkNative.EMERGENCY_KIND_SOS,
            hasCoord = true,
            latE7 = 377_749_000,
            lonE7 = -1_224_194_000,
        )!!
        val o = JSONObject(SassyTalkNative.emergencyDecode(frame)!!)
        assertFalse("unsealed beacon must not carry lat", o.has("lat"))
        assertFalse("unsealed beacon must not carry lon", o.has("lon"))
        assertFalse("frame should report itself unsealed", o.getBoolean("sealed"))
    }

    /**
     * With a session established the payload is AEAD-sealed, so coordinates
     * are safe to attach and the relay sees only ciphertext under the TLV
     * header. Verified structurally: the sender id must NOT appear as
     * cleartext anywhere in the sealed frame.
     */
    @Test
    fun coordinates_are_attached_and_payload_is_sealed_when_a_session_exists() {
        val qr = SassyTalkNative.generateChannelQR(1, 24, "EmergencyTest")
        assertTrue("precondition: session established", qr.isNotEmpty())
        assertTrue("precondition: crypto active", SassyTalkNative.isEncrypted())

        val frame = SassyTalkNative.emergencyActivate(
            kind = SassyTalkNative.EMERGENCY_KIND_SOS,
            hasCoord = true,
            latE7 = 377_749_000,
            lonE7 = -1_224_194_000,
        )!!
        val o = JSONObject(SassyTalkNative.emergencyDecode(frame)!!)
        assertTrue("sealed beacon should report sealed", o.getBoolean("sealed"))
        assertTrue("sealed beacon should carry lat", o.has("lat"))
        assertEquals(37.7749, o.getDouble("lat"), 0.0001)
        assertEquals(-122.4194, o.getDouble("lon"), 0.0001)

        val sender = o.getString("sender")
        val onWire = String(frame, Charsets.ISO_8859_1)
        assertFalse(
            "sender id must not appear in cleartext in a sealed frame",
            onWire.contains(sender),
        )
    }

    // ── Hostile input ───────────────────────────────────────────────────────

    @Test
    fun decode_rejects_garbage_without_crashing() {
        assertNull(SassyTalkNative.emergencyDecode(ByteArray(0)))
        assertNull(SassyTalkNative.emergencyDecode(byteArrayOf(0x00)))
        // Right opcode, lying length field.
        assertNull(
            SassyTalkNative.emergencyDecode(
                byteArrayOf(ControlFrame.OP_EMERGENCY, 0xFF.toByte(), 0xFF.toByte(), 0x01),
            )
        )
        // A non-emergency opcode must be ignored by this decoder entirely.
        assertNull(
            SassyTalkNative.emergencyDecode(
                byteArrayOf(ControlFrame.OP_HEARTBEAT, 0x01, 0x00, 0x01),
            )
        )
    }

    @Test
    fun truncated_emergency_frame_is_rejected() {
        val frame = SassyTalkNative.emergencyActivate(SassyTalkNative.EMERGENCY_KIND_SOS)!!
        val truncated = frame.copyOfRange(0, frame.size - 4)
        assertNull(SassyTalkNative.emergencyDecode(truncated))
    }

    // ── Beacon cadence ──────────────────────────────────────────────────────

    /**
     * The 5s re-broadcast cadence is core-owned; the Kotlin loop polls faster
     * and lets native decide when a frame is due, so the two cannot drift.
     */
    @Test
    fun tick_rebroadcasts_only_after_the_cadence_interval() {
        val t0 = 1_000_000L
        SassyTalkNative.emergencyActivate(SassyTalkNative.EMERGENCY_KIND_SOS, nowMs = t0)
        assertNull("no re-broadcast before the interval", SassyTalkNative.emergencyTick(t0 + 4_999))
        assertNotNull("re-broadcast at the interval", SassyTalkNative.emergencyTick(t0 + 5_000))
        assertNull("interval restarts from last beacon", SassyTalkNative.emergencyTick(t0 + 9_000))
        assertNotNull(SassyTalkNative.emergencyTick(t0 + 10_000))
    }

    @Test
    fun tick_is_inert_when_no_emergency_is_active() {
        assertFalse(SassyTalkNative.emergencyIsActive())
        assertNull(SassyTalkNative.emergencyTick(System.currentTimeMillis()))
    }
}
