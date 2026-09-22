// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! OS secret vault: Windows Credential Manager, macOS Keychain, Linux
//! libsecret when present. Callers fall back to the AES file store.

use std::env;

const TARGET_DEFAULT: &str = "SassyTalkie/session_psk";

pub fn target_name() -> String {
    env::var("SASSYTALKIE_VAULT_TARGET").unwrap_or_else(|_| TARGET_DEFAULT.to_string())
}

pub fn put(blob: &[u8]) -> Result<(), String> {
    put_named(&target_name(), blob)
}

pub fn get() -> Result<Option<Vec<u8>>, String> {
    get_named(&target_name())
}

pub fn delete() -> Result<(), String> {
    delete_named(&target_name())
}

pub fn put_named(target: &str, blob: &[u8]) -> Result<(), String> {
    os_put(target, blob)
}

pub fn get_named(target: &str) -> Result<Option<Vec<u8>>, String> {
    os_get(target)
}

pub fn delete_named(target: &str) -> Result<(), String> {
    os_delete(target)
}

pub fn available() -> bool {
    os_available()
}

#[cfg(windows)]
fn os_available() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn os_available() -> bool {
    true
}

#[cfg(target_os = "linux")]
fn os_available() -> bool {
    linux::available()
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn os_available() -> bool {
    false
}

#[cfg(windows)]
mod win {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn put(target: &str, blob: &[u8]) -> Result<(), String> {
        let mut target_w = wide(target);
        let mut data = blob.to_vec();
        let cred = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target_w.as_mut_ptr()),
            Comment: PWSTR::null(),
            LastWritten: FILETIME::default(),
            CredentialBlobSize: data.len() as u32,
            CredentialBlob: data.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: PWSTR::null(),
            UserName: PWSTR::null(),
        };
        unsafe { CredWriteW(&cred, 0) }.map_err(|e| format!("CredWriteW: {e}"))
    }

    pub fn get(target: &str) -> Result<Option<Vec<u8>>, String> {
        let target_w = wide(target);
        let mut pcred: *mut CREDENTIALW = std::ptr::null_mut();
        let ok = unsafe { CredReadW(PCWSTR(target_w.as_ptr()), CRED_TYPE_GENERIC, 0, &mut pcred) };
        match ok {
            Ok(()) if !pcred.is_null() => unsafe {
                let cred = &*pcred;
                let len = cred.CredentialBlobSize as usize;
                let bytes = if cred.CredentialBlob.is_null() || len == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(cred.CredentialBlob, len).to_vec()
                };
                CredFree(pcred as *const _);
                Ok(Some(bytes))
            },
            Ok(()) => Ok(None),
            Err(e) => {
                let code = e.code().0 as u32;
                if code == 0x80070490 || code == 1168 {
                    Ok(None)
                } else {
                    Err(format!("CredReadW: {e}"))
                }
            }
        }
    }

    pub fn delete(target: &str) -> Result<(), String> {
        let target_w = wide(target);
        match unsafe { CredDeleteW(PCWSTR(target_w.as_ptr()), CRED_TYPE_GENERIC, 0) } {
            Ok(()) => Ok(()),
            Err(e) => {
                let code = e.code().0 as u32;
                if code == 0x80070490 || code == 1168 {
                    Ok(())
                } else {
                    Err(format!("CredDeleteW: {e}"))
                }
            }
        }
    }
}

#[cfg(windows)]
fn os_put(target: &str, blob: &[u8]) -> Result<(), String> {
    win::put(target, blob)
}
#[cfg(windows)]
fn os_get(target: &str) -> Result<Option<Vec<u8>>, String> {
    win::get(target)
}
#[cfg(windows)]
fn os_delete(target: &str) -> Result<(), String> {
    win::delete(target)
}

#[cfg(target_os = "macos")]
mod mac {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const SERVICE: &str = "SassyTalkie";

    pub fn put(target: &str, blob: &[u8]) -> Result<(), String> {
        set_generic_password(SERVICE, target, blob).map_err(|e| format!("keychain put: {e}"))
    }

    pub fn get(target: &str) -> Result<Option<Vec<u8>>, String> {
        match get_generic_password(SERVICE, target) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("-25300") {
                    Ok(None)
                } else {
                    Err(format!("keychain get: {e}"))
                }
            }
        }
    }

    pub fn delete(target: &str) -> Result<(), String> {
        match delete_generic_password(SERVICE, target) {
            Ok(()) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("-25300") {
                    Ok(())
                } else {
                    Err(format!("keychain delete: {e}"))
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn os_put(target: &str, blob: &[u8]) -> Result<(), String> {
    mac::put(target, blob)
}
#[cfg(target_os = "macos")]
fn os_get(target: &str) -> Result<Option<Vec<u8>>, String> {
    mac::get(target)
}
#[cfg(target_os = "macos")]
fn os_delete(target: &str) -> Result<(), String> {
    mac::delete(target)
}

#[cfg(target_os = "linux")]
mod linux {
    use secret_service::{EncryptionType, SecretService};
    use std::collections::HashMap;

    fn attrs(target: &str) -> HashMap<&str, &str> {
        let mut m = HashMap::new();
        m.insert("application", "SassyTalkie");
        m.insert("target", target);
        m
    }

    pub fn available() -> bool {
        SecretService::new(EncryptionType::Dh).is_ok()
    }

    pub fn put(target: &str, blob: &[u8]) -> Result<(), String> {
        let ss = SecretService::new(EncryptionType::Dh).map_err(|e| format!("libsecret: {e}"))?;
        let collection = ss
            .get_default_collection()
            .map_err(|e| format!("libsecret collection: {e}"))?;
        collection
            .create_item(
                &format!("SassyTalkie:{target}"),
                attrs(target),
                blob,
                true,
                "application/octet-stream",
            )
            .map_err(|e| format!("libsecret put: {e}"))?;
        Ok(())
    }

    pub fn get(target: &str) -> Result<Option<Vec<u8>>, String> {
        let ss = match SecretService::new(EncryptionType::Dh) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let items = match ss.search_items(attrs(target)) {
            Ok(i) => i,
            Err(_) => return Ok(None),
        };
        let Some(item) = items.into_iter().next() else {
            return Ok(None);
        };
        match item.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) => Err(format!("libsecret get: {e}")),
        }
    }

    pub fn delete(target: &str) -> Result<(), String> {
        let ss = match SecretService::new(EncryptionType::Dh) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let items = match ss.search_items(attrs(target)) {
            Ok(i) => i,
            Err(_) => return Ok(()),
        };
        for item in items {
            let _ = item.delete();
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn os_put(target: &str, blob: &[u8]) -> Result<(), String> {
    linux::put(target, blob)
}
#[cfg(target_os = "linux")]
fn os_get(target: &str) -> Result<Option<Vec<u8>>, String> {
    linux::get(target)
}
#[cfg(target_os = "linux")]
fn os_delete(target: &str) -> Result<(), String> {
    linux::delete(target)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn os_put(_target: &str, _blob: &[u8]) -> Result<(), String> {
    Err("os vault unavailable".into())
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn os_get(_target: &str) -> Result<Option<Vec<u8>>, String> {
    Ok(None)
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn os_delete(_target: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(&mut input, None, None, None, None, 0, &mut output)
                .map_err(|e| format!("CryptProtectData: {e}"))?;
            let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(HLOCAL(output.pbData as *mut _));
            Ok(out)
        }
    }

    pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: blob.len() as u32,
            pbData: blob.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(&mut input, None, None, None, None, 0, &mut output)
                .map_err(|e| format!("CryptUnprotectData: {e}"))?;
            let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(HLOCAL(output.pbData as *mut _));
            Ok(out)
        }
    }
}

#[cfg(windows)]
pub fn protect_wrapping_key(plain: &[u8; 32]) -> Result<Vec<u8>, String> {
    dpapi::protect(plain)
}

#[cfg(windows)]
pub fn unprotect_wrapping_key(blob: &[u8]) -> Result<[u8; 32], String> {
    let pt = dpapi::unprotect(blob)?;
    if pt.len() != 32 {
        return Err("dpapi wrap key wrong length".into());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&pt);
    Ok(out)
}

#[cfg(not(windows))]
pub fn protect_wrapping_key(plain: &[u8; 32]) -> Result<Vec<u8>, String> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
pub fn unprotect_wrapping_key(blob: &[u8]) -> Result<[u8; 32], String> {
    if blob.len() != 32 {
        return Err("wrap key wrong length".into());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(blob);
    Ok(out)
}

/// Testable vault surface. OS backends and the AES file fallback share this.
pub trait SecretVault {
    fn put(&self, target: &str, blob: &[u8]) -> Result<(), String>;
    fn get(&self, target: &str) -> Result<Option<Vec<u8>>, String>;
    fn delete(&self, target: &str) -> Result<(), String>;
}

/// In-memory vault for unit tests (Windows CI does not need Keychain/libsecret).
pub struct MemoryVault {
    inner: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

impl Default for MemoryVault {
    fn default() -> Self {
        Self {
            inner: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl SecretVault for MemoryVault {
    fn put(&self, target: &str, blob: &[u8]) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|e| e.to_string())?
            .insert(target.to_string(), blob.to_vec());
        Ok(())
    }
    fn get(&self, target: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.inner.lock().map_err(|e| e.to_string())?.get(target).cloned())
    }
    fn delete(&self, target: &str) -> Result<(), String> {
        self.inner.lock().map_err(|e| e.to_string())?.remove(target);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_round_trip_or_unavailable() {
        let target = format!("SassyTalkie/test/{}", uuid::Uuid::new_v4());
        let payload = [7u8; 32];
        match put_named(&target, &payload) {
            Ok(()) => {
                let got = get_named(&target).unwrap().expect("stored");
                assert_eq!(got, payload);
                delete_named(&target).unwrap();
                assert!(get_named(&target).unwrap().is_none());
            }
            Err(e) => {
                assert!(
                    e.contains("unavailable") || !available(),
                    "unexpected vault error: {e}"
                );
            }
        }
    }

    #[test]
    fn wrapping_key_protect_round_trip() {
        let key = [3u8; 32];
        let blob = protect_wrapping_key(&key).unwrap();
        let back = unprotect_wrapping_key(&blob).unwrap();
        assert_eq!(back, key);
    }

    #[test]
    fn memory_vault_trait_round_trip() {
        let v = MemoryVault::default();
        v.put("t", &[1, 2, 3]).unwrap();
        assert_eq!(v.get("t").unwrap().as_deref(), Some(&[1, 2, 3][..]));
        v.delete("t").unwrap();
        assert!(v.get("t").unwrap().is_none());
    }
}
