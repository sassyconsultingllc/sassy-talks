// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! Room ID is not authorization. Join requires a room-secret proof (PSK HMAC)
//! and, when MDM supplies an enrollment token, that token must match.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

const PROOF_DOMAIN: &[u8] = b"sassytalkie-enroll-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Operator,
    Supervisor,
}

impl Role {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "supervisor" => Self::Supervisor,
            _ => Self::Operator,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Supervisor => "supervisor",
        }
    }

    /// Supervisors may export the technical audit and invoke local wipe.
    pub fn can_export_audit(self) -> bool {
        true
    }

    pub fn can_invoke_wipe(self) -> bool {
        matches!(self, Self::Supervisor)
    }
}

/// HMAC-SHA256(room_secret, domain || room_id || peer_id). Hex lowercase.
pub fn room_secret_proof(room_secret: &[u8], room_id: &str, peer_id: &str) -> Result<String, &'static str> {
    if room_secret.len() != 32 {
        return Err("room secret must be 32 bytes");
    }
    if room_id.len() < 8 || peer_id.is_empty() {
        return Err("room and peer are required");
    }
    let mut mac = HmacSha256::new_from_slice(room_secret).map_err(|_| "hmac key")?;
    mac.update(PROOF_DOMAIN);
    mac.update(&[0u8]);
    mac.update(room_id.as_bytes());
    mac.update(&[0u8]);
    mac.update(peer_id.as_bytes());
    Ok(hex_lower(&mac.finalize().into_bytes()))
}

pub fn verify_room_secret_proof(
    room_secret: &[u8],
    room_id: &str,
    peer_id: &str,
    presented: &str,
) -> bool {
    match room_secret_proof(room_secret, room_id, peer_id) {
        Ok(expected) => hex_eq(&expected, presented),
        Err(_) => false,
    }
}

/// MDM enrollment token is a separate shared secret. Room ID alone never joins.
pub fn enrollment_token_acceptable(required_token: Option<&str>, presented: Option<&str>) -> bool {
    match required_token.map(str::trim).filter(|s| !s.is_empty()) {
        None => true,
        Some(required) => match presented.map(str::trim).filter(|s| !s.is_empty()) {
            None => false,
            Some(got) => bool::from(required.as_bytes().ct_eq(got.as_bytes())),
        },
    }
}

/// Join is authorized only when a 32-byte room secret is present AND any
/// required enrollment token matches. Knowing the room id is not enough.
pub fn join_authorized(
    room_id: &str,
    room_secret: Option<&[u8]>,
    required_enrollment_token: Option<&str>,
    presented_enrollment_token: Option<&str>,
) -> bool {
    if room_id.len() < 8 {
        return false;
    }
    match room_secret {
        Some(secret) if secret.len() == 32 => {
            enrollment_token_acceptable(required_enrollment_token, presented_enrollment_token)
        }
        _ => false,
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn hex_eq(a: &str, b: &str) -> bool {
    let a = a.trim().as_bytes();
    let b = b.trim().as_bytes();
    if a.len() != b.len() {
        return false;
    }
    bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_id_alone_is_not_authorization() {
        assert!(!join_authorized("room-id-long-enough", None, None, None));
        assert!(!join_authorized("room-id-long-enough", Some(&[0u8; 16]), None, None));
    }

    #[test]
    fn psk_authorizes_when_no_enrollment_token_required() {
        assert!(join_authorized("room-id-long-enough", Some(&[7u8; 32]), None, None));
    }

    #[test]
    fn enrollment_token_mismatch_rejected() {
        assert!(!join_authorized(
            "room-id-long-enough",
            Some(&[7u8; 32]),
            Some("agency-token"),
            Some("wrong"),
        ));
        assert!(!join_authorized(
            "room-id-long-enough",
            Some(&[7u8; 32]),
            Some("agency-token"),
            None,
        ));
        assert!(join_authorized(
            "room-id-long-enough",
            Some(&[7u8; 32]),
            Some("agency-token"),
            Some("agency-token"),
        ));
    }

    #[test]
    fn room_secret_proof_round_trips_and_rejects_forgeries() {
        let secret = [9u8; 32];
        let proof = room_secret_proof(&secret, "room-abcd-efgh", "install-1").unwrap();
        assert!(verify_room_secret_proof(&secret, "room-abcd-efgh", "install-1", &proof));
        assert!(!verify_room_secret_proof(&secret, "room-abcd-efgh", "other", &proof));
        assert!(!verify_room_secret_proof(&[8u8; 32], "room-abcd-efgh", "install-1", &proof));
    }

    #[test]
    fn supervisor_can_wipe_operator_cannot() {
        assert!(!Role::Operator.can_invoke_wipe());
        assert!(Role::Supervisor.can_invoke_wipe());
        assert_eq!(Role::parse("SUPERVISOR"), Role::Supervisor);
        assert_eq!(Role::parse("member"), Role::Operator);
    }
}
