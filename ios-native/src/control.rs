// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-6OA5AKLYYTYD
//! control — iOS control-plane frames. Wire format is the shared core envelope.

use sassytalkie_core::pqc;
use sassytalkie_core::protocol::{encode_tlv, OP_HEARTBEAT};

pub const PRESENCE_IDLE: u8 = 0;
pub const PRESENCE_SPEAKING: u8 = 2;

/// Heartbeat with trailing capabilities byte (CAP_HYBRID_PQC advertised;
/// auto-PQC initiation remains off).
pub fn encode_heartbeat(epoch: u64, seq: u32, ts_ms: u64, state: u8, rtt_ms: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(24);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&ts_ms.to_le_bytes());
    p.push(state);
    p.extend_from_slice(&rtt_ms.to_le_bytes());
    p.push(pqc::local_capabilities());
    encode_tlv(OP_HEARTBEAT, &p)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sassytalkie_core::control_auth::{classify_inbound, ControlAuthCodec, InboundControl};
    use sassytalkie_core::protocol::OP_AUTHENTICATED;

    #[test]
    fn heartbeat_is_tlv_with_caps_byte() {
        let hb = encode_heartbeat(0xDEAD_BEEF_CAFE_F00D, 7, 1_700_000_000_000, PRESENCE_IDLE, 0);
        assert_eq!(hb[0], OP_HEARTBEAT);
        let len = u16::from_le_bytes([hb[1], hb[2]]) as usize;
        assert_eq!(len, 24);
        assert_eq!(hb.len(), 3 + 24);
        assert_eq!(hb[3 + 23], pqc::CAP_HYBRID_PQC);
    }

    #[test]
    fn raw_heartbeat_is_rejected_without_envelope() {
        let hb = encode_heartbeat(1, 1, 1_700_000_000_000, PRESENCE_IDLE, 0);
        let key = [3u8; 32];
        let codec = ControlAuthCodec::new(key, "room-ios-1", "ios-1", 11).unwrap();
        match classify_inbound(Some(&codec), &hb, 1_700_000_000_000) {
            InboundControl::RejectedUnauthenticated { opcode } => assert_eq!(opcode, OP_HEARTBEAT),
            other => panic!("expected reject, got {other:?}"),
        }
    }

    #[test]
    fn sealed_heartbeat_opens() {
        let key = [3u8; 32];
        let sender = ControlAuthCodec::new(key, "room-ios-1", "ios-a", 11).unwrap();
        let receiver = ControlAuthCodec::new(key, "room-ios-1", "ios-b", 22).unwrap();
        let now = 1_700_000_000_000u64;
        let inner = encode_heartbeat(11, 1, now, PRESENCE_IDLE, 0);
        let sealed = sender.seal(&inner, now).unwrap();
        assert_eq!(sealed[0], OP_AUTHENTICATED);
        match classify_inbound(Some(&receiver), &sealed, now) {
            InboundControl::Verified(v) => assert_eq!(v.sender_id, "ios-a"),
            other => panic!("expected verified, got {other:?}"),
        }
    }
}
