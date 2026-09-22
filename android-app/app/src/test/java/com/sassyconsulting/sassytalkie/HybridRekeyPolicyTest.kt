// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HybridRekeyPolicyTest {
    @Test
    fun `matching confirm installs only while staged handshake is live`() {
        val token = HybridRekeyPolicy.tokenFor(byteArrayOf(1, 2, 3, 4))
        val now = 1_700_000_000_000L
        assertTrue(HybridRekeyPolicy.confirmAcceptable(token, token.copyOf(), now, now))
        assertTrue(HybridRekeyPolicy.ackAcceptable(token, token.copyOf(), now, now))
    }

    @Test
    fun `forged confirm token is rejected`() {
        val staged = HybridRekeyPolicy.tokenFor(byteArrayOf(1, 2, 3, 4))
        val forged = HybridRekeyPolicy.tokenFor(byteArrayOf(9, 9, 9, 9))
        val now = 1_700_000_000_000L
        assertFalse(HybridRekeyPolicy.confirmAcceptable(staged, forged, now, now))
        assertFalse(HybridRekeyPolicy.confirmAcceptable(staged, null, now, now))
        assertFalse(HybridRekeyPolicy.confirmAcceptable(null, staged, now, now))
    }

    @Test
    fun `stale confirm after ttl is rejected`() {
        val token = HybridRekeyPolicy.tokenFor(byteArrayOf(7))
        val stagedAt = 1_700_000_000_000L
        val late = stagedAt + HybridRekeyPolicy.STAGED_TTL_MS + 1
        assertFalse(HybridRekeyPolicy.confirmAcceptable(token, token, late, stagedAt))
        assertFalse(HybridRekeyPolicy.confirmAcceptable(token, token, stagedAt - 1, stagedAt))
    }

    @Test
    fun `lost confirm leaves both on old key`() {
        val i = FourWayHybrid()
        val r = FourWayHybrid()
        r.onInitAtResponder()
        i.onRespAtInitiator()
        i.expireTogether()
        r.expireTogether()
        assertFalse(i.initiatorLive)
        assertFalse(r.responderLive)
        assertFalse(FourWayHybrid.pairTxSplit(i, r))
    }

    @Test
    fun `lost ack does not split tx keys`() {
        val i = FourWayHybrid()
        val r = FourWayHybrid()
        r.onInitAtResponder()
        i.onRespAtInitiator()
        assertTrue(r.onConfirmAtResponder())
        assertFalse(r.responderLive)
        i.expireTogether()
        r.expireTogether()
        assertFalse(i.initiatorLive)
        assertFalse(r.responderLive)
        assertFalse(i.txKeysSplit())
    }

    @Test
    fun `late ack within ttl then peer new-key commits responder`() {
        val i = FourWayHybrid()
        val r = FourWayHybrid()
        r.onInitAtResponder()
        i.onRespAtInitiator()
        assertTrue(r.onConfirmAtResponder())
        assertTrue(i.onAckAtInitiator())
        assertTrue(i.initiatorLive)
        assertFalse(r.responderLive)
        assertTrue(r.onPeerNewKeyAtResponder())
        assertTrue(r.responderLive)
        assertFalse(FourWayHybrid.pairTxSplit(i, r))
    }

    @Test
    fun `forged or unstaged ack does not install`() {
        val i = FourWayHybrid()
        assertFalse(i.onAckAtInitiator())
        assertFalse(i.initiatorLive)
        val r = FourWayHybrid()
        assertFalse(r.onConfirmAtResponder())
        assertFalse(r.responderLive)
    }

    @Test
    fun `timeout rollback drops staged keys together`() {
        val hs = FourWayHybrid()
        hs.onInitAtResponder()
        hs.onRespAtInitiator()
        hs.expireTogether()
        assertFalse(hs.initiatorStaged)
        assertFalse(hs.responderStaged)
        assertFalse(hs.initiatorLive)
        assertFalse(hs.responderLive)
        assertFalse(hs.txKeysSplit())
    }

    @Test
    fun `confirm retry reemits ack`() {
        val i = FourWayHybrid()
        val r = FourWayHybrid()
        r.onInitAtResponder()
        i.onRespAtInitiator()
        assertTrue(i.shouldRetryConfirm(2_000L, 1_000L))
        assertTrue(r.onConfirmAtResponder())
        assertTrue(r.onConfirmRetryAtResponder())
        i.noteConfirmRetry()
        i.noteConfirmRetry()
        i.noteConfirmRetry()
        assertFalse(i.shouldRetryConfirm(2_000L, 1_000L))
    }
}
