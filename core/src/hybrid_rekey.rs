// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! Staged hybrid/PQC rekey: neither side TXes on the new key until the
//! 4-way completes without a split. Auto-PQC initiation stays off unless a
//! caller explicitly enables it.
//!
//! Flow:
//! 1. INIT — responder stages (TX stays on the old key)
//! 2. RESP — initiator stages, sends CONFIRM (retried until ACK or TTL)
//! 3. CONFIRM — responder emits CONFIRM_ACK, keeps the new key as RX-only
//! 4. CONFIRM_ACK — initiator installs (TX+RX new). Responder installs TX
//!    on the new key only after seeing peer ciphertext (or an explicit commit).
//!
//! A lost CONFIRM or lost ACK therefore cannot leave one peer TXing on a
//! key the other never installed. Staged keys expire together.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::protocol::{
    encode_tlv, OP_HYBRID_CONFIRM, OP_HYBRID_CONFIRM_ACK, OP_HYBRID_INIT, OP_HYBRID_RESP,
};

pub const STAGED_TTL_MS: u64 = 15_000;
pub const CONFIRM_RETRY_MAX: u8 = 3;
pub const CONFIRM_RETRY_MS: u64 = 1_000;

/// SHA-256 of the responder handshake message — the confirm token.
pub fn token_for(responder_message: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(responder_message);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn confirm_acceptable(
    staged_token: Option<&[u8]>,
    confirm_token: Option<&[u8]>,
    now_ms: u64,
    staged_at_ms: u64,
) -> bool {
    let staged = match staged_token {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    let confirm = match confirm_token {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    if confirm.len() != staged.len() {
        return false;
    }
    if now_ms < staged_at_ms || now_ms.saturating_sub(staged_at_ms) > STAGED_TTL_MS {
        return false;
    }
    bool::from(staged.ct_eq(confirm))
}

pub fn ack_acceptable(
    staged_token: Option<&[u8]>,
    ack_token: Option<&[u8]>,
    now_ms: u64,
    staged_at_ms: u64,
) -> bool {
    confirm_acceptable(staged_token, ack_token, now_ms, staged_at_ms)
}

pub fn encode_hybrid_frame(op: u8, channel: u8, msg: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(1 + msg.len());
    p.push(channel);
    p.extend_from_slice(msg);
    encode_tlv(op, &p)
}

pub fn parse_hybrid_frame(payload: &[u8]) -> Option<(u8, &[u8])> {
    if payload.is_empty() {
        return None;
    }
    Some((payload[0], &payload[1..]))
}

/// Auto-PQC remains off. Callers may flip this only with an explicit product setting.
pub const AUTO_PQC_DEFAULT: bool = false;

pub fn is_hybrid_opcode(op: u8) -> bool {
    matches!(
        op,
        OP_HYBRID_INIT | OP_HYBRID_RESP | OP_HYBRID_CONFIRM | OP_HYBRID_CONFIRM_ACK
    )
}

/// Four-way hybrid install policy. TX on the new key is live only after
/// CONFIRM+ACK on the initiator and after peer-commit on the responder.
/// A lost CONFIRM or lost ACK therefore cannot permanently split keys.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FourWayHybrid {
    pub initiator_staged: bool,
    pub responder_staged: bool,
    pub initiator_live: bool,
    pub responder_live: bool,
    pub ack_sent: bool,
    pub confirm_retries: u8,
}

impl FourWayHybrid {
    pub fn on_init_at_responder(&mut self) {
        self.responder_staged = true;
        self.responder_live = false;
        self.ack_sent = false;
    }

    /// Initiator verified RESP: stage only. Do **not** install. Send CONFIRM.
    pub fn on_resp_at_initiator(&mut self) {
        self.initiator_staged = true;
        self.initiator_live = false;
        self.confirm_retries = 0;
    }

    /// Responder accepted CONFIRM: emit ACK, keep staged for RX, do **not**
    /// TX on the new key until the initiator is known to have installed.
    pub fn on_confirm_at_responder(&mut self) -> bool {
        if !self.responder_staged {
            return false;
        }
        self.ack_sent = true;
        true
    }

    /// Duplicate CONFIRM (initiator retry after lost ACK): re-emit ACK.
    pub fn on_confirm_retry_at_responder(&mut self) -> bool {
        self.ack_sent && self.responder_staged && !self.responder_live
    }

    /// Initiator accepted CONFIRM_ACK: install the staged session.
    pub fn on_ack_at_initiator(&mut self) -> bool {
        if !self.initiator_staged {
            return false;
        }
        self.initiator_staged = false;
        self.initiator_live = true;
        true
    }

    /// Responder may TX on the new key after the initiator is live (ACK
    /// delivered) — typically observed as ciphertext on the staged key.
    pub fn on_peer_new_key_at_responder(&mut self) -> bool {
        if !self.ack_sent || !self.responder_staged {
            return false;
        }
        self.responder_staged = false;
        self.responder_live = true;
        true
    }

    pub fn should_retry_confirm(&self, now_ms: u64, staged_at_ms: u64) -> bool {
        self.initiator_staged
            && !self.initiator_live
            && self.confirm_retries < CONFIRM_RETRY_MAX
            && now_ms >= staged_at_ms
            && now_ms.saturating_sub(staged_at_ms) <= STAGED_TTL_MS
    }

    pub fn note_confirm_retry(&mut self) {
        self.confirm_retries = self.confirm_retries.saturating_add(1);
    }

    pub fn expire_initiator_stage(&mut self) {
        if !self.initiator_live {
            self.initiator_staged = false;
        }
    }

    pub fn expire_responder_stage(&mut self) {
        if !self.responder_live {
            self.responder_staged = false;
            self.ack_sent = false;
        }
    }

    /// Drop both staged keys together. Live TX keys are left untouched.
    pub fn expire_together(&mut self) {
        self.expire_initiator_stage();
        self.expire_responder_stage();
    }

    /// Permanent TX split: one side live on the new key, the other not staged
    /// and not live (stuck on the old key with no way to catch up).
    pub fn tx_keys_split(&self) -> bool {
        match (self.initiator_live, self.responder_live) {
            (true, true) | (false, false) => false,
            (true, false) => !self.responder_staged && !self.ack_sent,
            (false, true) => !self.initiator_staged,
        }
    }

    /// Two-party view of [tx_keys_split] after each side has only filled in
    /// its own role flags.
    pub fn pair_tx_split(initiator: &Self, responder: &Self) -> bool {
        match (initiator.initiator_live, responder.responder_live) {
            (true, true) | (false, false) => false,
            (true, false) => !responder.responder_staged && !responder.ack_sent,
            (false, true) => !initiator.initiator_staged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_confirm_installs_while_live() {
        let token = token_for(&[1, 2, 3, 4]);
        let now = 1_700_000_000_000u64;
        assert!(confirm_acceptable(Some(&token), Some(&token), now, now));
        assert!(ack_acceptable(Some(&token), Some(&token), now, now));
    }

    #[test]
    fn forged_confirm_rejected() {
        let staged = token_for(&[1, 2, 3, 4]);
        let forged = token_for(&[9, 9, 9, 9]);
        let now = 1_700_000_000_000u64;
        assert!(!confirm_acceptable(Some(&staged), Some(&forged), now, now));
        assert!(!confirm_acceptable(Some(&staged), None, now, now));
        assert!(!confirm_acceptable(None, Some(&staged), now, now));
    }

    #[test]
    fn stale_confirm_after_ttl_rejected() {
        let token = token_for(&[7]);
        let staged_at = 1_700_000_000_000u64;
        let late = staged_at + STAGED_TTL_MS + 1;
        assert!(!confirm_acceptable(Some(&token), Some(&token), late, staged_at));
        assert!(!confirm_acceptable(Some(&token), Some(&token), staged_at.saturating_sub(1), staged_at));
    }

    #[test]
    fn hybrid_frame_round_trip() {
        let frame = encode_hybrid_frame(OP_HYBRID_CONFIRM, 3, &token_for(&[1]));
        let decoded = crate::control_auth::decode_control_frame(&frame).unwrap();
        assert_eq!(decoded.opcode, OP_HYBRID_CONFIRM);
        let (ch, msg) = parse_hybrid_frame(decoded.payload).unwrap();
        assert_eq!(ch, 3);
        assert_eq!(msg, token_for(&[1]));
    }

    #[test]
    fn auto_pqc_remains_off() {
        assert!(!AUTO_PQC_DEFAULT);
    }

    #[test]
    fn lost_confirm_leaves_both_on_old_key() {
        let mut initiator = FourWayHybrid::default();
        let mut responder = FourWayHybrid::default();
        responder.on_init_at_responder();
        initiator.on_resp_at_initiator();
        assert!(!initiator.initiator_live);
        assert!(!responder.responder_live);
        initiator.expire_together();
        responder.expire_together();
        assert!(!initiator.initiator_live);
        assert!(!initiator.initiator_staged);
        assert!(!responder.responder_live);
        assert!(!responder.responder_staged);
        assert!(!FourWayHybrid::pair_tx_split(&initiator, &responder));
    }

    #[test]
    fn lost_ack_does_not_split_tx_keys() {
        let mut initiator = FourWayHybrid::default();
        let mut responder = FourWayHybrid::default();
        responder.on_init_at_responder();
        initiator.on_resp_at_initiator();
        assert!(responder.on_confirm_at_responder());
        assert!(responder.ack_sent);
        assert!(!responder.responder_live);
        assert!(!initiator.initiator_live);
        // ACK never arrives. Initiator rolls back; responder drops unused stage.
        initiator.expire_together();
        responder.expire_together();
        assert!(!initiator.initiator_live);
        assert!(!responder.responder_live);
        assert!(!FourWayHybrid::pair_tx_split(&initiator, &responder));
    }

    #[test]
    fn late_ack_within_ttl_installs_initiator_then_responder() {
        let mut initiator = FourWayHybrid::default();
        let mut responder = FourWayHybrid::default();
        responder.on_init_at_responder();
        initiator.on_resp_at_initiator();
        assert!(responder.on_confirm_at_responder());
        assert!(initiator.on_ack_at_initiator());
        assert!(initiator.initiator_live);
        assert!(!responder.responder_live);
        assert!(responder.on_peer_new_key_at_responder());
        assert!(responder.responder_live);
        assert!(!FourWayHybrid::pair_tx_split(&initiator, &responder));
    }

    #[test]
    fn forged_or_unstaged_ack_does_not_install() {
        let mut initiator = FourWayHybrid::default();
        assert!(!initiator.on_ack_at_initiator());
        assert!(!initiator.initiator_live);
        let mut responder = FourWayHybrid::default();
        assert!(!responder.on_confirm_at_responder());
        assert!(!responder.responder_live);
        assert!(!responder.on_peer_new_key_at_responder());
    }

    #[test]
    fn timeout_rollback_drops_staged_keys_together() {
        let mut hs = FourWayHybrid::default();
        hs.on_init_at_responder();
        hs.on_resp_at_initiator();
        hs.expire_together();
        assert!(!hs.initiator_staged);
        assert!(!hs.responder_staged);
        assert!(!hs.initiator_live);
        assert!(!hs.responder_live);
        assert!(!hs.tx_keys_split());
    }

    #[test]
    fn confirm_retry_reemits_ack_until_ttl() {
        let mut initiator = FourWayHybrid::default();
        let mut responder = FourWayHybrid::default();
        responder.on_init_at_responder();
        initiator.on_resp_at_initiator();
        let staged_at = 1_000u64;
        assert!(initiator.should_retry_confirm(staged_at + CONFIRM_RETRY_MS, staged_at));
        initiator.note_confirm_retry();
        assert!(responder.on_confirm_at_responder());
        assert!(responder.on_confirm_retry_at_responder());
        initiator.note_confirm_retry();
        initiator.note_confirm_retry();
        assert!(!initiator.should_retry_confirm(staged_at + CONFIRM_RETRY_MS, staged_at));
        assert!(!initiator.initiator_live);
        assert!(!responder.responder_live);
    }

    #[test]
    fn confirm_ack_is_a_hybrid_opcode() {
        assert!(is_hybrid_opcode(OP_HYBRID_CONFIRM_ACK));
        assert_eq!(OP_HYBRID_CONFIRM_ACK, 0x20);
    }

    #[test]
    fn successful_four_way_does_not_split() {
        let mut initiator = FourWayHybrid::default();
        let mut responder = FourWayHybrid::default();
        responder.on_init_at_responder();
        initiator.on_resp_at_initiator();
        assert!(responder.on_confirm_at_responder());
        assert!(initiator.on_ack_at_initiator());
        assert!(responder.on_peer_new_key_at_responder());
        assert!(initiator.initiator_live && responder.responder_live);
        assert!(!FourWayHybrid::pair_tx_split(&initiator, &responder));
    }
}
