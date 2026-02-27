use std::collections::HashMap;
use log::info;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: String, pub name: String, pub is_muted: bool, pub is_favorite: bool,
}

pub struct UserRegistry { users: HashMap<String, UserProfile> }

impl UserRegistry {
    pub fn new() -> Self { Self { users: HashMap::new() } }
    pub fn register_user(&mut self, id: &str, name: &str) {
        let entry = self.users.entry(id.to_string()).or_insert_with(|| {
            info!("UserRegistry: new user {} ({})", name, id);
            UserProfile { id: id.to_string(), name: name.to_string(), is_muted: false, is_favorite: false }
        });
        entry.name = name.to_string();
    }
    #[allow(dead_code)]
    pub fn is_muted(&self, id: &str) -> bool { self.users.get(id).map(|u| u.is_muted).unwrap_or(false) }
    pub fn set_muted(&mut self, id: &str, muted: bool) {
        if let Some(user) = self.users.get_mut(id) { user.is_muted = muted; }
    }
    #[allow(dead_code)]
    pub fn is_favorite(&self, id: &str) -> bool { self.users.get(id).map(|u| u.is_favorite).unwrap_or(false) }
    pub fn set_favorite(&mut self, id: &str, favorite: bool) {
        if let Some(user) = self.users.get_mut(id) { user.is_favorite = favorite; }
    }
    pub fn to_json(&self) -> String {
        let profiles: Vec<&UserProfile> = self.users.values().collect();
        serde_json::to_string(&profiles).unwrap_or_else(|_| "[]".to_string())
    }
    pub fn favorites(&self) -> Vec<&UserProfile> { self.users.values().filter(|u| u.is_favorite).collect() }
    #[allow(dead_code)]
    pub fn others(&self) -> Vec<&UserProfile> { self.users.values().filter(|u| !u.is_favorite).collect() }
    pub fn derive_user_id(session_key: &[u8]) -> String {
        use sha2::{Sha256, Digest};
        let hash = Sha256::digest(session_key);
        hash[..8].iter().map(|b| format!("{:02x}", b)).collect()
    }
}


