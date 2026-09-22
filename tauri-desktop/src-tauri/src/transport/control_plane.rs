// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! Desktop authenticated control plane — same envelope as Android/iOS (`core::control_auth`).

use std::sync::Mutex;

use sassytalkie_core::control_auth::{
    classify_inbound, ControlAuthCodec, InboundControl, VerifiedControl,
};
use sassytalkie_core::crypto::CryptoSession;
use sassytalkie_core::hybrid_rekey::{
    confirm_acceptable, encode_hybrid_frame, parse_hybrid_frame, token_for, AUTO_PQC_DEFAULT,
    STAGED_TTL_MS,
};
use sassytalkie_core::pqc;
use sassytalkie_core::protocol::{
    encode_tlv, OP_EMERGENCY, OP_EMERGENCY_CLEAR, OP_HYBRID_CONFIRM, OP_HYBRID_CONFIRM_ACK,
    OP_HYBRID_INIT, OP_HYBRID_RESP, OP_MANDOWN,
};

use super::control::{self, PresenceState};

pub struct StagedHybrid {
    pub session: CryptoSession,
    pub token: [u8; 32],
    pub staged_at_ms: u64,
    pub channel: u8,
}

pub struct ControlPlane {
    codec: Mutex<Option<ControlAuthCodec>>,
    psk: Mutex<Option<[u8; 32]>>,
    pending_initiator: Mutex<Option<pqc::PskHybridInitiator>>,
    pending_initiator_session: Mutex<Option<CryptoSession>>,
    initiator_expect: Mutex<Option<([u8; 32], u64)>>,
    staged: Mutex<Option<StagedHybrid>>,
    /// Auto-PQC stays off unless an explicit product setting turns it on.
    pub auto_pqc: bool,
}

#[derive(Debug)]
pub enum ControlAction {
    /// Not a control TLV — caller should treat as audio.
    NotControl,
    Ignore,
    Rejected { reason: &'static str, opcode: u8 },
    Heartbeat,
    HybridOutbound(Vec<u8>),
    Emergency,
    InstalledPq,
}

impl ControlPlane {
    pub fn new() -> Self {
        Self {
            codec: Mutex::new(None),
            psk: Mutex::new(None),
            pending_initiator: Mutex::new(None),
            pending_initiator_session: Mutex::new(None),
            initiator_expect: Mutex::new(None),
            staged: Mutex::new(None),
            auto_pqc: AUTO_PQC_DEFAULT,
        }
    }

    pub fn install_psk(&self, key: [u8; 32], room_id: &str, sender_id: &str, epoch: u64) {
        *self.psk.lock().unwrap() = Some(key);
        *self.codec.lock().unwrap() =
            ControlAuthCodec::new(key, room_id, sender_id, epoch).ok();
    }

    pub fn clear(&self) {
        *self.codec.lock().unwrap() = None;
        *self.psk.lock().unwrap() = None;
        *self.pending_initiator.lock().unwrap() = None;
        *self.pending_initiator_session.lock().unwrap() = None;
        *self.initiator_expect.lock().unwrap() = None;
        *self.staged.lock().unwrap() = None;
    }

    pub fn seal(&self, inner: &[u8], now_ms: u64) -> Option<Vec<u8>> {
        self.codec.lock().unwrap().as_ref()?.seal(inner, now_ms).ok()
    }

    pub fn encode_heartbeat_sealed(
        &self,
        epoch: u64,
        seq: u32,
        now_ms: u64,
        state: PresenceState,
        rtt_ms: u16,
    ) -> Option<Vec<u8>> {
        let mut p = Vec::with_capacity(24);
        p.extend_from_slice(&epoch.to_le_bytes());
        p.extend_from_slice(&seq.to_le_bytes());
        p.extend_from_slice(&now_ms.to_le_bytes());
        p.push(state.as_byte());
        p.extend_from_slice(&rtt_ms.to_le_bytes());
        p.push(pqc::local_capabilities());
        let inner = encode_tlv(sassytalkie_core::protocol::OP_HEARTBEAT, &p);
        self.seal(&inner, now_ms)
    }

    pub fn handle_inbound(
        &self,
        bytes: &[u8],
        now_ms: u64,
        install_session: impl FnOnce(CryptoSession),
    ) -> ControlAction {
        let codec_guard = self.codec.lock().unwrap();
        match classify_inbound(codec_guard.as_ref(), bytes, now_ms) {
            InboundControl::NotControl => ControlAction::NotControl,
            InboundControl::LegacyHint { .. } => ControlAction::Ignore,
            InboundControl::RejectedUnauthenticated { opcode } => ControlAction::Rejected {
                reason: "unauthenticated",
                opcode,
            },
            InboundControl::AuthFailed => ControlAction::Rejected {
                reason: "auth_or_replay",
                opcode: 0x18,
            },
            InboundControl::Verified(verified) => {
                drop(codec_guard);
                self.dispatch_verified(verified, now_ms, install_session)
            }
        }
    }

    fn dispatch_verified(
        &self,
        verified: VerifiedControl,
        now_ms: u64,
        install_session: impl FnOnce(CryptoSession),
    ) -> ControlAction {
        let inner = sassytalkie_core::control_auth::decode_control_frame(&verified.inner_frame);
        let Some(decoded) = inner else {
            return ControlAction::Ignore;
        };
        match decoded.opcode {
            sassytalkie_core::protocol::OP_HEARTBEAT
            | sassytalkie_core::protocol::OP_RECV_ACK
            | sassytalkie_core::protocol::OP_EOT_ACK
            | sassytalkie_core::protocol::OP_WAKE
            | sassytalkie_core::protocol::OP_PTT_START_V2
            | sassytalkie_core::protocol::OP_PTT_STOP_V2
            | sassytalkie_core::protocol::OP_PARTNER_OFFLINE => ControlAction::Heartbeat,
            OP_EMERGENCY | OP_MANDOWN | OP_EMERGENCY_CLEAR => ControlAction::Emergency,
            OP_HYBRID_INIT => self.on_hybrid_init(decoded.payload, now_ms),
            OP_HYBRID_RESP => self.on_hybrid_resp(decoded.payload, now_ms),
            OP_HYBRID_CONFIRM => self.on_hybrid_confirm(decoded.payload, now_ms, install_session),
            OP_HYBRID_CONFIRM_ACK => {
                self.on_hybrid_confirm_ack(decoded.payload, now_ms, install_session)
            }
            _ => ControlAction::Ignore,
        }
    }

    fn on_hybrid_init(&self, payload: &[u8], now_ms: u64) -> ControlAction {
        let Some(psk) = *self.psk.lock().unwrap() else {
            return ControlAction::Rejected {
                reason: "hybrid_no_psk",
                opcode: OP_HYBRID_INIT,
            };
        };
        let Some((channel, init_bytes)) = parse_hybrid_frame(payload) else {
            return ControlAction::Rejected {
                reason: "hybrid_malformed",
                opcode: OP_HYBRID_INIT,
            };
        };
        let Ok(init_msg) = pqc::HybridInitiatorMessage::from_bytes(init_bytes) else {
            return ControlAction::Rejected {
                reason: "hybrid_malformed",
                opcode: OP_HYBRID_INIT,
            };
        };
        match pqc::psk_hybrid_respond(&psk, &init_msg) {
            Ok((resp, session)) => {
                let resp_bytes = resp.to_bytes();
                let token = token_for(&resp_bytes);
                *self.staged.lock().unwrap() = Some(StagedHybrid {
                    session,
                    token,
                    staged_at_ms: now_ms,
                    channel,
                });
                let inner = encode_hybrid_frame(OP_HYBRID_RESP, channel, &resp_bytes);
                match self.seal(&inner, now_ms) {
                    Some(frame) => ControlAction::HybridOutbound(frame),
                    None => ControlAction::Rejected {
                        reason: "hybrid_seal",
                        opcode: OP_HYBRID_INIT,
                    },
                }
            }
            Err(_) => ControlAction::Rejected {
                reason: "hybrid_respond",
                opcode: OP_HYBRID_INIT,
            },
        }
    }

    fn on_hybrid_resp(&self, payload: &[u8], now_ms: u64) -> ControlAction {
        let Some((channel, resp_bytes)) = parse_hybrid_frame(payload) else {
            return ControlAction::Rejected {
                reason: "hybrid_malformed",
                opcode: OP_HYBRID_RESP,
            };
        };
        let initiator = self.pending_initiator.lock().unwrap().take();
        let Some(initiator) = initiator else {
            return ControlAction::Ignore;
        };
        let Ok(resp_msg) = pqc::HybridResponderMessage::from_bytes(resp_bytes) else {
            return ControlAction::Rejected {
                reason: "hybrid_malformed",
                opcode: OP_HYBRID_RESP,
            };
        };
        match initiator.complete(&resp_msg) {
            Ok(session) => {
                *self.pending_initiator_session.lock().unwrap() = Some(session);
                let token = token_for(resp_bytes);
                *self.initiator_expect.lock().unwrap() = Some((token, now_ms));
                let inner = encode_hybrid_frame(OP_HYBRID_CONFIRM, channel, &token);
                match self.seal(&inner, now_ms) {
                    Some(frame) => ControlAction::HybridOutbound(frame),
                    None => ControlAction::Rejected {
                        reason: "hybrid_seal",
                        opcode: OP_HYBRID_RESP,
                    },
                }
            }
            Err(_) => ControlAction::Rejected {
                reason: "hybrid_complete",
                opcode: OP_HYBRID_RESP,
            },
        }
    }

    fn on_hybrid_confirm(
        &self,
        payload: &[u8],
        now_ms: u64,
        _install_session: impl FnOnce(CryptoSession),
    ) -> ControlAction {
        let Some((_channel, token)) = parse_hybrid_frame(payload) else {
            return ControlAction::Rejected {
                reason: "hybrid_confirm",
                opcode: OP_HYBRID_CONFIRM,
            };
        };
        let staged_guard = self.staged.lock().unwrap();
        let Some(staged) = staged_guard.as_ref() else {
            return ControlAction::Rejected {
                reason: "hybrid_confirm",
                opcode: OP_HYBRID_CONFIRM,
            };
        };
        if !confirm_acceptable(Some(&staged.token), Some(token), now_ms, staged.staged_at_ms) {
            return ControlAction::Rejected {
                reason: "hybrid_confirm",
                opcode: OP_HYBRID_CONFIRM,
            };
        }
        let channel = staged.channel;
        let ack_token = staged.token;
        drop(staged_guard);
        let _ttl = STAGED_TTL_MS;
        // ACK only — do not install TX. Staged session stays for RX until the
        // initiator is live on the new key (see try_decrypt_staged).
        let inner = encode_hybrid_frame(OP_HYBRID_CONFIRM_ACK, channel, &ack_token);
        match self.seal(&inner, now_ms) {
            Some(frame) => ControlAction::HybridOutbound(frame),
            None => ControlAction::Ignore,
        }
    }

    pub fn try_decrypt_staged(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let g = self.staged.lock().ok()?;
        g.as_ref()?.session.decrypt(bytes).ok()
    }

    pub fn promote_staged(&self) -> Option<CryptoSession> {
        self.staged.lock().ok()?.take().map(|s| s.session)
    }

    pub fn expire_staged(&self) {
        *self.staged.lock().unwrap() = None;
        *self.pending_initiator_session.lock().unwrap() = None;
        *self.initiator_expect.lock().unwrap() = None;
    }

    fn on_hybrid_confirm_ack(
        &self,
        payload: &[u8],
        now_ms: u64,
        install_session: impl FnOnce(CryptoSession),
    ) -> ControlAction {
        let Some((_channel, token)) = parse_hybrid_frame(payload) else {
            return ControlAction::Rejected {
                reason: "hybrid_confirm_ack",
                opcode: OP_HYBRID_CONFIRM_ACK,
            };
        };
        let expect = self.initiator_expect.lock().unwrap().take();
        let Some((staged_token, staged_at_ms)) = expect else {
            return ControlAction::Rejected {
                reason: "hybrid_confirm_ack",
                opcode: OP_HYBRID_CONFIRM_ACK,
            };
        };
        if !confirm_acceptable(Some(&staged_token), Some(token), now_ms, staged_at_ms) {
            *self.pending_initiator_session.lock().unwrap() = None;
            return ControlAction::Rejected {
                reason: "hybrid_confirm_ack",
                opcode: OP_HYBRID_CONFIRM_ACK,
            };
        }
        let session = self.pending_initiator_session.lock().unwrap().take();
        let Some(session) = session else {
            return ControlAction::Rejected {
                reason: "hybrid_confirm_ack",
                opcode: OP_HYBRID_CONFIRM_ACK,
            };
        };
        install_session(session);
        ControlAction::InstalledPq
    }
}

impl Default for ControlPlane {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sassytalkie_core::protocol::OP_AUTHENTICATED;

    fn key() -> [u8; 32] {
        [7u8; 32]
    }

    #[test]
    fn raw_privileged_control_is_rejected() {
        let plane = ControlPlane::new();
        plane.install_psk(key(), "room-abcdef", "desk-1", 11);
        let raw = control::encode_heartbeat(1, 1, 1_700_000_000_000, PresenceState::Idle, 0);
        match plane.handle_inbound(&raw, 1_700_000_000_000, |_| {}) {
            ControlAction::Rejected { reason, opcode } => {
                assert_eq!(reason, "unauthenticated");
                assert_eq!(opcode, sassytalkie_core::protocol::OP_HEARTBEAT);
            }
            other => panic!("expected reject, got {other:?}"),
        }
    }

    #[test]
    fn sealed_heartbeat_is_accepted() {
        let plane = ControlPlane::new();
        plane.install_psk(key(), "room-abcdef", "desk-1", 11);
        let now = 1_700_000_000_000u64;
        let sealed = plane
            .encode_heartbeat_sealed(11, 1, now, PresenceState::Idle, 0)
            .unwrap();
        assert_eq!(sealed[0], OP_AUTHENTICATED);
        match plane.handle_inbound(&sealed, now, |_| {}) {
            ControlAction::Heartbeat => {}
            other => panic!("expected heartbeat, got {other:?}"),
        }
    }

    #[test]
    fn auto_pqc_default_is_off() {
        assert!(!ControlPlane::new().auto_pqc);
        assert!(!AUTO_PQC_DEFAULT);
    }

    #[test]
    fn forged_confirm_does_not_install() {
        let plane = ControlPlane::new();
        plane.install_psk(key(), "room-abcdef", "desk-1", 11);
        let now = 1_700_000_000_000u64;
        let inner = encode_hybrid_frame(OP_HYBRID_CONFIRM, 1, &[1u8; 32]);
        let sealed = plane.seal(&inner, now).unwrap();
        let mut installed = false;
        match plane.handle_inbound(&sealed, now, |_| installed = true) {
            ControlAction::Rejected { reason, .. } => assert_eq!(reason, "hybrid_confirm"),
            other => panic!("expected reject, got {other:?}"),
        }
        assert!(!installed);
    }

    #[test]
    fn lost_ack_policy_does_not_split() {
        use sassytalkie_core::hybrid_rekey::FourWayHybrid;
        let mut i = FourWayHybrid::default();
        let mut r = FourWayHybrid::default();
        r.on_init_at_responder();
        i.on_resp_at_initiator();
        assert!(r.on_confirm_at_responder());
        i.expire_together();
        r.expire_together();
        assert!(!i.initiator_live);
        assert!(!r.responder_live);
        assert!(!i.tx_keys_split());
    }
}
