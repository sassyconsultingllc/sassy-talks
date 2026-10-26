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

    /** Wake pushes fire on PTT_START, so catch-up starts this far before the push arrived. */
    private const val WAKE_LEAD_MS = 2_000L

    // A wake can land before any client exists (cold start), so it is kept
    // process-wide until the next reconnect takes it. Mirrors iOS RelayClient.
    private val pendingWakeMs = java.util.concurrent.atomic.AtomicLong(0L)

    /** An FCM wake push arrived at [nowMs]. */
    fun noteWake(nowMs: Long) {
        pendingWakeMs.set(nowMs)
    }

    /** The pending wake time (0 if none), cleared so it anchors one reconnect only. */
    fun takePendingWake(): Long = pendingWakeMs.getAndSet(0L)

    /**
     * Earliest of the last live frame and the wake lead-in that is still inside
     * [MAX_GAP_MS]; 0 if neither is. A plain min() would pick a last-alive time
     * from before a long doze, fail the cap, and lose the wake catch-up.
     */
    fun anchorMs(lastAliveMs: Long, wakeMs: Long, nowMs: Long): Long {
        val wakeAnchor = if (wakeMs > WAKE_LEAD_MS) wakeMs - WAKE_LEAD_MS else 0L
        return listOf(lastAliveMs, wakeAnchor)
            .filter { it > 0L && nowMs - it <= MAX_GAP_MS }
            .minOrNull() ?: 0L
    }

    /** `since=` value for a reconnect, or null to skip catch-up. */
    fun sinceMs(lastAliveMs: Long, nowMs: Long): Long? {
        if (lastAliveMs <= 0L) return null
        if (nowMs - lastAliveMs > MAX_GAP_MS) return null
        return (lastAliveMs - MARGIN_MS).coerceAtLeast(0L)
    }

    /**
     * Per-room cursor. Entering a different room starts fresh, so a client
     * that was live in R1 and reconnects into R2 asks for no catch-up instead
     * of replaying R2 audio from before it joined.
     */
    class Cursor {
        private var room: String? = null
        private var lastAliveMs = 0L

        @Synchronized
        fun enterRoom(room: String) {
            if (room != this.room) {
                this.room = room
                lastAliveMs = 0L
            }
        }

        /** The current room's socket was alive at [nowMs] (open or any inbound frame). */
        @Synchronized
        fun markAlive(nowMs: Long) {
            lastAliveMs = nowMs
        }

        /** [wakeMs] is a pending wake push time (see [takePendingWake]), or 0. */
        @Synchronized
        fun sinceMs(nowMs: Long, wakeMs: Long = 0L): Long? =
            RelayCatchup.sinceMs(anchorMs(lastAliveMs, wakeMs, nowMs), nowMs)
    }
}
