/// Session Management - QR-based key exchange with time-limited sessions
///
/// Flow:
/// 1. Device A calls generate_session_qr() → gets JSON with key + metadata
/// 2. Device A displays QR code containing the JSON
/// 3. Device B scans QR → calls import_session() with the JSON
/// 4. Both devices now share the same AES-256-GCM key
/// 5. Session expires after configured duration (1 day default, 3 day max)

use std::time::{SystemTime, UNIX_EPOCH};
use log::info;
use serde::{Deserialize, Serialize};

use crate::crypto::CryptoSession;

/// Maximum session duration: 3 days
const MAX_SESSION_HOURS: u32 = 72;
/// Default session duration: 1 day
const DEFAULT_SESSION_HOURS: u32 = 24;

/// Session key with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionKey {
    /// Base64-encoded 32-byte AES key
    pub key: String,
    /// Device name that generated this session
    pub device: String,
    /// Session creation timestamp (unix seconds)
    pub created_at: u64,
    /// Session expiry timestamp (unix seconds)
    pub expires_at: u64,
    /// Unique session ID
    pub session_id: String,
}

/// Manages active sessions
pub struct SessionManager {
    active_session: Option<SessionKey>,
    device_name: String,
}

impl SessionManager {
    pub fn new(device_name: &str) -> Self {
        Self {
            active_session: None,
            device_name: device_name.to_string(),
        }
    }

    /// Generate a new session and return its QR payload as JSON
    pub fn generate_session_qr(&mut self, duration_hours: u32) -> Result<String, String> {
        let hours = if duration_hours == 0 { DEFAULT_SESSION_HOURS } else { duration_hours };
        let duration = hours.min(MAX_SESSION_HOURS).max(1);
        let now = current_unix_time()?;
        let expires = now + (duration as u64 * 3600);

        // Generate 32-byte random key
        let key_bytes: [u8; 32] = rand::random();
        let key_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &key_bytes,
        );

        let session_id = uuid::Uuid::new_v4().to_string();

        let session = SessionKey {
            key: key_b64,
            device: self.device_name.clone(),
            created_at: now,
            expires_at: expires,
            session_id,
        };

        let json = serde_json::to_string(&session)
            .map_err(|e| format!("Failed to serialize session: {}", e))?;

        self.active_session = Some(session.clone());
        info!("Session generated: {} (expires in {}h)", session.session_id, duration);

        Ok(json)
    }

    /// Import a session from a scanned QR code JSON payload
    pub fn import_session(&mut self, qr_json: &str) -> Result<CryptoSession, String> {
        let session: SessionKey = serde_json::from_str(qr_json)
            .map_err(|e| format!("Invalid QR data: {}", e))?;

        // Validate expiry
        let now = current_unix_time()?;
        if now > session.expires_at {
            return Err("Session has expired".to_string());
        }

        // Validate duration doesn't exceed max
        let duration_secs = session.expires_at - session.created_at;
        if duration_secs > MAX_SESSION_HOURS as u64 * 3600 {
            return Err("Session duration exceeds maximum".to_string());
        }

        // Decode key
        let key_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &session.key,
        ).map_err(|e| format!("Invalid key encoding: {}", e))?;

        if key_bytes.len() != 32 {
            return Err(format!("Invalid key length: {} (expected 32)", key_bytes.len()));
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);

        let crypto = CryptoSession::from_psk(&key_array);

        self.active_session = Some(session.clone());
        info!("Session imported from {}: {}", session.device, session.session_id);

        Ok(crypto)
    }

    /// Get the CryptoSession from the active session key
    pub fn get_crypto_session(&self) -> Result<CryptoSession, String> {
        let session = self.active_session.as_ref()
            .ok_or("No active session")?;

        // Check expiry
        let now = current_unix_time()?;
        if now > session.expires_at {
            return Err("Session has expired".to_string());
        }

        let key_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &session.key,
        ).map_err(|e| format!("Invalid key: {}", e))?;

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);

        Ok(CryptoSession::from_psk(&key_array))
    }

    /// Check if there's a valid (non-expired) session
    pub fn is_authenticated(&self) -> bool {
        match &self.active_session {
            Some(session) => {
                match current_unix_time() {
                    Ok(now) => now < session.expires_at,
                    Err(_) => false,
                }
            }
            None => false,
        }
    }

    /// Get session status as JSON
    pub fn get_session_status(&self) -> String {
        match &self.active_session {
            Some(session) => {
                let now = current_unix_time().unwrap_or(0);
                let expired = now > session.expires_at;
                let remaining_secs = if expired { 0 } else { session.expires_at - now };

                serde_json::json!({
                    "active": !expired,
                    "session_id": session.session_id,
                    "peer_device": session.device,
                    "created_at": session.created_at,
                    "expires_at": session.expires_at,
                    "remaining_seconds": remaining_secs,
                    "expired": expired,
                }).to_string()
            }
            None => {
                serde_json::json!({
                    "active": false,
                }).to_string()
            }
        }
    }

    /// Clear the active session
    pub fn clear_session(&mut self) {
        if let Some(ref session) = self.active_session {
            info!("Session cleared: {}", session.session_id);
        }
        self.active_session = None;
    }
}

fn current_unix_time() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| format!("System time error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_generate_and_import() {
        let mut host = SessionManager::new("Host");
        let qr_json = host.generate_session_qr(24).unwrap();

        let mut joiner = SessionManager::new("Joiner");
        let crypto = joiner.import_session(&qr_json).unwrap();

        // Both should be authenticated
        assert!(host.is_authenticated());
        assert!(joiner.is_authenticated());

        // Crypto should work
        let plaintext = b"test audio data";
        let encrypted = crypto.encrypt(plaintext).unwrap();
        let host_crypto = host.get_crypto_session().unwrap();
        let decrypted = host_crypto.decrypt(&encrypted).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_session_expiry_validation() {
        let mut mgr = SessionManager::new("Test");

        // Create an already-expired session
        let expired_json = serde_json::json!({
            "key": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &[0u8; 32]),
            "device": "Old",
            "created_at": 1000,
            "expires_at": 1001,
            "session_id": "expired-session",
        }).to_string();

        let result = mgr.import_session(&expired_json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_session_max_duration() {
        let mut mgr = SessionManager::new("Test");
        // Request 100 hours, should be clamped to 72
        let qr = mgr.generate_session_qr(100).unwrap();
        let session: SessionKey = serde_json::from_str(&qr).unwrap();
        let duration_hours = (session.expires_at - session.created_at) / 3600;
        assert!(duration_hours <= 72);
    }
}
