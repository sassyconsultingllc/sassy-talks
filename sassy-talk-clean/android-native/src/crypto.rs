/// Crypto Module - AES-256-GCM Encryption for Audio Transport
///
/// Handles key exchange (X25519 ECDH) and packet encryption/decryption.
/// Each session generates a fresh ephemeral keypair.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};
use sha2::{Sha256, Digest};
use rand::RngCore;
use log::{error, info};

/// Nonce size for AES-256-GCM (96 bits / 12 bytes)
const NONCE_SIZE: usize = 12;

/// Encryption session state
pub struct CryptoSession {
    cipher: Aes256Gcm,
    nonce_counter: u64,
}

impl CryptoSession {
    /// Create session from shared secret (post key-exchange)
    pub fn from_shared_secret(shared: &SharedSecret) -> Self {
        // Derive 256-bit AES key from shared secret via SHA-256
        let mut hasher = Sha256::new();
        hasher.update(shared.as_bytes());
        let key_bytes = hasher.finalize();

        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .expect("AES-256-GCM key init failed");

        Self {
            cipher,
            nonce_counter: 0,
        }
    }

    /// Create session from raw 32-byte key (for pre-shared key mode)
    pub fn from_psk(key: &[u8; 32]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(key)
            .expect("AES-256-GCM key init failed");

        Self {
            cipher,
            nonce_counter: 0,
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
        self.nonce_counter += 1;
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[4..12].copy_from_slice(&self.nonce_counter.to_le_bytes());
        nonce
    }
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
