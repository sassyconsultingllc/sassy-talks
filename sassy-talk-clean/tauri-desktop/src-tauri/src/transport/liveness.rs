use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq, Clone, Copy, serde::Serialize)]
pub enum PeerHealth { Healthy, Degraded, Stale }

struct PeerState {
    epoch: u64,
    last_rx_ms: u64,
    last_presence: u8,
    rtt_ms: Option<u32>,
    pending: HashMap<u32, u64>,
}

pub struct LivenessTracker {
    peers: HashMap<String, PeerState>,
}

impl LivenessTracker {
    pub fn new() -> Self { Self { peers: HashMap::new() } }

    pub fn on_heartbeat_sent(&mut self, peer: &str, _epoch: u64, seq: u32, ts_ms: u64) {
        let p = self.peers.entry(peer.to_string()).or_insert(PeerState {
            epoch: 0, last_rx_ms: 0, last_presence: 0, rtt_ms: None, pending: HashMap::new(),
        });
        if p.pending.len() > 64 { p.pending.clear(); }
        p.pending.insert(seq, ts_ms);
    }

    pub fn on_heartbeat(&mut self, peer: &str, epoch: u64, seq: u32,
                        _ts_ms: u64, now_ms: u64, state_byte: u8) {
        let p = self.peers.entry(peer.to_string()).or_insert(PeerState {
            epoch, last_rx_ms: now_ms, last_presence: state_byte,
            rtt_ms: None, pending: HashMap::new(),
        });
        p.epoch = epoch;
        p.last_rx_ms = now_ms;
        p.last_presence = state_byte;
        if let Some(sent) = p.pending.remove(&seq) {
            p.rtt_ms = Some((now_ms.saturating_sub(sent)) as u32);
        }
    }

    pub fn health(&self, peer: &str, now_ms: u64) -> PeerHealth {
        let Some(p) = self.peers.get(peer) else { return PeerHealth::Stale; };
        let age = now_ms.saturating_sub(p.last_rx_ms);
        if age < 3_000 { PeerHealth::Healthy }
        else if age < 8_000 { PeerHealth::Degraded }
        else { PeerHealth::Stale }
    }

    pub fn rtt_ms(&self, peer: &str) -> Option<u32> {
        self.peers.get(peer).and_then(|p| p.rtt_ms)
    }

    pub fn last_presence(&self, peer: &str) -> u8 {
        self.peers.get(peer).map_or(0, |p| p.last_presence)
    }

    pub fn last_heard_ms(&self, peer: &str) -> u64 {
        self.peers.get(peer).map_or(0, |p| p.last_rx_ms)
    }

    pub fn epoch_changed(&mut self, peer: &str, new_epoch: u64) -> bool {
        let p = self.peers.entry(peer.to_string()).or_insert(PeerState {
            epoch: new_epoch, last_rx_ms: 0, last_presence: 0,
            rtt_ms: None, pending: HashMap::new(),
        });
        let changed = p.epoch != 0 && p.epoch != new_epoch;
        p.epoch = new_epoch;
        changed
    }

    pub fn peer_ids(&self) -> Vec<String> {
        self.peers.keys().cloned().collect()
    }

    pub fn remove_peer(&mut self, peer: &str) {
        self.peers.remove(peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_peer_is_healthy() {
        let mut t = LivenessTracker::new();
        t.on_heartbeat("alice", 1, 0, 1000, 1000, 1);
        assert_eq!(t.health("alice", 1_500), PeerHealth::Healthy);
    }

    #[test]
    fn degrades_then_stales() {
        let mut t = LivenessTracker::new();
        t.on_heartbeat("alice", 1, 0, 0, 0, 0);
        assert_eq!(t.health("alice", 2_999), PeerHealth::Healthy);
        assert_eq!(t.health("alice", 3_001), PeerHealth::Degraded);
        assert_eq!(t.health("alice", 8_001), PeerHealth::Stale);
    }

    #[test]
    fn rtt_from_echo() {
        let mut t = LivenessTracker::new();
        t.on_heartbeat_sent("alice", 1, 7, 1000);
        t.on_heartbeat("alice", 1, 7, 1000, 1040, 1);
        assert_eq!(t.rtt_ms("alice"), Some(40));
    }

    #[test]
    fn unknown_peer_is_stale() {
        let t = LivenessTracker::new();
        assert_eq!(t.health("nobody", 0), PeerHealth::Stale);
    }

    #[test]
    fn epoch_change_detected() {
        let mut t = LivenessTracker::new();
        t.on_heartbeat("alice", 1, 0, 0, 0, 0);
        assert!(t.epoch_changed("alice", 2));
        assert!(!t.epoch_changed("alice", 2));
    }

    #[test]
    fn presence_tracked() {
        let mut t = LivenessTracker::new();
        t.on_heartbeat("alice", 1, 0, 0, 0, 3); // 3 = Muted
        assert_eq!(t.last_presence("alice"), 3);
    }
}
