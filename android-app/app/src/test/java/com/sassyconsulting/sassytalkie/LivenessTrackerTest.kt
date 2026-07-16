// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-FG2SLDY7IUD4
package com.sassyconsulting.sassytalkie

import org.junit.Assert.*
import org.junit.Test

class LivenessTrackerTest {
    @Test
    fun `fresh peer is healthy`() {
        val t = LivenessTracker()
        t.onHeartbeat("alice", epoch = 1L, seq = 0, tsMs = 1000, nowMs = 1000)
        assertEquals(PeerHealth.HEALTHY, t.health("alice", nowMs = 1500))
    }

    @Test
    fun `peer degrades after 3s and goes stale after 8s`() {
        val t = LivenessTracker()
        t.onHeartbeat("alice", 1L, 0, tsMs = 0, nowMs = 0)
        assertEquals(PeerHealth.HEALTHY, t.health("alice", 2_999))
        assertEquals(PeerHealth.DEGRADED, t.health("alice", 3_001))
        assertEquals(PeerHealth.STALE, t.health("alice", 8_001))
    }

    @Test
    fun `rtt is recorded from echoed heartbeat`() {
        val t = LivenessTracker()
        t.onHeartbeatSent("alice", epoch = 1L, seq = 7, tsMs = 1000)
        t.onHeartbeat("alice", epoch = 1L, seq = 7, tsMs = 1000, nowMs = 1040)
        assertEquals(40, t.rttMs("alice"))
    }

    @Test
    fun `rtt returns -1 for unknown peer`() {
        val t = LivenessTracker()
        assertEquals(-1, t.rttMs("nobody"))
    }

    @Test
    fun `epoch change resets state`() {
        val t = LivenessTracker()
        t.onHeartbeat("alice", 1L, 99, 0, 0)
        assertTrue(t.epochChanged("alice", newEpoch = 2L))
        assertFalse(t.epochChanged("alice", newEpoch = 2L))
    }

    @Test
    fun `unknown peer is stale`() {
        val t = LivenessTracker()
        assertEquals(PeerHealth.STALE, t.health("nobody", 0))
    }

    @Test
    fun `presence tracking works`() {
        val t = LivenessTracker()
        t.onHeartbeat("alice", 1L, 0, 0, 0)
        t.updatePresence("alice", PresenceState.MUTED)
        assertEquals(PresenceState.MUTED, t.presence("alice"))
    }

    @Test
    fun `lastHeardMs returns 0 for unknown`() {
        val t = LivenessTracker()
        assertEquals(0L, t.lastHeardMs("nobody"))
    }

    @Test
    fun `removePeer cleans up`() {
        val t = LivenessTracker()
        t.onHeartbeat("alice", 1L, 0, 0, 0)
        assertTrue(t.peerIds().contains("alice"))
        t.removePeer("alice")
        assertFalse(t.peerIds().contains("alice"))
    }

    @Test
    fun `pending echoes bounded at 64`() {
        val t = LivenessTracker()
        // Spam 100 sends without any receives
        for (i in 0..99) t.onHeartbeatSent("alice", 1L, i, i.toLong())
        // Should not crash, and rtt should still be -1 (old echoes cleared)
        assertEquals(-1, t.rttMs("alice"))
    }
}
