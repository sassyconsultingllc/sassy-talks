/// Session Management - Per-channel QR-based key exchange with time-limited sessions
///
/// Each channel (1-8) can have its own independent AES-256-GCM encryption key
/// and custom group name. This allows a user to be in multiple encrypted groups
/// simultaneously — e.g., "Alpha Team" on channel 1, "Night Shift" on channel 3.
///
/// Flow:
/// 1. Device A calls generate_session_qr(channel, duration, group_name)
/// 2. QR JSON includes the channel number + group name
/// 3. Device B scans QR → key stored in the same channel slot
/// 4. Both devices share the same AES key for that channel
/// 5. Switching channels switches which key is used for encrypt/decrypt

use std::time::{SystemTime, UNIX_EPOCH};
use log::info;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::CryptoSession;

/// Maximum session duration: 3 days
const MAX_SESSION_HOURS: u32 = 72;
/// Default session duration: 1 day
const DEFAULT_SESSION_HOURS: u32 = 24;
/// Maximum number of channels (displayed as 1-8)
pub const MAX_CHANNELS: usize = 8;

/// Session key with metadata (now includes channel + group name)
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
    /// Channel number (1-8). Legacy QR codes without this field default to 1.
    #[serde(default = "default_channel")]
    pub channel: u8,
    /// User-facing group name. Defaults to "Channel N".
    #[serde(default)]
    pub group_name: String,
    /// Stable cohort identifier across key rotations. Empty/missing in
    /// legacy QRs — the importer mints one locally in that case.
    #[serde(default)]
    pub cohort_id: String,
}

fn default_channel() -> u8 { 1 }

/// Wipe the base64-encoded AES key (and other sensitive strings) when the
/// SessionKey is dropped — i.e. when a channel is cleared, a session expires,
/// or the SessionManager itself goes away. This is defense-in-depth against
/// post-process memory inspection; it does NOT replace the on-disk hardening
/// in SassyTalkNative.kt.
impl Drop for SessionKey {
    fn drop(&mut self) {
        self.key.zeroize();
        self.device.zeroize();
        self.session_id.zeroize();
        self.group_name.zeroize();
        self.cohort_id.zeroize();
    }
}

/// Per-channel session slot
#[derive(Debug, Clone)]
pub struct ChannelSession {
    pub key: SessionKey,
    pub group_name: String,
}

impl Drop for ChannelSession {
    fn drop(&mut self) {
        // SessionKey drops itself; just wipe our duplicate copy of the name.
        self.group_name.zeroize();
    }
}

/// Per-channel session registry (replaces the old single-session SessionManager)
pub struct SessionManager {
    channels: [Option<ChannelSession>; MAX_CHANNELS],
    device_name: String,
}

impl SessionManager {
    pub fn new(device_name: &str) -> Self {
        Self {
            channels: Default::default(),
            device_name: device_name.to_string(),
        }
    }

    /// Generate a new session QR for a specific channel, minting a fresh cohort_id.
    pub fn generate_session_qr(
        &mut self,
        channel: u8,
        duration_hours: u32,
        group_name: &str,
    ) -> Result<String, String> {
        self.generate_session_qr_with_cohort(channel, duration_hours, group_name, None)
    }

    /// Generate a session QR, optionally reusing a previously-known cohort_id
    /// (used by the "Rejoin" flow so a regenerated session inherits cohort identity).
    pub fn generate_session_qr_with_cohort(
        &mut self,
        channel: u8,
        duration_hours: u32,
        group_name: &str,
        cohort_id: Option<&str>,
    ) -> Result<String, String> {
        let ch_idx = validate_channel(channel)?;
        let hours = if duration_hours == 0 { DEFAULT_SESSION_HOURS } else { duration_hours };
        let duration = hours.min(MAX_SESSION_HOURS).max(1);
        let now = current_unix_time()?;
        let expires = now + (duration as u64 * 3600);

        let key_bytes: [u8; 32] = rand::random();
        let key_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &key_bytes,
        );

        let session_id = uuid::Uuid::new_v4().to_string();
        let cohort = cohort_id
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let name = if group_name.is_empty() {
            format!("Channel {}", channel)
        } else {
            group_name.to_string()
        };

        let session = SessionKey {
            key: key_b64,
            device: self.device_name.clone(),
            created_at: now,
            expires_at: expires,
            session_id: session_id.clone(),
            channel,
            group_name: name.clone(),
            cohort_id: cohort.clone(),
        };

        let json = serde_json::to_string(&session)
            .map_err(|e| format!("Failed to serialize session: {}", e))?;

        self.channels[ch_idx] = Some(ChannelSession {
            key: session,
            group_name: name.clone(),
        });

        info!("Session generated for ch{} '{}' cohort {}: {} (expires in {}h)",
            channel, name, cohort, session_id, duration);

        Ok(json)
    }

    /// Import a session from a scanned QR code JSON payload.
    /// Returns (channel, CryptoSession) so the caller can set the active channel.
    pub fn import_session(&mut self, qr_json: &str) -> Result<(u8, CryptoSession), String> {
        let session: SessionKey = serde_json::from_str(qr_json)
            .map_err(|e| format!("Invalid QR data: {}", e))?;

        let channel = session.channel;
        let ch_idx = validate_channel(channel)?;

        let now = current_unix_time()?;
        if now > session.expires_at {
            return Err("Session has expired".to_string());
        }

        let duration_secs = session.expires_at - session.created_at;
        if duration_secs > MAX_SESSION_HOURS as u64 * 3600 {
            return Err("Session duration exceeds maximum".to_string());
        }

        let key_bytes = Zeroizing::new(base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &session.key,
        ).map_err(|e| format!("Invalid key encoding: {}", e))?);

        if key_bytes.len() != 32 {
            return Err(format!("Invalid key length: {} (expected 32)", key_bytes.len()));
        }

        let mut key_array = Zeroizing::new([0u8; 32]);
        key_array.copy_from_slice(&key_bytes);
        let crypto = CryptoSession::from_psk(&key_array);

        let name = if session.group_name.is_empty() {
            format!("Channel {}", channel)
        } else {
            session.group_name.clone()
        };

        info!("Session imported for ch{} '{}' from {}: {}",
            channel, name, session.device, session.session_id);

        self.channels[ch_idx] = Some(ChannelSession {
            key: session,
            group_name: name,
        });

        Ok((channel, crypto))
    }

    /// Get the CryptoSession for a specific channel (if it has a valid key).
    pub fn get_crypto_for_channel(&self, channel: u8) -> Option<CryptoSession> {
        let ch_idx = match validate_channel(channel) {
            Ok(i) => i,
            Err(_) => return None,
        };

        let cs = self.channels[ch_idx].as_ref()?;
        let now = current_unix_time().ok()?;
        if now > cs.key.expires_at {
            return None; // expired
        }

        let key_bytes = Zeroizing::new(base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &cs.key.key,
        ).ok()?);

        if key_bytes.len() != 32 { return None; }
        let mut arr = Zeroizing::new([0u8; 32]);
        arr.copy_from_slice(&key_bytes);
        Some(CryptoSession::from_psk(&arr))
    }

    /// Check if ANY channel has a valid (non-expired) session.
    pub fn is_authenticated(&self) -> bool {
        self.channels.iter().any(|slot| {
            if let Some(cs) = slot {
                current_unix_time().map(|now| now < cs.key.expires_at).unwrap_or(false)
            } else {
                false
            }
        })
    }

    /// Check if a specific channel has a valid session.
    pub fn channel_is_authenticated(&self, channel: u8) -> bool {
        self.get_crypto_for_channel(channel).is_some()
    }

    /// Get the group name for a channel.
    pub fn get_group_name(&self, channel: u8) -> String {
        let ch_idx = match validate_channel(channel) {
            Ok(i) => i,
            Err(_) => return format!("Channel {}", channel),
        };
        self.channels[ch_idx].as_ref()
            .map(|cs| cs.group_name.clone())
            .unwrap_or_else(|| format!("Channel {}", channel))
    }

    /// Set a custom group name for a channel.
    pub fn set_group_name(&mut self, channel: u8, name: &str) {
        if let Ok(idx) = validate_channel(channel) {
            if let Some(cs) = self.channels[idx].as_mut() {
                cs.group_name = name.to_string();
                cs.key.group_name = name.to_string();
            }
        }
    }

    /// Get the session_id for a channel (used as relay room ID).
    pub fn get_session_id(&self, channel: u8) -> Option<String> {
        let ch_idx = validate_channel(channel).ok()?;
        self.channels[ch_idx].as_ref().map(|cs| cs.key.session_id.clone())
    }

    /// Get the first valid session_id across all channels (for relay room).
    pub fn get_any_session_id(&self) -> Option<String> {
        self.channels.iter().filter_map(|slot| {
            slot.as_ref().map(|cs| cs.key.session_id.clone())
        }).next()
    }

    /// Get session status as JSON (for UI display).
    pub fn get_session_status(&self) -> String {
        let now = current_unix_time().unwrap_or(0);

        let channels: Vec<serde_json::Value> = (0..MAX_CHANNELS).map(|i| {
            let ch = (i + 1) as u8;
            match &self.channels[i] {
                Some(cs) => {
                    let expired = now > cs.key.expires_at;
                    let remaining = if expired { 0 } else { cs.key.expires_at - now };
                    serde_json::json!({
                        "channel": ch,
                        "active": !expired,
                        "group_name": cs.group_name,
                        "session_id": cs.key.session_id,
                        "peer_device": cs.key.device,
                        "remaining_seconds": remaining,
                        "fingerprint": &cs.key.session_id[..8],
                    })
                }
                None => serde_json::json!({
                    "channel": ch,
                    "active": false,
                    "group_name": format!("Channel {}", ch),
                }),
            }
        }).collect();

        serde_json::json!({
            "channels": channels,
            "any_active": self.is_authenticated(),
        }).to_string()
    }

    /// Get channel info as JSON array (lightweight, for channel picker).
    pub fn get_channel_info(&self) -> String {
        let now = current_unix_time().unwrap_or(0);
        let info: Vec<serde_json::Value> = (0..MAX_CHANNELS).map(|i| {
            let ch = (i + 1) as u8;
            match &self.channels[i] {
                Some(cs) => {
                    let active = now < cs.key.expires_at;
                    serde_json::json!({
                        "channel": ch,
                        "active": active,
                        "name": cs.group_name,
                        "fingerprint": &cs.key.session_id[..8],
                    })
                }
                None => serde_json::json!({
                    "channel": ch,
                    "active": false,
                    "name": format!("Channel {}", ch),
                }),
            }
        }).collect();
        serde_json::to_string(&info).unwrap_or_else(|_| "[]".to_string())
    }

    /// Clear a specific channel's session.
    pub fn clear_channel(&mut self, channel: u8) {
        if let Ok(idx) = validate_channel(channel) {
            if let Some(cs) = self.channels[idx].take() {
                info!("Session cleared for ch{}: {}", channel, cs.key.session_id);
            }
        }
    }

    /// Clear ALL channel sessions.
    pub fn clear_session(&mut self) {
        for i in 0..MAX_CHANNELS {
            if let Some(cs) = self.channels[i].take() {
                info!("Session cleared for ch{}: {}", i + 1, cs.key.session_id);
            }
        }
    }

    pub fn set_device_name(&mut self, name: &str) {
        self.device_name = name.to_string();
    }
}

fn validate_channel(channel: u8) -> Result<usize, String> {
    if channel < 1 || channel > MAX_CHANNELS as u8 {
        Err(format!("Invalid channel {} (must be 1-{})", channel, MAX_CHANNELS))
    } else {
        Ok((channel - 1) as usize)
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
        let qr_json = host.generate_session_qr(1, 24, "Alpha Team").unwrap();

        let mut joiner = SessionManager::new("Joiner");
        let (ch, mut crypto) = joiner.import_session(&qr_json).unwrap();

        assert_eq!(ch, 1);
        assert!(host.is_authenticated());
        assert!(joiner.is_authenticated());
        assert!(joiner.channel_is_authenticated(1));
        assert!(!joiner.channel_is_authenticated(2));

        // Crypto should work
        let plaintext = b"test audio data";
        let encrypted = crypto.encrypt(plaintext).unwrap();
        let host_crypto = host.get_crypto_for_channel(1).unwrap();
        let decrypted = host_crypto.decrypt(&encrypted).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_per_channel_isolation() {
        let mut mgr = SessionManager::new("Test");
        mgr.generate_session_qr(1, 24, "Team A").unwrap();
        mgr.generate_session_qr(3, 24, "Team B").unwrap();

        assert!(mgr.channel_is_authenticated(1));
        assert!(!mgr.channel_is_authenticated(2));
        assert!(mgr.channel_is_authenticated(3));

        assert_eq!(mgr.get_group_name(1), "Team A");
        assert_eq!(mgr.get_group_name(2), "Channel 2");
        assert_eq!(mgr.get_group_name(3), "Team B");

        // Different keys for different channels
        let c1 = mgr.get_crypto_for_channel(1).unwrap();
        let mut c3 = mgr.get_crypto_for_channel(3).unwrap();
        let encrypted = c3.encrypt(b"secret").unwrap();
        assert!(c1.decrypt(&encrypted).is_err()); // wrong key
    }

    #[test]
    fn test_legacy_qr_defaults_to_channel_1() {
        // Legacy QR without channel field
        let mut host = SessionManager::new("Host");
        let qr = host.generate_session_qr(1, 24, "").unwrap();

        // Strip channel field to simulate legacy
        let mut parsed: serde_json::Value = serde_json::from_str(&qr).unwrap();
        parsed.as_object_mut().unwrap().remove("channel");
        parsed.as_object_mut().unwrap().remove("group_name");
        let legacy_json = serde_json::to_string(&parsed).unwrap();

        let mut joiner = SessionManager::new("Joiner");
        let (ch, _) = joiner.import_session(&legacy_json).unwrap();
        assert_eq!(ch, 1); // defaults to channel 1
    }

    #[test]
    fn test_session_expiry_validation() {
        let mut mgr = SessionManager::new("Test");
        let expired_json = serde_json::json!({
            "key": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &[0u8; 32]),
            "device": "Old",
            "created_at": 1000,
            "expires_at": 1001,
            "session_id": "expired-session",
            "channel": 1,
            "group_name": "Expired",
        }).to_string();

        let result = mgr.import_session(&expired_json);
        assert!(result.is_err());
        assert!(result.err().unwrap().contains("expired"));
    }

    #[test]
    fn test_channel_info_json() {
        let mut mgr = SessionManager::new("Test");
        mgr.generate_session_qr(2, 24, "Ops").unwrap();
        let info = mgr.get_channel_info();
        assert!(info.contains("\"Ops\""));
        assert!(info.contains("\"channel\":2"));
    }

    #[test]
    fn test_session_includes_cohort_id_field() {
        let mut host = SessionManager::new("Host");
        let qr_json = host.generate_session_qr(1, 24, "Alpha Team").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&qr_json).unwrap();
        let cohort_id = parsed.get("cohort_id").and_then(|v| v.as_str()).unwrap_or("");
        assert!(!cohort_id.is_empty(), "cohort_id must be present and non-empty");
        assert_eq!(cohort_id.len(), 36, "cohort_id must be a UUID");
    }

    #[test]
    fn test_generate_with_reused_cohort_id_preserves_it() {
        let mut host = SessionManager::new("Host");
        let qr1 = host.generate_session_qr_with_cohort(1, 24, "Alpha", None).unwrap();
        let cid: String = serde_json::from_str::<serde_json::Value>(&qr1).unwrap()
            ["cohort_id"].as_str().unwrap().to_string();
        let qr2 = host.generate_session_qr_with_cohort(1, 24, "Alpha", Some(&cid)).unwrap();
        let cid2: String = serde_json::from_str::<serde_json::Value>(&qr2).unwrap()
            ["cohort_id"].as_str().unwrap().to_string();
        assert_eq!(cid, cid2, "supplied cohort_id must round-trip");
    }
}
