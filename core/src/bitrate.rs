//! bitrate — receiver-side Opus bitrate guard (design: `docs/bitrate-guard-design.md`).
//!
//! Every stock SassyTalkie client encodes Opus at ~24 kbps VBR, but nothing on
//! the receive path *enforces* it: a tampered or forked client could send 8 kbps
//! mush or 64 kbps bandwidth-burn and the receiver would happily decode it. In a
//! group Mix, mismatched bitrates also make the AGC pump.
//!
//! This module implements the design doc's Option B (receiver-side estimation
//! from frame size, **no wire-format change**): estimate a frame's bitrate from
//! its byte length and reject frames whose measured bitrate falls outside a
//! tolerance band around the negotiated bitrate. Opus VBR makes single-frame
//! measurement noisy, so [`BitrateGuard`] averages over a sliding window and
//! exempts tiny DTX/CN silence beacons.
//!
//! Enforcement wiring (per-peer reject counter → evict) is left to the consumer
//! RX path; a peer's negotiated bitrate comes from the session handshake and
//! defaults to [`DEFAULT_BITRATE_BPS`] for peers that don't advertise one.

/// The bitrate every stock client uses today (see `OpusEncoder.applyPttDefaults`).
pub const DEFAULT_BITRATE_BPS: u32 = 24_000;

/// Default Opus frame duration in ms (20 ms PTT frames).
pub const DEFAULT_FRAME_MS: u32 = 20;

/// Default acceptance band around the negotiated bitrate, in percent. Opus VBR
/// spread is wide, so start permissive; tighten once real telemetry exists.
pub const DEFAULT_TOLERANCE_PCT: u32 = 50;

/// Frames at or below this many bytes are DTX/CN silence beacons — exempt from
/// the guard regardless of bitrate (they're tiny by design).
pub const SILENCE_EXEMPT_BYTES: usize = 10;

/// Sliding window length (frames) used to smooth VBR noise before judging.
pub const WINDOW_FRAMES: usize = 10;

/// Estimated bitrate (bps) for an Opus frame of `frame_bytes` bytes covering
/// `frame_ms` ms of audio. Returns 0 for a zero-length frame duration.
#[inline]
pub fn estimate_bitrate_bps(frame_bytes: usize, frame_ms: u32) -> u32 {
    if frame_ms == 0 {
        return 0;
    }
    ((frame_bytes as u64 * 8 * 1000) / frame_ms as u64) as u32
}

/// True if a single frame's measured bitrate is within `tolerance_pct` of the
/// negotiated bitrate. Silence beacons (`<= SILENCE_EXEMPT_BYTES`) always pass.
pub fn frame_within_bounds(
    frame_bytes: usize,
    frame_ms: u32,
    negotiated_bps: u32,
    tolerance_pct: u32,
) -> bool {
    if frame_bytes <= SILENCE_EXEMPT_BYTES {
        return true;
    }
    let measured = estimate_bitrate_bps(frame_bytes, frame_ms);
    let delta = measured.abs_diff(negotiated_bps);
    // Use u64 for the tolerance product so a large negotiated bitrate can't
    // overflow u32 when multiplied by the percentage.
    let allowed = (negotiated_bps as u64 * tolerance_pct as u64 / 100) as u32;
    delta <= allowed
}

/// Sliding-window bitrate guard for one peer. Averages the last [`WINDOW_FRAMES`]
/// non-silence frames and judges the average against the tolerance band, which
/// smooths out Opus VBR's per-frame spikes.
#[derive(Debug, Clone)]
pub struct BitrateGuard {
    negotiated_bps: u32,
    frame_ms: u32,
    tolerance_pct: u32,
    window: [u32; WINDOW_FRAMES],
    len: usize,
    pos: usize,
    /// Cumulative frames judged out-of-band (for the consumer's evict policy).
    rejects: u32,
}

impl BitrateGuard {
    /// New guard for a peer's negotiated bitrate. Pass [`DEFAULT_BITRATE_BPS`]
    /// for peers that don't advertise a codec in the handshake.
    pub fn new(negotiated_bps: u32) -> Self {
        Self::with_params(negotiated_bps, DEFAULT_FRAME_MS, DEFAULT_TOLERANCE_PCT)
    }

    pub fn with_params(negotiated_bps: u32, frame_ms: u32, tolerance_pct: u32) -> Self {
        Self {
            negotiated_bps,
            frame_ms: frame_ms.max(1),
            tolerance_pct,
            window: [0; WINDOW_FRAMES],
            len: 0,
            pos: 0,
            rejects: 0,
        }
    }

    /// Feed a decoded/received frame's byte length. Returns true if the frame
    /// should be ACCEPTED, false if it should be dropped. Silence beacons are
    /// always accepted and don't disturb the running average.
    pub fn accept(&mut self, frame_bytes: usize) -> bool {
        if frame_bytes <= SILENCE_EXEMPT_BYTES {
            return true;
        }
        let measured = estimate_bitrate_bps(frame_bytes, self.frame_ms);
        self.window[self.pos] = measured;
        self.pos = (self.pos + 1) % WINDOW_FRAMES;
        if self.len < WINDOW_FRAMES {
            self.len += 1;
        }

        // Need a few samples before judging so a single loud onset frame doesn't
        // trip the guard; accept while warming up.
        if self.len < 3 {
            return true;
        }

        let sum: u64 = self.window[..self.len].iter().map(|&v| v as u64).sum();
        let avg = (sum / self.len as u64) as u32;
        let delta = avg.abs_diff(self.negotiated_bps);
        let allowed = (self.negotiated_bps as u64 * self.tolerance_pct as u64 / 100) as u32;
        let ok = delta <= allowed;
        if !ok {
            self.rejects = self.rejects.saturating_add(1);
        }
        ok
    }

    /// Total frames rejected so far (consumer decides the evict threshold).
    pub fn rejects(&self) -> u32 {
        self.rejects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_matches_hand_math() {
        // 60 bytes over 20 ms = 60*8/0.02 = 24_000 bps.
        assert_eq!(estimate_bitrate_bps(60, 20), 24_000);
        assert_eq!(estimate_bitrate_bps(0, 20), 0);
        assert_eq!(estimate_bitrate_bps(60, 0), 0);
    }

    #[test]
    fn nominal_frame_is_accepted() {
        assert!(frame_within_bounds(60, 20, DEFAULT_BITRATE_BPS, DEFAULT_TOLERANCE_PCT));
    }

    #[test]
    fn silence_beacon_is_exempt() {
        // A 3-byte CN frame estimates to a tiny bitrate but must not be dropped.
        assert!(frame_within_bounds(3, 20, DEFAULT_BITRATE_BPS, DEFAULT_TOLERANCE_PCT));
    }

    #[test]
    fn wildly_high_bitrate_is_rejected() {
        // 400 bytes/20ms = 160 kbps, far outside 24k ±50%.
        assert!(!frame_within_bounds(400, 20, DEFAULT_BITRATE_BPS, DEFAULT_TOLERANCE_PCT));
    }

    #[test]
    fn guard_warms_up_then_rejects_sustained_overshoot() {
        let mut g = BitrateGuard::new(DEFAULT_BITRATE_BPS);
        // Warm-up frames accepted.
        assert!(g.accept(300));
        assert!(g.accept(300));
        // By now the window average (300 bytes = 120 kbps) is well over band.
        let _ = g.accept(300);
        let mut rejected = false;
        for _ in 0..WINDOW_FRAMES {
            if !g.accept(300) {
                rejected = true;
            }
        }
        assert!(rejected, "sustained 120 kbps stream must be rejected");
        assert!(g.rejects() > 0);
    }

    #[test]
    fn guard_accepts_nominal_stream() {
        let mut g = BitrateGuard::new(DEFAULT_BITRATE_BPS);
        for _ in 0..50 {
            assert!(g.accept(60), "nominal 24 kbps frames must pass");
        }
        assert_eq!(g.rejects(), 0);
    }

    #[test]
    fn guard_tolerates_vbr_spikes_around_nominal() {
        let mut g = BitrateGuard::new(DEFAULT_BITRATE_BPS);
        // Alternate 40 and 80 bytes (16 kbps / 32 kbps) — averages to nominal.
        for i in 0..40 {
            let bytes = if i % 2 == 0 { 40 } else { 80 };
            assert!(g.accept(bytes));
        }
        assert_eq!(g.rejects(), 0);
    }
}
