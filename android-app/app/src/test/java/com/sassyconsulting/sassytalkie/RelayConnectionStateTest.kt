// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RelayConnectionStateTest {
    @Test
    fun `kotlin socket or native connected state is live`() {
        assertTrue(RelayConnectionState.isLive(true, "disconnected", false))
        assertTrue(RelayConnectionState.isLive(false, "connected", false))
        assertTrue(RelayConnectionState.isLive(false, "disconnected", true))
        assertFalse(RelayConnectionState.isLive(false, "disconnected", false))
        assertFalse(RelayConnectionState.isLive(false, null, false))
    }

    @Test
    fun `sticky disconnected slot is not a live overlay`() {
        assertEquals("down", RelayConnectionState.overlayWsLabel(false))
        assertEquals("up", RelayConnectionState.overlayWsLabel(true))
        assertEquals(
            "idle",
            RelayConnectionState.telemetryWsState(false, "disconnected", false),
        )
        assertEquals(
            "connected",
            RelayConnectionState.telemetryWsState(true, "disconnected", false),
        )
        assertEquals(
            "connected",
            RelayConnectionState.telemetryWsState(false, "connected", false),
        )
    }

    @Test
    fun `radio chrome never says waiting for relay when socket is up`() {
        assertEquals(
            "Relay connected — confirming room…",
            RelayConnectionState.radioStatusLine(
                socketLive = true,
                roomConfirmed = false,
            ),
        )
        assertEquals(
            "Relay connected — sealed-sender room pending",
            RelayConnectionState.radioStatusLine(
                socketLive = true,
                roomConfirmed = true,
                sealedSenderPending = true,
            ),
        )
        assertEquals(
            null,
            RelayConnectionState.radioStatusLine(
                socketLive = true,
                roomConfirmed = true,
            ),
        )
        val connecting = RelayConnectionState.radioStatusLine(
            socketLive = false,
            roomConfirmed = false,
        )
        assertEquals("Connecting to relay…", connecting)
        assertFalse(connecting!!.contains("Waiting for relay", ignoreCase = true))
        val socketUp = RelayConnectionState.radioStatusLine(true, false)!!
        assertFalse(socketUp.contains("Waiting for relay", ignoreCase = true))
        assertTrue(RelayConnectionState.isLive(true, "disconnected", false))
    }

    @Test
    fun `overlay up matches live socket`() {
        assertEquals("up", RelayConnectionState.overlayWsLabel(true))
        assertTrue(RelayConnectionState.isLive(true, "idle", false))
    }
}
