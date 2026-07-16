// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IY5GUIRKI2FK
//! Wire-protocol opcodes shared by every SassyTalkie endpoint.
//!
//! Every constant in this file is on the wire. Changing a value is a
//! breaking protocol change — old clients won't recognise the new opcode and
//! relay/peer interop silently fails. To extend, add a new constant; never
//! repurpose an existing one.
//!
//! Why this lives in `sassytalkie-core` and not in the consumer crates:
//! the Android `android-native/src/cellular_transport.rs` and the desktop
//! `tauri-desktop/src-tauri/src/transport/control.rs` had duplicated
//! constants that drifted — Android added `OP_WAKE` (0x17) and
//! `OP_REPLAY_FRAME` (0x19) but the desktop file never picked them up.
//! By sourcing both from this module, additions on either side propagate
//! automatically.

// ── Legacy single-byte opcodes ────────────────────────────────────────────
// These predate the TLV-framed v2 protocol. Frames consist of just the
// opcode byte (no length prefix, no payload). Receivers MUST treat any
// inbound byte < 0x10 as a legacy opcode for backward compatibility.

/// Legacy PTT-start (no epoch / seq). Superseded by [OP_PTT_START_V2].
pub const OP_PTT_START: u8 = 0x01;
/// Legacy PTT-stop. Superseded by [OP_PTT_STOP_V2].
pub const OP_PTT_STOP:  u8 = 0x02;

// ── TLV-framed v2+ opcodes ────────────────────────────────────────────────
// Wire format for every opcode in this block:
//   [opcode:u8] [payload_len:u16 LE] [payload bytes]
// Receivers MUST validate `payload_len` matches the expected shape for the
// opcode before acting on it; a coincidental byte-0 match on a random
// encrypted-audio nonce will otherwise misfire (~1/256 of audio frames).

/// Periodic heartbeat (keepalive + presence advertisement). Payload includes
/// the sender's epoch + sequence + capabilities bitmap.
pub const OP_HEARTBEAT: u8 = 0x10;

/// Peer dropped from the room — relay → remaining peers. Payload:
///   [peer_id_len:u8] [peer_id bytes (UTF-8)]
/// Variable length, so receivers must read `peer_id_len` to know how far
/// to advance.
pub const OP_PARTNER_OFFLINE: u8 = 0x14;

/// PTT start (v2 with epoch + start_seq for replay-rejection on the receiver
/// side). Payload: `[epoch:u64][start_seq:u32]` (12 bytes fixed).
pub const OP_PTT_START_V2: u8 = 0x15;

/// PTT stop (v2 with epoch). Payload: `[epoch:u64]` (8 bytes fixed). Earlier
/// drafts also carried a final-seq trailer; current consumers treat any
/// payload `>= 8` as valid and ignore trailing bytes.
pub const OP_PTT_STOP_V2: u8 = 0x16;

/// Wake-push trigger — the sender is starting a transmission and wants the
/// relay to fan out FCM wake-pushes to any offline peers. Payload:
/// `[epoch:u64][sender_ts_ms:u64]` (16 bytes fixed).
pub const OP_WAKE: u8 = 0x17;

/// Replayed audio frame from the relay's per-peer ring buffer (catch-up
/// after a reconnect). Variable length. Wire layout:
///   [0x19] [peer_id_len:u16 LE] [peer_id bytes] [original_audio_frame]
/// The trailing `original_audio_frame` is the encrypted audio (nonce +
/// ciphertext + tag) exactly as the originating peer sent it.
pub const OP_REPLAY_FRAME: u8 = 0x19;

// ── Helpers ───────────────────────────────────────────────────────────────

/// Returns true when an opcode byte is in the TLV-framed v2 range (>= 0x10).
/// Below that, callers should treat the byte as a legacy single-byte frame.
#[inline]
pub fn is_tlv_opcode(op: u8) -> bool { op >= 0x10 }

/// Encode a payload as `[op][len:u16 LE][payload...]`.
///
/// Returns the full wire frame. Allocates one Vec.
pub fn encode_tlv(op: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut out = Vec::with_capacity(3 + payload.len());
    out.push(op);
    out.push((len & 0xFF) as u8);
    out.push(((len >> 8) & 0xFF) as u8);
    out.extend_from_slice(payload);
    out
}

/// Decoded TLV frame view — `payload` borrows into the caller's buffer.
#[derive(Debug, Clone, Copy)]
pub struct Tlv<'a> {
    pub opcode: u8,
    pub payload: &'a [u8],
}

/// Parse the first TLV frame in `bytes`. Returns `None` if the buffer is
/// too short, advertises a length past its own end, or the opcode is < 0x10
/// (use the legacy path for those).
pub fn parse_tlv(bytes: &[u8]) -> Option<Tlv<'_>> {
    if bytes.len() < 3 { return None; }
    if !is_tlv_opcode(bytes[0]) { return None; }
    let payload_len = (bytes[1] as usize) | ((bytes[2] as usize) << 8);
    if bytes.len() < 3 + payload_len { return None; }
    Some(Tlv { opcode: bytes[0], payload: &bytes[3..3 + payload_len] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_opcodes_are_below_tlv_range() {
        assert!(!is_tlv_opcode(OP_PTT_START));
        assert!(!is_tlv_opcode(OP_PTT_STOP));
        assert!(is_tlv_opcode(OP_HEARTBEAT));
        assert!(is_tlv_opcode(OP_PTT_START_V2));
        assert!(is_tlv_opcode(OP_PTT_STOP_V2));
        assert!(is_tlv_opcode(OP_WAKE));
        assert!(is_tlv_opcode(OP_PARTNER_OFFLINE));
        assert!(is_tlv_opcode(OP_REPLAY_FRAME));
    }

    #[test]
    fn tlv_round_trip() {
        let payload = [1u8, 2, 3, 4, 5];
        let frame = encode_tlv(OP_HEARTBEAT, &payload);
        let parsed = parse_tlv(&frame).unwrap();
        assert_eq!(parsed.opcode, OP_HEARTBEAT);
        assert_eq!(parsed.payload, &payload);
    }

    #[test]
    fn parse_tlv_rejects_truncated_payload() {
        // Claims 10-byte payload but only 3 bytes after header.
        let bytes = [OP_HEARTBEAT, 10, 0, 0xAA, 0xBB, 0xCC];
        assert!(parse_tlv(&bytes).is_none());
    }

    #[test]
    fn parse_tlv_rejects_legacy_opcode() {
        let bytes = [OP_PTT_START, 0, 0];
        assert!(parse_tlv(&bytes).is_none());
    }
}
