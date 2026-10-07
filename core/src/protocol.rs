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

/// AES-GCM authenticated wrapper for every v2 control frame. Inner opcode
/// is bound in AAD; raw 0x10..=0x1f must not be acted on without opening.
pub const OP_AUTHENTICATED: u8 = 0x18;

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

/// Hybrid PQC handshake confirmation. Initiator → responder after the
/// responder message is verified; the responder must not install the
/// proposed session until this authenticated confirm arrives.
/// Payload: `[channel:u8][sha256(responder_message)]`.
pub const OP_HYBRID_CONFIRM: u8 = 0x1F;

/// Hybrid PQC handshake confirm-ack. Responder → initiator after installing
/// on CONFIRM. The initiator must not install the proposed session until
/// this authenticated ack arrives; a lost CONFIRM therefore cannot leave
/// the initiator on a new key the responder never confirmed.
/// Payload: `[channel:u8][sha256(responder_message)]` (same token as CONFIRM).
/// Outer routing window is 0x10..=0x20 so this is not treated as audio.
pub const OP_HYBRID_CONFIRM_ACK: u8 = 0x20;

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
    ("AUTHENTICATED", OP_AUTHENTICATED),
    ("REPLAY_FRAME", OP_REPLAY_FRAME),
    ("EMERGENCY", OP_EMERGENCY),
    ("HYBRID_INIT", OP_HYBRID_INIT),
    ("HYBRID_RESP", OP_HYBRID_RESP),
    ("MANDOWN", OP_MANDOWN),
    ("EMERGENCY_CLEAR", OP_EMERGENCY_CLEAR),
    ("HYBRID_CONFIRM", OP_HYBRID_CONFIRM),
    ("HYBRID_CONFIRM_ACK", OP_HYBRID_CONFIRM_ACK),
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

/// The classifier every relay client uses to route a binary WebSocket message:
/// opcode in `0x10..=0x20` AND a TLV length that exactly accounts for the frame.
/// Sealed audio starts with a random 12-byte nonce, so the joint opcode +
/// exact-length test makes a false positive astronomically rare. Sealed control
/// (`OP_AUTHENTICATED`) is TLV-shaped too and is matched here.
pub fn is_control_frame_shape(bytes: &[u8]) -> bool {
    if bytes.len() < 3 || !(0x10..=0x20).contains(&bytes[0]) {
        return false;
    }
    let payload_len = (bytes[1] as usize) | ((bytes[2] as usize) << 8);
    bytes.len() == 3 + payload_len
}

/// Longest peer id a relay replay header may carry (the relay caps peer ids at
/// 64 bytes). Anything larger is not a replay header.
pub const MAX_REPLAY_PEER_ID_LEN: usize = 64;

/// The original frame inside an [`OP_REPLAY_FRAME`], or `None` when `bytes` is
/// not a well-formed replay frame.
///
/// The relay wraps catch-up frames as `[0x19][peer_id_len:u16 LE][peer_id][original]`
/// (NOT a TLV — the length covers only the peer id). Clients must unwrap before
/// decrypting: handed to the AEAD as-is, the 3-byte header shifts the nonce and
/// every replayed frame fails authentication.
///
/// A live sealed-audio frame can begin with 0x19 (random nonce byte), so the
/// header must also carry a plausible peer-id length and leave a frame at least
/// as long as an AES-GCM nonce + tag; the residual collision rate is ~4e-6.
///
/// Callers should drop a replayed frame that [`is_control_frame_shape`]: control
/// is time-sensitive (a stale `PTT_START` would grab the floor) and only audio
/// is meant to be caught up.
pub fn replay_inner(bytes: &[u8]) -> Option<&[u8]> {
    const MIN_SEALED_FRAME_LEN: usize = 12 + 16;
    if bytes.len() < 3 || bytes[0] != OP_REPLAY_FRAME {
        return None;
    }
    let id_len = (bytes[1] as usize) | ((bytes[2] as usize) << 8);
    if id_len > MAX_REPLAY_PEER_ID_LEN {
        return None;
    }
    let inner = bytes.get(3 + id_len..)?;
    if inner.len() < MIN_SEALED_FRAME_LEN {
        return None;
    }
    Some(inner)
}

/// How far back a reconnecting client may ask the relay to replay. The relay
/// retains ~30 s; a longer gap is "you missed that transmission", and replaying
/// it seconds-late as a burst would sound like a ghost transmission.
pub const MAX_CATCHUP_GAP_MS: u64 = 15_000;

/// The `since=` cursor a reconnecting client should send, given when its last
/// socket was last known to be alive. `None` means "don't ask for catch-up":
/// never connected, or the gap is longer than [`MAX_CATCHUP_GAP_MS`]. A 250 ms
/// margin covers the frame in flight when the socket died; anything replayed
/// that was already heard is rejected by the AEAD replay window.
pub fn catchup_since_ms(last_alive_ms: u64, now_ms: u64) -> Option<u64> {
    if last_alive_ms == 0 || now_ms.saturating_sub(last_alive_ms) > MAX_CATCHUP_GAP_MS {
        return None;
    }
    Some(last_alive_ms.saturating_sub(250))
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
        assert!(is_tlv_opcode(OP_HYBRID_CONFIRM_ACK));
        assert!((0x10..=0x20).contains(&OP_HYBRID_CONFIRM_ACK));
    }

    #[test]
    fn tlv_round_trip() {
        let payload = [1u8, 2, 3, 4, 5];
        let frame = encode_tlv(OP_HEARTBEAT, &payload);
        let parsed = parse_tlv(&frame).unwrap();
        assert_eq!(parsed.opcode, OP_HEARTBEAT);
        assert_eq!(parsed.payload, &payload);
    }

    /// Mirrors `buildReplayFrame` in cloudflare-worker/src/ptt-relay.js.
    fn relay_replay(peer: &str, original: &[u8]) -> Vec<u8> {
        let mut out = vec![OP_REPLAY_FRAME, peer.len() as u8, (peer.len() >> 8) as u8];
        out.extend_from_slice(peer.as_bytes());
        out.extend_from_slice(original);
        out
    }

    #[test]
    fn replay_inner_unwraps_relay_frames() {
        let sealed_audio: Vec<u8> = (0u8..60).collect();
        assert_eq!(replay_inner(&relay_replay("", &sealed_audio)), Some(&sealed_audio[..]));
        assert_eq!(
            replay_inner(&relay_replay("peer-1234", &sealed_audio)),
            Some(&sealed_audio[..])
        );
    }

    #[test]
    fn replay_inner_rejects_non_replay_bytes() {
        let sealed_audio: Vec<u8> = (0u8..60).collect();
        assert_eq!(replay_inner(&sealed_audio), None, "first byte is not 0x19");
        assert_eq!(replay_inner(&[OP_REPLAY_FRAME, 0]), None, "truncated header");
        // A live frame whose random nonce happens to start 0x19 and whose next
        // two bytes read as a huge "peer id length" is not a replay frame.
        let mut live = sealed_audio.clone();
        live[0] = OP_REPLAY_FRAME;
        live[1] = 0xFF;
        live[2] = 0x40;
        assert_eq!(replay_inner(&live), None);
        // Header present but nothing worth decrypting after it.
        assert_eq!(replay_inner(&relay_replay("", &[1, 2, 3])), None);
        let too_long_id = "x".repeat(MAX_REPLAY_PEER_ID_LEN + 1);
        assert_eq!(replay_inner(&relay_replay(&too_long_id, &sealed_audio)), None);
    }

    #[test]
    fn replayed_control_is_recognisable_so_clients_can_drop_it() {
        let sealed_control = encode_tlv(OP_AUTHENTICATED, &[7u8; 40]);
        let wrapped_control = relay_replay("", &sealed_control);
        assert!(is_control_frame_shape(replay_inner(&wrapped_control).unwrap()));
        let audio: Vec<u8> = (0u8..60).collect();
        let wrapped = relay_replay("", &audio);
        assert!(!is_control_frame_shape(replay_inner(&wrapped).unwrap()));
    }

    #[test]
    fn control_shape_requires_exact_length() {
        let hb = encode_tlv(OP_HEARTBEAT, &[0u8; 24]);
        assert!(is_control_frame_shape(&hb));
        let mut longer = hb.clone();
        longer.push(0);
        assert!(!is_control_frame_shape(&longer));
        assert!(!is_control_frame_shape(&encode_tlv(0x21, &[0u8; 4])));
        assert!(!is_control_frame_shape(&[OP_HEARTBEAT, 0]));
    }

    #[test]
    fn catchup_cursor_only_bridges_short_gaps() {
        let t = 1_700_000_000_000u64;
        assert_eq!(catchup_since_ms(0, t), None, "never connected");
        assert_eq!(catchup_since_ms(t - 2_000, t), Some(t - 2_250));
        assert_eq!(catchup_since_ms(t - MAX_CATCHUP_GAP_MS, t), Some(t - MAX_CATCHUP_GAP_MS - 250));
        assert_eq!(catchup_since_ms(t - MAX_CATCHUP_GAP_MS - 1, t), None, "gap too long");
        // Clock stepped backwards: still a short gap, still a usable cursor.
        assert_eq!(catchup_since_ms(t + 500, t), Some(t + 250));
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
