/// Crypto Module - AES-256-GCM Encryption for Audio Transport
///
/// Handles key exchange (X25519 ECDH) and packet encryption/decryption.
/// Each session generates a fresh ephemeral keypair.
///
/// Key hygiene: AES round keys live in `Aes256Gcm` which zeroizes on drop
/// (via the `zeroize` feature of aes-gcm). The KDF intermediate buffer
/// is held in `Zeroizing<[u8; 32]>` so it's wiped after the cipher is built.
/// X25519 ephemerals zeroize on drop via x25519-dalek's `zeroize` feature.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};
use hkdf::Hkdf;
use sha2::Sha256;
use rand::RngCore;
use zeroize::Zeroizing;
use log::info;

/// Nonce size for AES-256-GCM (96 bits / 12 bytes)
const NONCE_SIZE: usize = 12;

/// HKDF domain-separation tag. Bumping this breaks interop with v1 SHA-256-KDF
/// peers — but the active QR/PSK flow goes through `from_psk` and is unaffected.
/// Only the X25519 ECDH path uses this KDF.
const HKDF_INFO: &[u8] = b"sassytalkie-aead-v2";

/// Derive a 32-byte AES-256 key from input keying material via HKDF-SHA256.
/// Returns a Zeroizing wrapper so the caller can rely on automatic wipe.
fn derive_aes_key(ikm: &[u8]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(HKDF_INFO, &mut *out)
        .expect("HKDF expand of 32 bytes cannot fail");
    out
}

/// Encryption session state
pub struct CryptoSession {
    cipher: Aes256Gcm,
    nonce_counter: u64,
    /// Per-session random prefix occupying the first 4 bytes of each nonce.
    /// Without this, two devices sharing a PSK would both start their counter
    /// at 0 and reuse nonces — fatal for AES-GCM confidentiality.
    nonce_prefix: [u8; 4],
}

impl CryptoSession {
    /// Create session from shared secret (post key-exchange).
    /// Uses HKDF-SHA256 instead of plain SHA-256 for proper KDF hygiene.
    pub fn from_shared_secret(shared: &SharedSecret) -> Self {
        let key_bytes = derive_aes_key(shared.as_bytes());

        let cipher = Aes256Gcm::new_from_slice(&*key_bytes)
            .expect("AES-256-GCM key init failed");
        // key_bytes drops here, zeroizing the intermediate buffer.

        Self {
            cipher,
            nonce_counter: 0,
            nonce_prefix: random_nonce_prefix(),
        }
    }

    /// Create session from raw 32-byte key (for pre-shared key mode)
    pub fn from_psk(key: &[u8; 32]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(key)
            .expect("AES-256-GCM key init failed");

        Self {
            cipher,
            nonce_counter: 0,
            nonce_prefix: random_nonce_prefix(),
        }
    }

    /// Encrypt plaintext, returns nonce || ciphertext || tag
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let nonce_bytes = self.next_nonce();
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self.cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| format!("Encryption failed: {}", e))?;

        // Prepend nonce to ciphertext
        let mut output = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt data (expects nonce || ciphertext || tag)
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if data.len() < NONCE_SIZE + 16 {
            return Err("Data too short for decryption".to_string());
        }

        let nonce = Nonce::from_slice(&data[..NONCE_SIZE]);
        let ciphertext = &data[NONCE_SIZE..];

        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| format!("Decryption failed: {}", e))
    }

    fn next_nonce(&mut self) -> [u8; NONCE_SIZE] {
        self.nonce_counter = self.nonce_counter
            .checked_add(1)
            .expect("nonce counter overflow — session must be re-keyed");
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[..4].copy_from_slice(&self.nonce_prefix);
        nonce[4..12].copy_from_slice(&self.nonce_counter.to_le_bytes());
        nonce
    }
}

fn random_nonce_prefix() -> [u8; 4] {
    let mut prefix = [0u8; 4];
    OsRng.fill_bytes(&mut prefix);
    prefix
}

/// Key exchange helper
pub struct KeyExchange {
    secret: Option<EphemeralSecret>,
    pub local_public: PublicKey,
}

impl KeyExchange {
    /// Generate new ephemeral keypair
    pub fn new() -> Self {
        let secret = EphemeralSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Self {
            secret: Some(secret),
            local_public: public,
        }
    }

    /// Get local public key bytes for transmission
    pub fn public_key_bytes(&self) -> [u8; 32] {
        *self.local_public.as_bytes()
    }

    /// Complete key exchange with remote public key, consumes the secret
    pub fn complete(mut self, remote_public_bytes: &[u8; 32]) -> Result<CryptoSession, String> {
        let remote_public = PublicKey::from(*remote_public_bytes);
        let secret = self.secret.take()
            .ok_or("Key exchange already completed")?;
        let shared = secret.diffie_hellman(&remote_public);

        info!("Key exchange completed");
        Ok(CryptoSession::from_shared_secret(&shared))
    }
}

/// Generate a random 32-byte pre-shared key
pub fn generate_psk() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_psk() {
        let key = generate_psk();
        let mut session_a = CryptoSession::from_psk(&key);
        let session_b = CryptoSession::from_psk(&key);

        let plaintext = b"hello walkie talkie";
        let encrypted = session_a.encrypt(plaintext).unwrap();
        let decrypted = session_b.decrypt(&encrypted).unwrap();

        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_key_exchange() {
        let kx_a = KeyExchange::new();
        let kx_b = KeyExchange::new();

        let pub_a = kx_a.public_key_bytes();
        let pub_b = kx_b.public_key_bytes();

        let mut session_a = kx_b.complete(&pub_a).unwrap();
        let session_b = kx_a.complete(&pub_b).unwrap();

        let plaintext = b"secure audio frame data";
        let encrypted = session_a.encrypt(plaintext).unwrap();
        let decrypted = session_b.decrypt(&encrypted).unwrap();

        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key_a = generate_psk();
        let key_b = generate_psk();
        let mut session_a = CryptoSession::from_psk(&key_a);
        let session_b = CryptoSession::from_psk(&key_b);

        let encrypted = session_a.encrypt(b"secret").unwrap();
        assert!(session_b.decrypt(&encrypted).is_err());
    }
}
