// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! Authenticated control envelope (opcode 0x18).
//!
//! Byte-identical to Android `AuthenticatedControlCodec`. AAD binds protocol
//! version, opcode, room, sender, epoch, and sequence. Raw 0x10..=0x1F frames
//! must not be executed as privileged ops — open this envelope first.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::protocol::{encode_tlv, OP_AUTHENTICATED};

const MAGIC: &[u8; 4] = b"STCP";
const OUTER_AAD: &[u8] = b"sassytalkie-control-envelope-v1";
const VERSION: u8 = 1;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const ROOM_BINDING_BYTES: usize = 16;
const MAX_SENDER_BYTES: usize = 64;
const MAX_INNER_BYTES: usize = 8 * 1024;
const MAX_REPLAY_SENDERS: usize = 256;
const REPLAY_BITS: u64 = 64;
pub const MAX_AGE_MS: i64 = 2 * 60 * 1000;
pub const MAX_FUTURE_SKEW_MS: i64 = 30 * 1000;
const MIN_BODY_BYTES: usize = 4 + 1 + ROOM_BINDING_BYTES + 1 + 1 + 8 + 8 + 8 + 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedControl {
    pub sender_id: String,
    pub epoch: u64,
    pub sequence: u64,
    pub issued_at_ms: u64,
    pub inner_frame: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAuthError {
    Malformed,
    LegacyNotAllowed,
    TooLarge,
    SequenceExhausted,
    Crypto,
}

/// AES-GCM authenticated wrapper for v2 control frames.
pub struct ControlAuthCodec {
    cipher: Aes256Gcm,
    room_binding: [u8; ROOM_BINDING_BYTES],
    sender_id: String,
    epoch: u64,
    tx_sequence: AtomicU64,
    replay: std::sync::Mutex<ReplayWindow>,
}

impl ControlAuthCodec {
    pub fn new(key: [u8; 32], room_id: &str, sender_id: &str, epoch: u64) -> Result<Self, ControlAuthError> {
        let sender_bytes = sender_id.as_bytes();
        if sender_bytes.is_empty() || sender_bytes.len() > MAX_SENDER_BYTES {
            return Err(ControlAuthError::Malformed);
        }
        let digest = Sha256::digest(room_id.as_bytes());
        let mut room_binding = [0u8; ROOM_BINDING_BYTES];
        room_binding.copy_from_slice(&digest[..ROOM_BINDING_BYTES]);
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&key).map_err(|_| ControlAuthError::Crypto)?,
            room_binding,
            sender_id: sender_id.to_string(),
            epoch,
            tx_sequence: AtomicU64::new(0),
            replay: std::sync::Mutex::new(ReplayWindow::default()),
        })
    }

    pub fn seal(&self, inner_frame: &[u8], now_ms: u64) -> Result<Vec<u8>, ControlAuthError> {
        self.seal_with_rng(inner_frame, now_ms, |buf| rand::rngs::OsRng.fill_bytes(buf))
    }

    pub fn seal_with_rng<F>(&self, inner_frame: &[u8], now_ms: u64, mut fill_nonce: F) -> Result<Vec<u8>, ControlAuthError>
    where
        F: FnMut(&mut [u8]),
    {
        let decoded = decode_control_frame(inner_frame).ok_or(ControlAuthError::Malformed)?;
        if decoded.opcode < 0x10 {
            return Err(ControlAuthError::LegacyNotAllowed);
        }
        if inner_frame.len() > MAX_INNER_BYTES {
            return Err(ControlAuthError::TooLarge);
        }
        let seq = self.tx_sequence.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if seq == 0 {
            return Err(ControlAuthError::SequenceExhausted);
        }
        let sender = self.sender_id.as_bytes();
        let mut body = Vec::with_capacity(
            MAGIC.len() + 1 + ROOM_BINDING_BYTES + 1 + sender.len() + 8 + 8 + 8 + 2 + inner_frame.len(),
        );
        body.extend_from_slice(MAGIC);
        body.push(decoded.opcode);
        body.extend_from_slice(&self.room_binding);
        body.push(sender.len() as u8);
        body.extend_from_slice(sender);
        body.extend_from_slice(&self.epoch.to_le_bytes());
        body.extend_from_slice(&seq.to_le_bytes());
        body.extend_from_slice(&now_ms.to_le_bytes());
        body.extend_from_slice(&(inner_frame.len() as u16).to_le_bytes());
        body.extend_from_slice(inner_frame);

        let mut nonce = [0u8; NONCE_BYTES];
        fill_nonce(&mut nonce);
        let aad = outer_aad(decoded.opcode);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: &body, aad: &aad })
            .map_err(|_| ControlAuthError::Crypto)?;

        let mut payload = Vec::with_capacity(2 + NONCE_BYTES + ciphertext.len());
        payload.push(VERSION);
        payload.push(decoded.opcode);
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&ciphertext);
        Ok(encode_tlv(OP_AUTHENTICATED, &payload))
    }

    pub fn open(&self, envelope: &[u8], now_ms: u64) -> Option<VerifiedControl> {
        let outer = parse_tlv_exact(envelope)?;
        if outer.opcode != OP_AUTHENTICATED {
            return None;
        }
        if outer.payload.len() < 2 + NONCE_BYTES + TAG_BYTES || outer.payload[0] != VERSION {
            return None;
        }
        let opcode_hint = outer.payload[1];
        let nonce = &outer.payload[2..2 + NONCE_BYTES];
        let ciphertext = &outer.payload[2 + NONCE_BYTES..];
        let aad = outer_aad(opcode_hint);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad: &aad })
            .ok()?;
        if plaintext.len() < MIN_BODY_BYTES {
            return None;
        }
        let mut i = 0;
        if &plaintext[i..i + 4] != MAGIC {
            return None;
        }
        i += 4;
        let bound_opcode = plaintext[i];
        i += 1;
        let bound_room = &plaintext[i..i + ROOM_BINDING_BYTES];
        if bound_room != self.room_binding {
            return None;
        }
        i += ROOM_BINDING_BYTES;
        let sender_len = plaintext[i] as usize;
        i += 1;
        if !(1..=MAX_SENDER_BYTES).contains(&sender_len) || plaintext.len() < i + sender_len + 8 + 8 + 8 + 2 {
            return None;
        }
        let sender = std::str::from_utf8(&plaintext[i..i + sender_len]).ok()?.to_string();
        i += sender_len;
        let sender_epoch = u64::from_le_bytes(plaintext[i..i + 8].try_into().ok()?);
        i += 8;
        let sequence = u64::from_le_bytes(plaintext[i..i + 8].try_into().ok()?);
        i += 8;
        let issued_at_ms = u64::from_le_bytes(plaintext[i..i + 8].try_into().ok()?);
        i += 8;
        let inner_len = u16::from_le_bytes(plaintext[i..i + 2].try_into().ok()?) as usize;
        i += 2;
        if inner_len == 0 || inner_len > MAX_INNER_BYTES || plaintext.len() != i + inner_len {
            return None;
        }
        let inner = plaintext[i..].to_vec();
        let decoded = decode_control_frame(&inner)?;
        if decoded.opcode != bound_opcode || decoded.opcode != opcode_hint || decoded.opcode == OP_AUTHENTICATED {
            return None;
        }
        let age = now_ms as i64 - issued_at_ms as i64;
        if age > MAX_AGE_MS || age < -MAX_FUTURE_SKEW_MS {
            return None;
        }
        {
            let mut replay = self.replay.lock().ok()?;
            if !replay.accept(&sender, sender_epoch, sequence) {
                return None;
            }
        }
        Some(VerifiedControl {
            sender_id: sender,
            epoch: sender_epoch,
            sequence,
            issued_at_ms,
            inner_frame: inner,
        })
    }
}

/// Fail-closed: privileged/emergency/rekey opcodes in 0x10..=0x1F must arrive
/// inside an authenticated 0x18 envelope. Legacy 0x01..=0x05 remain hints only.
pub fn reject_unauthenticated_privileged(outer_opcode: u8) -> bool {
    outer_opcode >= 0x10 && outer_opcode != OP_AUTHENTICATED
}

#[derive(Debug)]
pub enum InboundControl {
    /// Audio or unknown non-control bytes.
    NotControl,
    /// Legacy 0x01..=0x05 — non-privileged compatibility hints only.
    LegacyHint { opcode: u8 },
    /// Raw 0x10..=0x1F other than 0x18 — must not be executed.
    RejectedUnauthenticated { opcode: u8 },
    /// 0x18 envelope that failed open (forged, stale, replay, wrong room).
    AuthFailed,
    /// Authenticated inner frame.
    Verified(VerifiedControl),
}

/// Classify an inbound binary frame. `codec == None` still rejects raw privileged ops.
pub fn classify_inbound(codec: Option<&ControlAuthCodec>, bytes: &[u8], now_ms: u64) -> InboundControl {
    if bytes.is_empty() {
        return InboundControl::NotControl;
    }
    let op = bytes[0];
    if op < 0x10 {
        return InboundControl::LegacyHint { opcode: op };
    }
    if !is_control_tlv(bytes) {
        return InboundControl::NotControl;
    }
    if reject_unauthenticated_privileged(op) {
        return InboundControl::RejectedUnauthenticated { opcode: op };
    }
    match codec.and_then(|c| c.open(bytes, now_ms)) {
        Some(verified) => InboundControl::Verified(verified),
        None => InboundControl::AuthFailed,
    }
}

pub fn is_control_tlv(bytes: &[u8]) -> bool {
    if bytes.len() < 3 {
        return false;
    }
    let op = bytes[0];
    if !(0x10..=0x20).contains(&op) {
        return false;
    }
    let len = u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
    bytes.len() == 3 + len
}

#[derive(Debug, Clone, Copy)]
pub struct DecodedControl<'a> {
    pub opcode: u8,
    pub payload: &'a [u8],
}

/// Android `ControlFrame.decode`: exact TLV length, legacy single-byte below 0x10.
pub fn decode_control_frame(bytes: &[u8]) -> Option<DecodedControl<'_>> {
    if bytes.is_empty() {
        return None;
    }
    let op = bytes[0];
    if op < 0x10 {
        return Some(DecodedControl { opcode: op, payload: &[] });
    }
    parse_tlv_exact(bytes)
}

pub fn parse_tlv_exact(bytes: &[u8]) -> Option<DecodedControl<'_>> {
    if bytes.len() < 3 {
        return None;
    }
    if bytes[0] < 0x10 {
        return None;
    }
    let len = u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
    if 3 + len != bytes.len() {
        return None;
    }
    Some(DecodedControl {
        opcode: bytes[0],
        payload: &bytes[3..],
    })
}

fn outer_aad(opcode: u8) -> Vec<u8> {
    let mut aad = Vec::with_capacity(OUTER_AAD.len() + 2);
    aad.extend_from_slice(OUTER_AAD);
    aad.push(VERSION);
    aad.push(opcode);
    aad
}

#[derive(Default)]
struct ReplayWindow {
    states: HashMap<String, ReplayState>,
    order: Vec<String>,
}

struct ReplayState {
    highest: u64,
    bitmap: u64,
}

impl ReplayWindow {
    fn accept(&mut self, sender: &str, epoch: u64, sequence: u64) -> bool {
        if epoch == 0 || sequence == 0 {
            return false;
        }
        let key = format!("{sender}\0{epoch}");
        if let Some(state) = self.states.get_mut(&key) {
            if sequence > state.highest {
                let shift = sequence - state.highest;
                state.bitmap = if shift >= REPLAY_BITS {
                    1
                } else {
                    (state.bitmap << shift) | 1
                };
                state.highest = sequence;
                return true;
            }
            let behind = state.highest - sequence;
            if behind >= REPLAY_BITS {
                return false;
            }
            let bit = 1u64 << behind;
            if state.bitmap & bit != 0 {
                return false;
            }
            state.bitmap |= bit;
            return true;
        }
        if self.states.len() >= MAX_REPLAY_SENDERS {
            if let Some(oldest) = self.order.first().cloned() {
                self.states.remove(&oldest);
                self.order.remove(0);
            }
        }
        self.states.insert(key.clone(), ReplayState { highest: sequence, bitmap: 1 });
        self.order.push(key);
        true
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{encode_tlv, OP_HEARTBEAT, OP_PTT_START, OP_PTT_START_V2, OP_RECV_ACK};

    fn key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        k
    }

    fn recv_ack() -> Vec<u8> {
        let mut p = Vec::with_capacity(20);
        p.extend_from_slice(&11u64.to_le_bytes());
        p.extend_from_slice(&42u32.to_le_bytes());
        p.extend_from_slice(&1_700_000_000_000u64.to_le_bytes());
        encode_tlv(OP_RECV_ACK, &p)
    }

    #[test]
    fn round_trip_binds_sender() {
        let sender = ControlAuthCodec::new(key(), "room-1", "device-a", 11).unwrap();
        let receiver = ControlAuthCodec::new(key(), "room-1", "device-b", 22).unwrap();
        let inner = recv_ack();
        let now = 1_700_000_000_000u64;
        let verified = receiver.open(&sender.seal(&inner, now).unwrap(), now).unwrap();
        assert_eq!(verified.sender_id, "device-a");
        assert_eq!(verified.epoch, 11);
        assert!(verified.sequence > 0);
        assert_eq!(verified.inner_frame, inner);
    }

    #[test]
    fn forged_and_wrong_room_fail_closed() {
        let sender = ControlAuthCodec::new(key(), "room-1", "device-a", 11).unwrap();
        let now = 1_700_000_000_000u64;
        let mut envelope = sender.seal(&encode_tlv(OP_PTT_START_V2, &[0; 12]), now).unwrap();
        let last = envelope.len() - 1;
        envelope[last] ^= 1;
        let receiver = ControlAuthCodec::new(key(), "room-1", "device-b", 22).unwrap();
        assert!(receiver.open(&envelope, now).is_none());

        let valid = sender.seal(&encode_tlv(OP_HEARTBEAT, &[0; 23]), now).unwrap();
        let other_room = ControlAuthCodec::new(key(), "other-room", "device-b", 22).unwrap();
        assert!(other_room.open(&valid, now).is_none());
    }

    #[test]
    fn replay_rejected_reordering_accepted() {
        let sender = ControlAuthCodec::new(key(), "room-1", "device-a", 11).unwrap();
        let receiver = ControlAuthCodec::new(key(), "room-1", "device-b", 22).unwrap();
        let now = 1_700_000_000_000u64;
        let first = sender.seal(&encode_tlv(OP_HEARTBEAT, &[1; 23]), now).unwrap();
        let second = sender.seal(&encode_tlv(OP_HEARTBEAT, &[2; 23]), now).unwrap();
        assert!(receiver.open(&second, now).is_some());
        assert!(receiver.open(&first, now).is_some());
        assert!(receiver.open(&first, now).is_none());
    }

    #[test]
    fn stale_and_future_rejected() {
        let sender = ControlAuthCodec::new(key(), "room-1", "device-a", 11).unwrap();
        let receiver = ControlAuthCodec::new(key(), "room-1", "device-b", 22).unwrap();
        let now = 1_700_000_000_000u64;
        let stale = sender.seal(&encode_tlv(OP_HEARTBEAT, &[0; 23]), now - 121_000).unwrap();
        assert!(receiver.open(&stale, now).is_none());
        let future = sender.seal(&encode_tlv(OP_HEARTBEAT, &[0; 23]), now + 31_000).unwrap();
        assert!(receiver.open(&future, now).is_none());
    }

    #[test]
    fn legacy_cannot_be_wrapped() {
        let sender = ControlAuthCodec::new(key(), "room-1", "device-a", 11).unwrap();
        assert_eq!(
            sender.seal(&[OP_PTT_START], 1_700_000_000_000),
            Err(ControlAuthError::LegacyNotAllowed)
        );
    }

    #[test]
    fn raw_privileged_opcodes_are_rejected() {
        assert!(reject_unauthenticated_privileged(0x10));
        assert!(reject_unauthenticated_privileged(0x1A));
        assert!(reject_unauthenticated_privileged(0x1F));
        assert!(reject_unauthenticated_privileged(0x20));
        assert!(!reject_unauthenticated_privileged(OP_AUTHENTICATED));
        assert!(!reject_unauthenticated_privileged(OP_PTT_START));
    }
}
