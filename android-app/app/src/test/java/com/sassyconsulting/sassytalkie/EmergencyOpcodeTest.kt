// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Guards the wire-opcode space.
 *
 * WHY THIS EXISTS: `core/src/emergency.rs` self-allocated OP_MANDOWN=0x1B and
 * OP_EMERGENCY_CLEAR=0x1C after checking only the constants in
 * `core/src/protocol.rs`. But this Kotlin file carried a second, larger
 * registry that had never been promoted into Rust, and it already used
 * 0x1B/0x1C for the hybrid PQC handshake. Wiring emergency at those values
 * would have routed a man-down beacon into `PttCoordinator.handleHybridInit` —
 * a life-safety frame parsed as a key exchange, with no compiler or test
 * complaining. Two registries and no cross-check is the actual defect; these
 * tests are the cheap standing guard against it recurring.
 *
 * The Rust half of this guard is `protocol::tests::all_opcodes_are_unique`.
 */
class EmergencyOpcodeTest {

    @Test
    fun `all opcodes are unique`() {
        val seen = mutableMapOf<Byte, String>()
        for ((name, op) in ControlFrame.ALL_OPCODES) {
            val prior = seen[op]
            if (prior != null) {
                throw AssertionError(
                    "opcode collision: $name and $prior both use 0x${op.toString(16)}"
                )
            }
            seen[op] = name
        }
        assertEquals(
            "ALL_OPCODES must list every opcode exactly once",
            ControlFrame.ALL_OPCODES.size,
            seen.size,
        )
    }

    @Test
    fun `emergency opcodes do not collide with hybrid pqc handshake`() {
        assertNotEquals(ControlFrame.OP_MANDOWN, ControlFrame.OP_HYBRID_INIT)
        assertNotEquals(ControlFrame.OP_EMERGENCY_CLEAR, ControlFrame.OP_HYBRID_RESP)
        assertNotEquals(ControlFrame.OP_EMERGENCY, ControlFrame.OP_HYBRID_INIT)
        assertNotEquals(ControlFrame.OP_EMERGENCY, ControlFrame.OP_HYBRID_RESP)
    }

    /**
     * The relay client routes `op in 0x10..0x20` to the control-frame
     * handler; anything else is handed to the audio decoder. An emergency
     * opcode outside that window would be silently swallowed as audio.
     */
    @Test
    fun `emergency opcodes sit inside the control routing window`() {
        for (op in listOf(
            ControlFrame.OP_EMERGENCY,
            ControlFrame.OP_MANDOWN,
            ControlFrame.OP_EMERGENCY_CLEAR,
            ControlFrame.OP_HYBRID_CONFIRM_ACK,
        )) {
            val v = op.toInt() and 0xFF
            assertTrue("0x${op.toString(16)} must be >= 0x10 (TLV-framed)", v >= 0x10)
            assertTrue("0x${op.toString(16)} must be <= 0x20 (control window)", v <= 0x20)
        }
    }

    /**
     * Pins the exact wire values. These bytes are on the wire and shared with
     * `core/src/protocol.rs`, iOS and desktop — changing one without changing
     * every peer is a silent interop break, so the value change has to be
     * deliberate enough to edit this test too.
     */
    @Test
    fun `emergency opcode values match the core protocol registry`() {
        assertEquals(0x1A.toByte(), ControlFrame.OP_EMERGENCY)
        assertEquals(0x1D.toByte(), ControlFrame.OP_MANDOWN)
        assertEquals(0x1E.toByte(), ControlFrame.OP_EMERGENCY_CLEAR)
        assertEquals(0x1F.toByte(), ControlFrame.OP_HYBRID_CONFIRM)
        assertEquals(0x20.toByte(), ControlFrame.OP_HYBRID_CONFIRM_ACK)
        assertEquals(0x19.toByte(), ControlFrame.OP_REPLAY_FRAME)
    }
}
