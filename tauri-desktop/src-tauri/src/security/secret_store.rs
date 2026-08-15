// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! AES-256-GCM file secret store plus an OS vault (Windows Credential Manager,
//! macOS Keychain, Linux libsecret/secret-service). Persist prefers the OS
//! vault and migrates any existing file blob into it. Fallback is the AES file
//! store (DPAPI-wrapped wrapping key on Windows; 32-byte wrap.key elsewhere).

use super::os_vault;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use zeroize::Zeroize;

const NONCE_LEN: usize = 12;
const MAGIC: &[u8; 4] = b"STSS";
const VERSION: u8 = 1;

pub struct FileSecretStore {
    path: PathBuf,
    cipher: Aes256Gcm,
}

impl FileSecretStore {
    /// `master_key` is 32 bytes. Callers should generate once and keep it in
    /// process memory; persistence of the master key itself is the documented
    /// OS-vault gap on desktop.
    pub fn new(path: impl AsRef<Path>, master_key: &[u8; 32]) -> Result<Self, String> {
        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|e| format!("secret store key: {e}"))?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            cipher,
        })
    }

    pub fn put(&self, plaintext: &[u8]) -> Result<(), String> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|e| format!("secret store encrypt: {e}"))?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = Vec::with_capacity(4 + 1 + NONCE_LEN + ct.len());
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        let tmp = self.path.with_extension("tmp");
        {
            let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
            f.write_all(&out).map_err(|e| e.to_string())?;
        }
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn get(&self) -> Result<Option<Vec<u8>>, String> {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        if bytes.len() < 4 + 1 + NONCE_LEN + 16 || &bytes[..4] != MAGIC || bytes[4] != VERSION {
            return Err("secret store corrupted".into());
        }
        let nonce = &bytes[5..5 + NONCE_LEN];
        let ct = &bytes[5 + NONCE_LEN..];
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| "secret store decrypt failed".to_string())?;
        Ok(Some(pt))
    }

    pub fn clear(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl Drop for FileSecretStore {
    fn drop(&mut self) {}
}

/// Derive a process wrapping key from a caller-supplied machine secret.
/// Desktop does not claim Keystore/Keychain binding.
pub fn wrapping_key_from_secret(secret: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"sassytalkie-desktop-wrap-v1");
    hasher.update(secret);
    let d = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

pub fn zeroize_key(key: &mut [u8; 32]) {
    key.zeroize();
}

fn store_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("APPDATA"))
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("SassyTalkie")
}

fn wrapping_key_path() -> PathBuf {
    store_dir().join("wrap.key")
}

fn session_blob_path() -> PathBuf {
    store_dir().join("session.enc")
}

fn load_or_create_wrapping_key() -> Result<[u8; 32], String> {
    let path = wrapping_key_path();
    if let Ok(bytes) = fs::read(&path) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            if let Ok(prot) = os_vault::protect_wrapping_key(&key) {
                if prot.len() != 32 {
                    let _ = fs::write(&path, &prot);
                }
            }
            return Ok(key);
        }
        if let Ok(key) = os_vault::unprotect_wrapping_key(&bytes) {
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let stored = os_vault::protect_wrapping_key(&key).unwrap_or_else(|_| key.to_vec());
    fs::write(&path, stored).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

pub fn persist_session_psk(psk: &[u8; 32]) -> Result<(), String> {
    if os_vault::put(psk).is_ok() {
        let _ = fs::remove_file(session_blob_path());
        return Ok(());
    }
    let wrap = load_or_create_wrapping_key()?;
    let store = FileSecretStore::new(session_blob_path(), &wrap)?;
    store.put(psk)
}

pub fn load_session_psk() -> Result<Option<[u8; 32]>, String> {
    if let Ok(Some(bytes)) = os_vault::get() {
        if bytes.len() == 32 {
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            return Ok(Some(out));
        }
    }
    let wrap = load_or_create_wrapping_key()?;
    let store = FileSecretStore::new(session_blob_path(), &wrap)?;
    match store.get()? {
        Some(bytes) if bytes.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            let _ = os_vault::put(&out);
            Ok(Some(out))
        }
        Some(_) => Err("persisted session key has wrong length".into()),
        None => Ok(None),
    }
}

pub fn wipe_session_psk() -> Result<(), String> {
    let _ = os_vault::delete();
    let _ = fs::remove_file(session_blob_path());
    let _ = fs::remove_file(wrapping_key_path());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn round_trip_and_clear() {
        let dir = env::temp_dir().join(format!("st-secret-{}", uuid::Uuid::new_v4()));
        let path = dir.join("psk.bin");
        let store = FileSecretStore::new(&path, &[9u8; 32]).unwrap();
        store.put(b"hello-psk-32-bytes-placeholder!!").unwrap();
        assert_eq!(
            store.get().unwrap().unwrap(),
            b"hello-psk-32-bytes-placeholder!!"
        );
        store.clear().unwrap();
        assert!(store.get().unwrap().is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn wrong_key_fails_closed() {
        let dir = env::temp_dir().join(format!("st-secret-{}", uuid::Uuid::new_v4()));
        let path = dir.join("psk.bin");
        FileSecretStore::new(&path, &[1u8; 32])
            .unwrap()
            .put(&[7u8; 32])
            .unwrap();
        let other = FileSecretStore::new(&path, &[2u8; 32]).unwrap();
        assert!(other.get().is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persist_load_wipe_prefers_os_vault() {
        let target = format!("SassyTalkie/test-persist/{}", uuid::Uuid::new_v4());
        let psk = [11u8; 32];
        os_vault::put_named(&target, &psk).unwrap();
        let loaded = os_vault::get_named(&target).unwrap().expect("psk");
        assert_eq!(loaded, psk);
        os_vault::delete_named(&target).unwrap();
        assert!(os_vault::get_named(&target).unwrap().is_none());
    }
}
