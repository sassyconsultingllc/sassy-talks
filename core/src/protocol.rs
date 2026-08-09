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
/// Receiver acknowledges a PTT-start (BLE signalling handshake).
pub const OP_READY_ACK: u8 = 0x03;
/// Liveness ping (legacy; superseded by [OP_HEARTBEAT]).
pub const OP_PING: u8 = 0x04;
/// Channel-change announcement (legacy).
pub const OP_CHANNEL_SYNC: u8 = 0x05;

// ── TLV-framed v2+ opcodes ────────────────────────────────────────────────
// Wire format for every opcode in this block:
//   [opcode:u8] [payload_len:u16 LE] [payload bytes]
// Receivers MUST validate `payload_len` matches the expected shape for the
// opcode before acting on it; a coincidental byte-0 match on a random
// encrypted-audio nonce will otherwise misfire (~1/256 of audio frames).

/// Periodic heartbeat (keepalive + presence advertisement). Payload includes
/// the sender's epoch + sequence + capabilities bitmap.
pub const OP_HEARTBEAT: u8 = 0x10;

/// Receiver → sender, during an active transmission: "your audio is landing".
/// Payload: `[epoch:u64][seq:u32][ts_ms:u64]`.
pub const OP_RECV_ACK: u8 = 0x11;

/// End-of-transmission acknowledgement — receiver confirms it drained through
/// the final frame. Payload: `[epoch:u64][up_to_seq:u32]`.
pub const OP_EOT_ACK: u8 = 0x12;

/// Peer capability advertisement (codec, sample rate, feature bits). JSON body.
pub const OP_CAPABILITIES: u8 = 0x13;

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

// ── Life-safety + key-agreement opcodes ───────────────────────────────────
//
// HISTORY — read before allocating another opcode. `emergency.rs` originally
// self-allocated 0x1A/0x1B/0x1C after checking only the constants in THIS
// file. But the Kotlin `ControlFrame` carried a second, larger registry that
// had never been promoted here, and it already used 0x1B/0x1C for the hybrid
// PQC handshake. Wiring emergency at those values would have routed a
// man-down beacon into `handleHybridInit` — a life-safety frame parsed as a
// key exchange. Man-down moved to 0x1D and clear to 0x1E, and every opcode
// on the wire now lives in this file so a partial registry can't recur.
// `all_opcodes_are_unique` below is the regression guard.

/// Manual SOS / distress beacon. Payload: an `emergency::EmergencySignal`,
/// AEAD-sealed when a session key exists (see `emergency_seal`).
pub const OP_EMERGENCY: u8 = 0x1A;

/// Hybrid PQC handshake, initiator → responder (X25519 + ML-KEM-768).
pub const OP_HYBRID_INIT: u8 = 0x1B;

/// Hybrid PQC handshake, responder → initiator.
pub const OP_HYBRID_RESP: u8 = 0x1C;

/// Automatic man-down trip beacon. Same `EmergencySignal` body as
/// [OP_EMERGENCY]; the distinct opcode lets a receiver escalate an automatic
/// trip differently from a deliberate press without parsing the body first.
pub const OP_MANDOWN: u8 = 0x1D;

/// Stand-down / "I'm OK" for a prior beacon from this sender. Payload: an
/// `emergency::EmergencyClear`.
pub const OP_EMERGENCY_CLEAR: u8 = 0x1E;

/// Every opcode this protocol defines, for uniqueness checking and for
/// consumers that need to validate an inbound byte against the whole set.
/// Keep in sync when adding an opcode — `all_opcodes_are_unique` fails loudly
/// if a value is reused, which is the failure this array exists to prevent.
pub const ALL_OPCODES: &[(&str, u8)] = &[
    ("PTT_START", OP_PTT_START),
    ("PTT_STOP", OP_PTT_STOP),
    ("READY_ACK", OP_READY_ACK),
    ("PING", OP_PING),
    ("CHANNEL_SYNC", OP_CHANNEL_SYNC),
    ("HEARTBEAT", OP_HEARTBEAT),
    ("RECV_ACK", OP_RECV_ACK),
    ("EOT_ACK", OP_EOT_ACK),
    ("CAPABILITIES", OP_CAPABILITIES),
    ("PARTNER_OFFLINE", OP_PARTNER_OFFLINE),
    ("PTT_START_V2", OP_PTT_START_V2),
    ("PTT_STOP_V2", OP_PTT_STOP_V2),
    ("WAKE", OP_WAKE),
    ("REPLAY_FRAME", OP_REPLAY_FRAME),
    ("EMERGENCY", OP_EMERGENCY),
    ("HYBRID_INIT", OP_HYBRID_INIT),
    ("HYBRID_RESP", OP_HYBRID_RESP),
    ("MANDOWN", OP_MANDOWN),
    ("EMERGENCY_CLEAR", OP_EMERGENCY_CLEAR),
];

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

    /// The guard that would have caught the emergency/hybrid collision before
    /// it reached a build. Every opcode must be unique across the WHOLE
    /// registry — legacy and TLV alike — because a receiver dispatches on the
    /// raw byte with no other discriminator.
    #[test]
    fn all_opcodes_are_unique() {
        for (i, (name_a, op_a)) in ALL_OPCODES.iter().enumerate() {
            for (name_b, op_b) in ALL_OPCODES.iter().skip(i + 1) {
                assert_ne!(
                    op_a, op_b,
                    "opcode collision: {name_a} and {name_b} both use {op_a:#04x}"
                );
            }
        }
    }

    /// Life-safety opcodes must sit in 0x10..=0x1F. Consumers route only that
    /// window to the control-frame handler (see the relay client's
    /// `op in 0x10..0x1F` check); an emergency frame outside it would be
    /// handed to the audio decoder and dropped.
    #[test]
    fn emergency_opcodes_are_in_the_control_routing_window() {
        for op in [OP_EMERGENCY, OP_MANDOWN, OP_EMERGENCY_CLEAR] {
            assert!(is_tlv_opcode(op), "{op:#04x} must be TLV-framed");
            assert!(
                (0x10..=0x1F).contains(&op),
                "{op:#04x} must be inside the 0x10..0x1F control-routing window"
            );
        }
    }

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
