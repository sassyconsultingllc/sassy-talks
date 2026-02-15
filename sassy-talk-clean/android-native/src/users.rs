/// User Registry - Manages mute/favorite status for peers
///
/// Each user is identified by a truncated hash of the session key they used.
/// This provides consistent identity across reconnections within a session.

use std::collections::HashMap;
use log::info;
use serde::{Deserialize, Serialize};

/// User profile stored in registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub name: String,
    pub is_muted: bool,
    pub is_favorite: bool,
}

/// Registry of known users
pub struct UserRegistry {
    users: HashMap<String, UserProfile>,
}

impl UserRegistry {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    /// Register or update a user
    pub fn register_user(&mut self, id: &str, name: &str) {
        let entry = self.users.entry(id.to_string()).or_insert_with(|| {
            info!("UserRegistry: new user {} ({})", name, id);
            UserProfile {
                id: id.to_string(),
                name: name.to_string(),
                is_muted: false,
                is_favorite: false,
            }
        });
        // Update name if it changed
        entry.name = name.to_string();
    }

    /// Check if a user is muted (returns false for unknown users)
    pub fn is_muted(&self, id: &str) -> bool {
        self.users.get(id).map(|u| u.is_muted).unwrap_or(false)
    }

    /// Set mute status
    pub fn set_muted(&mut self, id: &str, muted: bool) {
        if let Some(user) = self.users.get_mut(id) {
            user.is_muted = muted;
            info!("UserRegistry: {} {} {}", if muted { "muted" } else { "unmuted" }, user.name, id);
        }
    }

    /// Check if a user is favorited
    pub fn is_favorite(&self, id: &str) -> bool {
        self.users.get(id).map(|u| u.is_favorite).unwrap_or(false)
    }

    /// Set favorite status
    pub fn set_favorite(&mut self, id: &str, favorite: bool) {
        if let Some(user) = self.users.get_mut(id) {
            user.is_favorite = favorite;
            info!("UserRegistry: {} {} {}", if favorite { "favorited" } else { "unfavorited" }, user.name, id);
        }
    }

    /// Get all users as JSON array
    pub fn to_json(&self) -> String {
        let profiles: Vec<&UserProfile> = self.users.values().collect();
        serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".to_string())
    }

    /// Get favorites
    pub fn favorites(&self) -> Vec<&UserProfile> {
        self.users.values().filter(|u| u.is_favorite).collect()
    }

    /// Get non-favorites (excluding muted optionally)
    pub fn others(&self) -> Vec<&UserProfile> {
        self.users.values().filter(|u| !u.is_favorite).collect()
    }

    /// Derive a user ID from session key bytes (first 8 bytes of SHA-256)
    pub fn derive_user_id(session_key: &[u8]) -> String {
        use sha2::{Sha256, Digest};
        let hash = Sha256::digest(session_key);
        hex::encode(&hash[..8])
    }
}

/// Hex encoding helper (no external dep needed)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_registry() {
        let mut reg = UserRegistry::new();

        reg.register_user("abc123", "Alice");
        reg.register_user("def456", "Bob");

        assert!(!reg.is_muted("abc123"));
        assert!(!reg.is_favorite("abc123"));

        reg.set_muted("abc123", true);
        assert!(reg.is_muted("abc123"));
        assert!(!reg.is_muted("def456"));

        reg.set_favorite("def456", true);
        assert!(reg.is_favorite("def456"));

        assert_eq!(reg.favorites().len(), 1);
        assert_eq!(reg.others().len(), 1);
    }

    #[test]
    fn test_user_id_derivation() {
        let key1 = [1u8; 32];
        let key2 = [2u8; 32];
        let id1 = UserRegistry::derive_user_id(&key1);
        let id2 = UserRegistry::derive_user_id(&key2);
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_json_output() {
        let mut reg = UserRegistry::new();
        reg.register_user("abc", "Alice");
        reg.set_muted("abc", true);
        let json = reg.to_json();
        assert!(json.contains("Alice"));
        assert!(json.contains("\"is_muted\":true"));
    }
}
