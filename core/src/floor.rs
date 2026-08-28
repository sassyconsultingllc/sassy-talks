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
