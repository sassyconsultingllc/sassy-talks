package com.sassyconsulting.sassytalkie

import java.util.concurrent.ConcurrentHashMap

enum class PeerHealth { HEALTHY, DEGRADED, STALE }

class LivenessTracker {
    private data class PeerState(
        var epoch: Long = 0,
        var lastRxMs: Long = 0,
        var lastPresence: PresenceState = PresenceState.IDLE,
        var rttMs: Int = -1,
        var caps: Int = 0, // last-advertised capability bitmap (CAP_HYBRID_PQC etc.)
        val pendingEchoes: ConcurrentHashMap<Int, Long> = ConcurrentHashMap(), // seq -> sentTsMs
    )

    private val peers = ConcurrentHashMap<String, PeerState>()

    /** Record that we sent a heartbeat to this peer (for RTT matching). */
    fun onHeartbeatSent(peerId: String, epoch: Long, seq: Int, tsMs: Long) {
        val p = peers.getOrPut(peerId) { PeerState(epoch = epoch) }
        synchronized(p) {
            if (p.pendingEchoes.size > 64) p.pendingEchoes.clear()
            p.pendingEchoes[seq] = tsMs
        }
    }

    /** Process an inbound heartbeat from this peer. */
    fun onHeartbeat(peerId: String, epoch: Long, seq: Int, tsMs: Long, nowMs: Long, caps: Int = 0) {
        val p = peers.getOrPut(peerId) { PeerState(epoch = epoch) }
        synchronized(p) {
            p.epoch = epoch
            p.lastRxMs = nowMs
            p.caps = caps
            val sentAt = p.pendingEchoes.remove(seq)
            if (sentAt != null) {
                p.rttMs = (nowMs - sentAt).toInt().coerceAtLeast(0)
            }
        }
    }

    /** Last capability bitmap advertised by a peer (0 if unknown/legacy). */
    fun peerCaps(peerId: String): Int {
        val p = peers[peerId] ?: return 0
        return synchronized(p) { p.caps }
    }

    /** Update the last-known presence state for a peer (extracted from heartbeat). */
    fun updatePresence(peerId: String, state: PresenceState) {
        val p = peers[peerId] ?: return
        synchronized(p) {
            p.lastPresence = state
        }
    }

    /** True if we have ever received a heartbeat from this peer. */
    fun isTracked(peerId: String): Boolean = peers.containsKey(peerId)

    /** Get health assessment for a peer. */
    fun health(peerId: String, nowMs: Long): PeerHealth {
        val p = peers[peerId] ?: return PeerHealth.STALE
        synchronized(p) {
            val age = nowMs - p.lastRxMs
            return when {
                age < 3_000 -> PeerHealth.HEALTHY
                age < 8_000 -> PeerHealth.DEGRADED
                else        -> PeerHealth.STALE
            }
        }
    }

    /** Get last measured RTT in ms, or -1 if unknown. */
    fun rttMs(peerId: String): Int {
        val p = peers[peerId] ?: return -1
        synchronized(p) {
            return p.rttMs
        }
    }

    /** Get last known presence of a peer. */
    fun presence(peerId: String): PresenceState {
        val p = peers[peerId] ?: return PresenceState.IDLE
        synchronized(p) {
            return p.lastPresence
        }
    }

    /** Check if peer's epoch changed (they restarted). Returns true on first change. */
    fun epochChanged(peerId: String, newEpoch: Long): Boolean {
        val p = peers.getOrPut(peerId) { PeerState(epoch = newEpoch) }
        synchronized(p) {
            val changed = p.epoch != 0L && p.epoch != newEpoch
            p.epoch = newEpoch
            return changed
        }
    }

    /** Get last-heard timestamp for a peer (absolute ms), or 0 if never. */
    fun lastHeardMs(peerId: String): Long {
        val p = peers[peerId] ?: return 0
        synchronized(p) {
            return p.lastRxMs
        }
    }

    /** Get all tracked peer IDs. */
    fun peerIds(): Set<String> = peers.keys.toSet()

    /** Remove a peer (e.g., on explicit disconnect). */
    fun removePeer(peerId: String) { peers.remove(peerId) }
}
