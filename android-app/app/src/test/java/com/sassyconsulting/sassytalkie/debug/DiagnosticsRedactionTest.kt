// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie.debug

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticsRedactionTest {

    private val room = "788fed7c-3441-44e9-a4fe-01c8a98decf8"

    @Test
    fun logLineKeepsOnlyTheRoomPrefix() {
        val json = """{"transport":"Cellular","relay":{"state":"connected","room":"$room","queued":3}}"""
        val logged = DiagnosticsCollector.redactRoomForLog(json)
        assertFalse(logged.contains(room))
        assertTrue(logged.contains(""""room":"788fed7c""""))
        // Everything else is untouched.
        assertTrue(logged.contains(""""state":"connected""""))
        assertTrue(logged.contains(""""queued":3"""))
    }

    @Test
    fun shortEmptyAndMissingRoomsPassThrough() {
        val short = """{"relay":{"room":"abc"}}"""
        assertEquals(short, DiagnosticsCollector.redactRoomForLog(short))
        val empty = """{"relay":{"room":""}}"""
        assertEquals(empty, DiagnosticsCollector.redactRoomForLog(empty))
        val none = """{"relay":{"state":"idle"}}"""
        assertEquals(none, DiagnosticsCollector.redactRoomForLog(none))
    }

    @Test
    fun theInAppParserStillSeesTheFullRoom() {
        // Redaction is for the log line only; the room-match check in the
        // diagnostics panel compares full ids.
        val stats = DiagnosticsCollector.parseCellularStats("""{"state":"connected","room":"$room"}""")
        assertEquals(room, stats.room)
    }
}
