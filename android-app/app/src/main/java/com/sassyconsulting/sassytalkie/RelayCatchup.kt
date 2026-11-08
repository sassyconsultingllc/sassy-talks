// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RLYCATCHUP4K
package com.sassyconsulting.sassytalkie

/**
 * Relay store-and-forward cursor. Mirrors `core::protocol::catchup_since_ms`
 * (desktop and iOS call that directly) so all three clients ask for the same
 * window.
 *
 * The relay retains ~30 s of room audio after a peer drops. A reconnecting
 * client sends `since=<ms>` and gets only frames that arrived after it went
 * dark, wrapped as OP_REPLAY_FRAME (unwrapped natively before decryption).
 * Frames already heard live are rejected by the AEAD replay window.
 */
object RelayCatchup {
    /** Gaps longer than this get no replay: a 20 s-late burst is a ghost transmission. */
    const val MAX_GAP_MS = 15_000L
    /** Covers the frame in flight when the socket died. */
    private const val MARGIN_MS = 250L

    /** `since=` value for a reconnect, or null to skip catch-up. */
    fun sinceMs(lastAliveMs: Long, nowMs: Long): Long? {
        if (lastAliveMs <= 0L) return null
        if (nowMs - lastAliveMs > MAX_GAP_MS) return null
        return (lastAliveMs - MARGIN_MS).coerceAtLeast(0L)
    }
}
