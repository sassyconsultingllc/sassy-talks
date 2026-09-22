// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import java.security.MessageDigest

/**
 * Staged hybrid/PQC rekey. Mirrors `core/src/hybrid_rekey.rs`.
 * Neither side TXes on the new key until CONFIRM+ACK complete without a split.
 */
internal object HybridRekeyPolicy {
    const val STAGED_TTL_MS = 15_000L
    const val CONFIRM_RETRY_MAX = 3
    const val CONFIRM_RETRY_MS = 1_000L

    fun tokenFor(responderMessage: ByteArray): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(responderMessage)

    fun confirmAcceptable(
        stagedToken: ByteArray?,
        confirmToken: ByteArray?,
        nowMs: Long,
        stagedAtMs: Long,
    ): Boolean {
        if (stagedToken == null || confirmToken == null) return false
        if (confirmToken.size != stagedToken.size) return false
        if (nowMs - stagedAtMs > STAGED_TTL_MS || nowMs < stagedAtMs) return false
        return MessageDigest.isEqual(stagedToken, confirmToken)
    }

    fun ackAcceptable(
        stagedToken: ByteArray?,
        ackToken: ByteArray?,
        nowMs: Long,
        stagedAtMs: Long,
    ): Boolean = confirmAcceptable(stagedToken, ackToken, nowMs, stagedAtMs)
}

/** Local 4-way TX policy. TX-live only after ACK (initiator) / peer-new-key (responder). */
internal data class FourWayHybrid(
    var initiatorStaged: Boolean = false,
    var responderStaged: Boolean = false,
    var initiatorLive: Boolean = false,
    var responderLive: Boolean = false,
    var ackSent: Boolean = false,
    var confirmRetries: Int = 0,
) {
    fun onInitAtResponder() {
        responderStaged = true
        responderLive = false
        ackSent = false
    }

    fun onRespAtInitiator() {
        initiatorStaged = true
        initiatorLive = false
        confirmRetries = 0
    }

    fun onConfirmAtResponder(): Boolean {
        if (!responderStaged) return false
        ackSent = true
        return true
    }

    fun onConfirmRetryAtResponder(): Boolean =
        ackSent && responderStaged && !responderLive

    fun onAckAtInitiator(): Boolean {
        if (!initiatorStaged) return false
        initiatorStaged = false
        initiatorLive = true
        return true
    }

    fun onPeerNewKeyAtResponder(): Boolean {
        if (!ackSent || !responderStaged) return false
        responderStaged = false
        responderLive = true
        return true
    }

    fun shouldRetryConfirm(nowMs: Long, stagedAtMs: Long): Boolean =
        initiatorStaged &&
            !initiatorLive &&
            confirmRetries < HybridRekeyPolicy.CONFIRM_RETRY_MAX &&
            nowMs >= stagedAtMs &&
            nowMs - stagedAtMs <= HybridRekeyPolicy.STAGED_TTL_MS

    fun noteConfirmRetry() {
        confirmRetries += 1
    }

    fun expireTogether() {
        if (!initiatorLive) initiatorStaged = false
        if (!responderLive) {
            responderStaged = false
            ackSent = false
        }
    }

    fun txKeysSplit(): Boolean = when {
        initiatorLive == responderLive -> false
        initiatorLive && !responderLive -> !responderStaged && !ackSent
        else -> !initiatorStaged
    }

    companion object {
        fun pairTxSplit(initiator: FourWayHybrid, responder: FourWayHybrid): Boolean = when {
            initiator.initiatorLive == responder.responderLive -> false
            initiator.initiatorLive && !responder.responderLive ->
                !responder.responderStaged && !responder.ackSent
            else -> !initiator.initiatorStaged
        }
    }
}
