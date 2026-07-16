// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-4MGEPKJJGRNQ
package com.sassyconsulting.sassytalkie

import com.sassyconsulting.sassytalkie.ui.peerStatus
import com.sassyconsulting.sassytalkie.PresenceState
import com.sassyconsulting.sassytalkie.PeerHealth
import org.junit.Assert.assertEquals
import org.junit.Test

class PeerStatusTest {

    @Test
    fun registryPeerWithoutHeartbeats_showsOnChannel_notOffline() {
        val s = peerStatus(
            presence = PresenceState.IDLE,
            health = null,
            lastHeardMs = 0L,
            rttMs = -1,
            inActiveSet = false,
            isTracked = false,
            nowMs = System.currentTimeMillis(),
        )
        assertEquals("On channel", s.label)
    }

    @Test
    fun untrackedStaleHealth_showsOnChannel_notOutOfContact() {
        val s = peerStatus(
            presence = PresenceState.IDLE,
            health = PeerHealth.STALE,
            lastHeardMs = 0L,
            rttMs = -1,
            inActiveSet = false,
            isTracked = false,
            nowMs = System.currentTimeMillis(),
        )
        assertEquals("On channel", s.label)
    }

    @Test
    fun activePeer_showsInChannel() {
        val s = peerStatus(
            presence = PresenceState.IDLE,
            health = PeerHealth.HEALTHY,
            lastHeardMs = System.currentTimeMillis(),
            rttMs = 42,
            inActiveSet = true,
            isTracked = true,
            nowMs = System.currentTimeMillis(),
        )
        assertEquals("In channel · 42 ms", s.label)
    }

    @Test
    fun staleTrackedPeer_showsOutOfContact() {
        val s = peerStatus(
            presence = PresenceState.IDLE,
            health = PeerHealth.STALE,
            lastHeardMs = System.currentTimeMillis() - 60_000L,
            rttMs = -1,
            inActiveSet = false,
            isTracked = true,
            nowMs = System.currentTimeMillis(),
        )
        assertEquals("Out of contact", s.label)
    }

    @Test
    fun speaking_overridesIdle() {
        val s = peerStatus(
            presence = PresenceState.SPEAKING,
            health = PeerHealth.HEALTHY,
            lastHeardMs = 0L,
            rttMs = -1,
            inActiveSet = true,
            isTracked = true,
            nowMs = System.currentTimeMillis(),
        )
        assertEquals("Speaking", s.label)
    }
}
