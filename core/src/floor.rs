// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-FLR7WQ3XKD8M
//! floor — deterministic busy-channel and emergency-preemption policy.
//!
//! This is the Rust half of the policy that ships in the Android app as
//! `FloorArbitration.kt`. Both halves MUST agree: every upgraded receiver on a
//! channel runs [`remote_wins`] against the same `OP_PTT_START_V2` payload, and
//! the whole point is that exactly one of two simultaneously-keying radios
//! yields. If Android and iOS disagree, contention resolves to either two
//! talkers or none — the two failure modes a walkie must never have.
//!
//! Floor occupancy is deliberately NOT the same thing as the UI "peer speaking"
//! LED. The LED blinks off during a 400 ms cellular gap; using it as the TX lock
//! lets a second radio key up while the first stream is still draining.
//!
//! ## Epoch comparison is SIGNED — read before changing
//!
//! Session epochs are random non-zero 64-bit values that travel the wire as 8
//! little-endian bytes, so the bytes are unambiguous. The *comparison* is not.
//! Android generates them with Kotlin `Random.nextLong()` and compares with
//! `remoteEpoch < localEpoch` on a **signed** `Long`, so roughly half of all
//! epochs are negative and sort BELOW every positive epoch. Comparing the same
//! bytes as `u64` inverts the outcome for any pair that straddles the sign bit —
//! i.e. ~50% of contentions would have Android yield to iOS *and* iOS yield to
//! Android, or neither yield. [`remote_wins`] therefore takes `u64` (the wire
//! type) and compares `as i64` to reproduce Kotlin's ordering exactly.

/// UI LED dwell only. Never use this as the TX floor lock.
pub const UI_SPEAKING_MS: u64 = 400;

/// Keep the floor held after PTT_STOP so the jitter buffer can drain.
pub const DRAIN_HOLD_MS: u64 = 300;

/// Audio-silence stale hold. Must outlast relay jitter (100–500 ms) plus the
/// Live prebuffer, otherwise a gap looks like "channel free".
pub const STALE_HOLD_MS: u64 = 1_500;

/// Hard safety ceiling for every TX source, including latching accessories.
/// Mirrors Android `PttCoordinator.DEFAULT_MAX_TX_MS`.
pub const DEFAULT_MAX_TX_MS: u64 = 60_000;

/// Should a local PTT press be refused because the channel is busy?
///
/// A local emergency overrides a held floor — a distress call is never blocked
/// by someone else holding the channel.
#[inline]
pub fn should_block_local(floor_held: bool, local_emergency: bool) -> bool {
    floor_held && !local_emergency
}

/// Does the REMOTE peer win a simultaneous floor request against us?
///
/// Ordering, highest priority first:
///   1. Emergency beats non-emergency.
///   2. Lower session epoch wins (**signed** comparison — see module docs).
///   3. Lexicographically smaller peer id wins (tie-break for the negligible
///      equal-epoch case; only applied when both ids are known).
///   4. Otherwise we keep the floor.
///
/// Byte-for-byte equivalent to `FloorArbitration.remoteWins` in the Android app.
pub fn remote_wins(
    local_epoch: u64,
    local_emergency: bool,
    remote_epoch: u64,
    remote_emergency: bool,
    local_peer_id: &str,
    remote_peer_id: &str,
) -> bool {
    if remote_emergency != local_emergency {
        return remote_emergency;
    }
    if remote_epoch != local_epoch {
        // Signed, to match Kotlin's `Long` comparison. See module docs.
        return (remote_epoch as i64) < (local_epoch as i64);
    }
    if !local_peer_id.is_empty() && !remote_peer_id.is_empty() {
        return remote_peer_id < local_peer_id;
    }
    false
}

// ── Runtime floor occupancy ──────────────────────────────────────────────
//
// Shared by iOS and desktop (Android keeps its coroutine-job implementation
// in PttCoordinator, which this mirrors). Deadlines, not timers: holds are
// absolute expiry timestamps evaluated lazily on read, so every accessor takes
// the caller's `now_ms` and the type is testable without sleeping. Moved here
// from ios-native so the desktop's floor control is the same code, not a copy.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// Why a local PTT press was refused or ended — worded to match Android's
/// PttCoordinator strings so support/QA see the same text on every platform.
pub const REJECT_CHANNEL_BUSY: &str = "Channel busy";
pub const REJECT_NOT_ENCRYPTED: &str = "Authenticate via QR first";
pub const REJECT_MAX_TX: &str = "Transmission stopped after safety time limit";
/// Another radio won floor arbitration while we were transmitting.
pub const REJECT_PREEMPTED: &str = "Channel taken by another radio";

/// Local floor bookkeeping.
pub struct FloorState {
    /// Peer id that currently owns the floor, if any.
    owner: Mutex<Option<String>>,
    /// Absolute ms timestamp at which the current hold lapses. 0 = no hold.
    hold_until_ms: AtomicU64,
    /// Absolute ms timestamp at which the UI "peer speaking" LED should clear.
    speaking_until_ms: AtomicU64,
    /// True while THIS device is broadcasting a distress beacon.
    self_emergency: AtomicBool,
    /// Last local-press rejection reason, consumed by the UI.
    reject_reason: Mutex<Option<String>>,
}

impl FloorState {
    pub fn new() -> Self {
        Self {
            owner: Mutex::new(None),
            hold_until_ms: AtomicU64::new(0),
            speaking_until_ms: AtomicU64::new(0),
            self_emergency: AtomicBool::new(false),
            reject_reason: Mutex::new(None),
        }
    }

    /// Grant/renew the floor to `peer_id` for `hold_ms`, and light the UI LED.
    ///
    /// Called both on `OP_PTT_START_V2` and on every inbound audio frame, so a
    /// stream whose control frame was lost still occupies the floor (Android's
    /// `onPeerAudioFrame` does the same). Renewing is a plain store — an earlier
    /// hold for the same peer is simply extended.
    pub fn hold(&self, peer_id: &str, hold_ms: u64, now_ms: u64) {
        *self.owner.lock().unwrap_or_else(|e| e.into_inner()) = Some(peer_id.to_string());
        self.hold_until_ms
            .store(now_ms.saturating_add(hold_ms), Ordering::SeqCst);
        self.speaking_until_ms
            .store(now_ms.saturating_add(UI_SPEAKING_MS), Ordering::SeqCst);
    }

    /// Shorten the hold to the jitter-buffer drain window after a clean
    /// `OP_PTT_STOP_V2`, so the channel does not stay locked for the full stale
    /// hold once we know the talker finished.
    ///
    /// Only shortens: if the remaining hold is already inside the drain window we
    /// leave it, and if a *different* peer has since taken the floor we do
    /// nothing at all (mirrors Android's `if (floorPeerId == peerId)` guard).
    pub fn release_after_drain(&self, peer_id: &str, now_ms: u64) {
        let owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        if owner.as_deref() != Some(peer_id) {
            return;
        }
        let drain_deadline = now_ms.saturating_add(DRAIN_HOLD_MS);
        // fetch_update rather than a bare store: an inbound audio frame may have
        // extended the hold between the caller's `now_ms` and here.
        let _ = self
            .hold_until_ms
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current > drain_deadline {
                    Some(drain_deadline)
                } else {
                    None
                }
            });
    }

    /// Drop the floor immediately (local release, or session wipe).
    pub fn clear(&self) {
        *self.owner.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.hold_until_ms.store(0, Ordering::SeqCst);
        self.speaking_until_ms.store(0, Ordering::SeqCst);
    }

    /// Drop the floor only if `peer_id` still owns it.
    pub fn clear_if_owner(&self, peer_id: &str) {
        let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        if owner.as_deref() == Some(peer_id) {
            *owner = None;
            self.hold_until_ms.store(0, Ordering::SeqCst);
        }
    }

    /// Is the floor currently occupied by anyone?
    pub fn is_held(&self, now_ms: u64) -> bool {
        self.hold_until_ms.load(Ordering::SeqCst) > now_ms
    }

    /// Current floor owner, or `None` once the hold has lapsed.
    pub fn owner(&self, now_ms: u64) -> Option<String> {
        if !self.is_held(now_ms) {
            return None;
        }
        self.owner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Should the UI show a peer as speaking?
    pub fn peer_speaking(&self, now_ms: u64) -> bool {
        self.speaking_until_ms.load(Ordering::SeqCst) > now_ms
    }

    pub fn self_emergency(&self) -> bool {
        self.self_emergency.load(Ordering::SeqCst)
    }

    pub fn set_self_emergency(&self, active: bool) {
        self.self_emergency.store(active, Ordering::SeqCst);
    }

    /// Record why a press was refused, for the UI to pick up.
    pub fn set_reject_reason(&self, reason: &str) {
        *self.reject_reason.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason.to_string());
    }

    pub fn clear_reject_reason(&self) {
        *self.reject_reason.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Read-and-clear the pending rejection reason (one-shot, like a snackbar).
    pub fn take_reject_reason(&self) -> Option<String> {
        self.reject_reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Would a local press be refused right now? Wraps the shared policy so the
    /// emergency override lives in exactly one place.
    pub fn should_block_local(&self, now_ms: u64) -> bool {
        should_block_local(self.is_held(now_ms), self.self_emergency())
    }
}

impl Default for FloorState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_emergency_overrides_busy_channel() {
        assert!(should_block_local(true, false));
        assert!(!should_block_local(true, true));
        assert!(!should_block_local(false, false));
        assert!(!should_block_local(false, true));
    }

    #[test]
    fn emergency_beats_epoch() {
        // Remote has the LOSING (higher) epoch but is in emergency — it wins.
        assert!(remote_wins(1, false, 9_999, true, "a", "b"));
        // And symmetrically, a local emergency holds the floor against a lower epoch.
        assert!(!remote_wins(9_999, true, 1, false, "a", "b"));
    }

    #[test]
    fn lower_epoch_wins_when_priority_matches() {
        assert!(remote_wins(500, false, 100, false, "a", "b"));
        assert!(!remote_wins(100, false, 500, false, "a", "b"));
    }

    /// The regression this module exists to prevent. `u64::MAX` is `-1` as i64,
    /// so under Kotlin's signed ordering it is the SMALLEST epoch and wins.
    /// An unsigned comparison would call it the largest and lose — meaning
    /// Android and iOS would both yield (dead air) or both talk (garble).
    #[test]
    fn epoch_comparison_is_signed_like_kotlin() {
        let negative_as_i64 = u64::MAX; // -1i64
        let positive = 1u64;
        assert!(
            remote_wins(positive, false, negative_as_i64, false, "", ""),
            "remote epoch -1 must win against local epoch 1 (signed ordering)"
        );
        assert!(
            !remote_wins(negative_as_i64, false, positive, false, "", ""),
            "local epoch -1 must keep the floor against remote epoch 1"
        );
    }

    #[test]
    fn peer_id_breaks_equal_epoch_ties() {
        assert!(remote_wins(7, false, 7, false, "ios-b", "ios-a"));
        assert!(!remote_wins(7, false, 7, false, "ios-a", "ios-b"));
        // Unknown ids => incumbent keeps the floor.
        assert!(!remote_wins(7, false, 7, false, "", "ios-a"));
        assert!(!remote_wins(7, false, 7, false, "ios-a", ""));
    }

    /// Exactly one side must yield, for every combination. This is the property
    /// that actually matters on the air.
    #[test]
    fn arbitration_is_antisymmetric() {
        let epochs = [1u64, 2, 500, u64::MAX, u64::MAX - 1, 1 << 63];
        for &a in &epochs {
            for &b in &epochs {
                if a == b {
                    continue;
                }
                let a_yields = remote_wins(a, false, b, false, "peer-a", "peer-b");
                let b_yields = remote_wins(b, false, a, false, "peer-b", "peer-a");
                assert_ne!(
                    a_yields, b_yields,
                    "epochs {a}/{b} must produce exactly one winner"
                );
            }
        }
    }

    #[test]
    fn hold_windows_are_ordered() {
        // The drain hold must not outlast the stale hold, and the UI LED must be
        // the shortest of the three or it would imply floor state it doesn't own.
        assert!(UI_SPEAKING_MS < STALE_HOLD_MS);
        assert!(DRAIN_HOLD_MS < STALE_HOLD_MS);
    }
}

#[cfg(test)]
mod floor_state_tests {
    use super::*;

    const T0: u64 = 1_700_000_000_000;

    #[test]
    fn hold_expires_on_its_own_without_a_timer() {
        let f = FloorState::new();
        f.hold("peer-a", STALE_HOLD_MS, T0);
        assert!(f.is_held(T0 + 1));
        assert!(f.is_held(T0 + STALE_HOLD_MS - 1));
        assert!(!f.is_held(T0 + STALE_HOLD_MS));
        assert_eq!(f.owner(T0 + 1).as_deref(), Some("peer-a"));
        assert_eq!(f.owner(T0 + STALE_HOLD_MS), None);
    }

    #[test]
    fn ui_led_clears_before_the_floor_does() {
        let f = FloorState::new();
        f.hold("peer-a", STALE_HOLD_MS, T0);
        // The 400 ms LED lapses long before the 1500 ms floor lock — the exact
        // asymmetry that stops a cellular gap from reading as "channel free".
        assert!(!f.peer_speaking(T0 + UI_SPEAKING_MS));
        assert!(f.is_held(T0 + UI_SPEAKING_MS));
    }

    #[test]
    fn inbound_audio_reasserts_a_lapsing_hold() {
        let f = FloorState::new();
        f.hold("peer-a", STALE_HOLD_MS, T0);
        let later = T0 + STALE_HOLD_MS - 100;
        f.hold("peer-a", STALE_HOLD_MS, later);
        assert!(f.is_held(T0 + STALE_HOLD_MS + 100));
    }

    #[test]
    fn drain_shortens_but_never_extends_the_hold() {
        let f = FloorState::new();
        f.hold("peer-a", STALE_HOLD_MS, T0);
        f.release_after_drain("peer-a", T0);
        assert!(f.is_held(T0 + DRAIN_HOLD_MS - 1));
        assert!(!f.is_held(T0 + DRAIN_HOLD_MS));

        // A stop arriving when less than DRAIN_HOLD_MS remains must not push the
        // deadline back out.
        let g = FloorState::new();
        g.hold("peer-a", 50, T0);
        g.release_after_drain("peer-a", T0);
        assert!(!g.is_held(T0 + 50));
    }

    #[test]
    fn drain_ignores_a_stop_from_a_peer_that_no_longer_owns_the_floor() {
        let f = FloorState::new();
        f.hold("peer-a", STALE_HOLD_MS, T0);
        f.hold("peer-b", STALE_HOLD_MS, T0 + 10);
        // Late STOP from A must not curtail B's floor.
        f.release_after_drain("peer-a", T0 + 20);
        assert!(f.is_held(T0 + STALE_HOLD_MS));
        assert_eq!(f.owner(T0 + 100).as_deref(), Some("peer-b"));
    }

    #[test]
    fn local_emergency_beats_a_held_floor() {
        let f = FloorState::new();
        f.hold("peer-a", STALE_HOLD_MS, T0);
        assert!(f.should_block_local(T0 + 1));
        f.set_self_emergency(true);
        assert!(!f.should_block_local(T0 + 1));
    }

    #[test]
    fn reject_reason_is_one_shot() {
        let f = FloorState::new();
        assert_eq!(f.take_reject_reason(), None);
        f.set_reject_reason(REJECT_CHANNEL_BUSY);
        assert_eq!(f.take_reject_reason().as_deref(), Some(REJECT_CHANNEL_BUSY));
        assert_eq!(f.take_reject_reason(), None);
    }

    #[test]
    fn clear_if_owner_only_matches_the_owner() {
        let f = FloorState::new();
        f.hold("peer-a", STALE_HOLD_MS, T0);
        f.clear_if_owner("peer-b");
        assert!(f.is_held(T0 + 1));
        f.clear_if_owner("peer-a");
        assert!(!f.is_held(T0 + 1));
    }
}
