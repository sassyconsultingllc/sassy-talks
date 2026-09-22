// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-PTF9NZQK4V2C
//! ptt_frames — TLV codecs for the PTT / liveness control opcodes.
//!
//! These moved into `core` for the reason `protocol.rs` documents at length: the
//! opcode CONSTANTS were centralised but the ENCODERS were not, so
//! `tauri-desktop/src-tauri/src/transport/control.rs` and the Android
//! `ControlFrame.kt` each carried their own copy and drifted. The observable
//! drift today is `OP_PTT_START_V2`: Android writes a 13-byte payload whose
//! trailing byte is the emergency-priority flag that drives floor preemption,
//! while the desktop copy writes 12 bytes and therefore can never request
//! priority. iOS is built against this module so it matches the shipping Android
//! app, which is the wire authority.
//!
//! Payload layouts (all little-endian), wrapped by [`crate::protocol::encode_tlv`]:
//!   - `OP_PTT_START_V2`  `[epoch:u64][start_seq:u32][emergency:u8]`  (13 B)
//!   - `OP_PTT_STOP_V2`   `[epoch:u64][end_seq:u32]`                  (12 B)
//!   - `OP_WAKE`          `[epoch:u64][sender_ts_ms:u64]`             (16 B)
//!   - `OP_RECV_ACK`      `[epoch:u64][last_seq:u32][ts_ms:u64]`      (20 B)
//!   - `OP_EOT_ACK`       `[epoch:u64][up_to_seq:u32]`                (12 B)
//!   - `OP_HEARTBEAT`     `[epoch:u64][seq:u32][ts_ms:u64][state:u8][rtt:u16][caps:u8]` (24 B)
//!
//! Every parser here is total: it validates length before indexing and returns
//! `None` rather than panicking, because these bytes arrive from the network.

use crate::protocol::{
    encode_tlv, OP_EOT_ACK, OP_HEARTBEAT, OP_PTT_START_V2, OP_PTT_STOP_V2, OP_RECV_ACK, OP_WAKE,
};

/// Presence advertised in the heartbeat. Values match Android `PresenceState`.
pub const PRESENCE_IDLE: u8 = 0;
pub const PRESENCE_LISTENING: u8 = 1;
pub const PRESENCE_SPEAKING: u8 = 2;
pub const PRESENCE_MUTED: u8 = 3;
pub const PRESENCE_AWAY: u8 = 4;
pub const PRESENCE_BACKGROUNDED: u8 = 5;
pub const PRESENCE_DND: u8 = 6;

/// A decoded `OP_PTT_START_V2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PttStart {
    pub epoch: u64,
    pub start_seq: u32,
    /// Emergency priority — wins floor arbitration outright.
    pub emergency: bool,
}

/// A decoded `OP_PTT_STOP_V2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PttStop {
    pub epoch: u64,
    pub end_seq: u32,
}

/// A decoded `OP_HEARTBEAT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heartbeat {
    pub epoch: u64,
    pub seq: u32,
    pub ts_ms: u64,
    pub state: u8,
    pub rtt_ms: u16,
    /// Capability bitmap. 0 for peers that predate the trailing caps byte.
    pub caps: u8,
}

/// A decoded `OP_RECV_ACK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvAck {
    pub epoch: u64,
    pub last_seq: u32,
    pub ts_ms: u64,
}

/// A decoded `OP_EOT_ACK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EotAck {
    pub epoch: u64,
    pub up_to_seq: u32,
}

/// A decoded `OP_WAKE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wake {
    pub epoch: u64,
    pub sender_ts_ms: u64,
}

#[inline]
fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

#[inline]
fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

// ── PTT_START_V2 ──────────────────────────────────────────────────────────

/// Encode `OP_PTT_START_V2`. Always emits the 13-byte emergency-aware payload
/// that the shipping Android app emits.
pub fn encode_ptt_start_v2(epoch: u64, start_seq: u32, emergency: bool) -> Vec<u8> {
    let mut p = Vec::with_capacity(13);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&start_seq.to_le_bytes());
    p.push(if emergency { 1 } else { 0 });
    encode_tlv(OP_PTT_START_V2, &p)
}

/// Parse `OP_PTT_START_V2`. Accepts the legacy 12-byte payload (no emergency
/// byte) that the desktop build still emits, reading it as `emergency = false`.
pub fn parse_ptt_start_v2(payload: &[u8]) -> Option<PttStart> {
    if payload.len() < 12 {
        return None;
    }
    Some(PttStart {
        epoch: u64_at(payload, 0)?,
        start_seq: u32_at(payload, 8)?,
        emergency: payload.len() >= 13 && (payload[12] & 0x01) != 0,
    })
}

// ── PTT_STOP_V2 ───────────────────────────────────────────────────────────

pub fn encode_ptt_stop_v2(epoch: u64, end_seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&end_seq.to_le_bytes());
    encode_tlv(OP_PTT_STOP_V2, &p)
}

/// Parse `OP_PTT_STOP_V2`. `protocol.rs` documents the minimum as 8 bytes
/// (epoch only) with trailing bytes ignored; the shipping Android encoder always
/// sends 12, so `end_seq` is reported as 0 when only the epoch is present rather
/// than rejecting an otherwise-valid stop.
pub fn parse_ptt_stop_v2(payload: &[u8]) -> Option<PttStop> {
    if payload.len() < 8 {
        return None;
    }
    Some(PttStop {
        epoch: u64_at(payload, 0)?,
        end_seq: if payload.len() >= 12 {
            u32_at(payload, 8)?
        } else {
            0
        },
    })
}

// ── HEARTBEAT ─────────────────────────────────────────────────────────────

/// Encode `OP_HEARTBEAT` with the trailing capabilities byte. Appending keeps
/// the frame readable by 23-byte-only peers (they ignore the extra byte).
pub fn encode_heartbeat(epoch: u64, seq: u32, ts_ms: u64, state: u8, rtt_ms: u16, caps: u8) -> Vec<u8> {
    let mut p = Vec::with_capacity(24);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&ts_ms.to_le_bytes());
    p.push(state);
    p.extend_from_slice(&rtt_ms.to_le_bytes());
    p.push(caps);
    encode_tlv(OP_HEARTBEAT, &p)
}

pub fn parse_heartbeat(payload: &[u8]) -> Option<Heartbeat> {
    if payload.len() < 23 {
        return None;
    }
    Some(Heartbeat {
        epoch: u64_at(payload, 0)?,
        seq: u32_at(payload, 8)?,
        ts_ms: u64_at(payload, 12)?,
        state: payload[20],
        rtt_ms: u16::from_le_bytes([payload[21], payload[22]]),
        caps: if payload.len() >= 24 { payload[23] } else { 0 },
    })
}

// ── RECV_ACK / EOT_ACK / WAKE ─────────────────────────────────────────────

pub fn encode_recv_ack(epoch: u64, last_seq: u32, ts_ms: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(20);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&last_seq.to_le_bytes());
    p.extend_from_slice(&ts_ms.to_le_bytes());
    encode_tlv(OP_RECV_ACK, &p)
}

pub fn parse_recv_ack(payload: &[u8]) -> Option<RecvAck> {
    if payload.len() < 20 {
        return None;
    }
    Some(RecvAck {
        epoch: u64_at(payload, 0)?,
        last_seq: u32_at(payload, 8)?,
        ts_ms: u64_at(payload, 12)?,
    })
}

pub fn encode_eot_ack(epoch: u64, up_to_seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&up_to_seq.to_le_bytes());
    encode_tlv(OP_EOT_ACK, &p)
}

pub fn parse_eot_ack(payload: &[u8]) -> Option<EotAck> {
    if payload.len() < 12 {
        return None;
    }
    Some(EotAck {
        epoch: u64_at(payload, 0)?,
        up_to_seq: u32_at(payload, 8)?,
    })
}

pub fn encode_wake(epoch: u64, sender_ts_ms: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(16);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&sender_ts_ms.to_le_bytes());
    encode_tlv(OP_WAKE, &p)
}

pub fn parse_wake(payload: &[u8]) -> Option<Wake> {
    if payload.len() < 16 {
        return None;
    }
    Some(Wake {
        epoch: u64_at(payload, 0)?,
        sender_ts_ms: u64_at(payload, 8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::parse_tlv;

    #[test]
    fn ptt_start_round_trips_with_emergency() {
        let frame = encode_ptt_start_v2(0xDEAD_BEEF_CAFE_F00D, 42, true);
        let tlv = parse_tlv(&frame).unwrap();
        assert_eq!(tlv.opcode, OP_PTT_START_V2);
        assert_eq!(tlv.payload.len(), 13, "Android emits a 13-byte START_V2");
        let parsed = parse_ptt_start_v2(tlv.payload).unwrap();
        assert_eq!(parsed.epoch, 0xDEAD_BEEF_CAFE_F00D);
        assert_eq!(parsed.start_seq, 42);
        assert!(parsed.emergency);
    }

    /// The desktop build still emits a 12-byte START_V2. Receivers must accept
    /// it rather than dropping the floor request entirely.
    #[test]
    fn ptt_start_accepts_legacy_twelve_byte_payload() {
        let mut p = Vec::new();
        p.extend_from_slice(&7u64.to_le_bytes());
        p.extend_from_slice(&3u32.to_le_bytes());
        let parsed = parse_ptt_start_v2(&p).unwrap();
        assert_eq!(parsed.epoch, 7);
        assert_eq!(parsed.start_seq, 3);
        assert!(!parsed.emergency);
    }

    #[test]
    fn ptt_stop_round_trips() {
        let frame = encode_ptt_stop_v2(9, 1234);
        let tlv = parse_tlv(&frame).unwrap();
        assert_eq!(tlv.opcode, OP_PTT_STOP_V2);
        let parsed = parse_ptt_stop_v2(tlv.payload).unwrap();
        assert_eq!(parsed.epoch, 9);
        assert_eq!(parsed.end_seq, 1234);
    }

    #[test]
    fn heartbeat_round_trips_with_caps() {
        let frame = encode_heartbeat(11, 5, 1_700_000_000_000, PRESENCE_SPEAKING, 250, 0x01);
        let tlv = parse_tlv(&frame).unwrap();
        assert_eq!(tlv.opcode, OP_HEARTBEAT);
        assert_eq!(tlv.payload.len(), 24);
        let hb = parse_heartbeat(tlv.payload).unwrap();
        assert_eq!(hb.epoch, 11);
        assert_eq!(hb.seq, 5);
        assert_eq!(hb.ts_ms, 1_700_000_000_000);
        assert_eq!(hb.state, PRESENCE_SPEAKING);
        assert_eq!(hb.rtt_ms, 250);
        assert_eq!(hb.caps, 0x01);
    }

    /// Backward compatibility with peers that predate the caps byte.
    #[test]
    fn heartbeat_tolerates_missing_caps_byte() {
        let mut p = Vec::new();
        p.extend_from_slice(&1u64.to_le_bytes());
        p.extend_from_slice(&2u32.to_le_bytes());
        p.extend_from_slice(&3u64.to_le_bytes());
        p.push(PRESENCE_IDLE);
        p.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(p.len(), 23);
        assert_eq!(parse_heartbeat(&p).unwrap().caps, 0);
    }

    #[test]
    fn acks_and_wake_round_trip() {
        let ra = parse_recv_ack(parse_tlv(&encode_recv_ack(1, 2, 3)).unwrap().payload).unwrap();
        assert_eq!((ra.epoch, ra.last_seq, ra.ts_ms), (1, 2, 3));

        let ea = parse_eot_ack(parse_tlv(&encode_eot_ack(4, 5)).unwrap().payload).unwrap();
        assert_eq!((ea.epoch, ea.up_to_seq), (4, 5));

        let wk = parse_wake(parse_tlv(&encode_wake(6, 7)).unwrap().payload).unwrap();
        assert_eq!((wk.epoch, wk.sender_ts_ms), (6, 7));
    }

    /// Truncated frames arrive from the network; parsers must never panic.
    #[test]
    fn parsers_reject_short_payloads_without_panicking() {
        for n in 0..24usize {
            let short = vec![0u8; n];
            if n < 12 {
                assert!(parse_ptt_start_v2(&short).is_none());
            }
            if n < 8 {
                assert!(parse_ptt_stop_v2(&short).is_none());
            }
            if n < 23 {
                assert!(parse_heartbeat(&short).is_none());
            }
            if n < 20 {
                assert!(parse_recv_ack(&short).is_none());
            }
            if n < 12 {
                assert!(parse_eot_ack(&short).is_none());
            }
            if n < 16 {
                assert!(parse_wake(&short).is_none());
            }
        }
    }
}
