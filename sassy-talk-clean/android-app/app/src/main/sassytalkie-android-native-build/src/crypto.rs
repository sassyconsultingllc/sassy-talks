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
use log::info;

const NONCE_SIZE: usize = 12;

pub struct CryptoSession {
    cipher: Aes256Gcm,
    nonce_counter: u64,
}

impl CryptoSession {
    pub fn from_shared_secret(shared: &SharedSecret) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(shared.as_bytes());
        let key_bytes = hasher.finalize();
        let cipher = Aes256Gcm::new_from_slice(&key_bytes).expect("AES-256-GCM key init failed");
        Self { cipher, nonce_counter: 0 }
    }

    pub fn from_psk(key: &[u8; 32]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(key).expect("AES-256-GCM key init failed");
        Self { cipher, nonce_counter: 0 }
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let nonce_bytes = self.next_nonce();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self.cipher.encrypt(nonce, plaintext).map_err(|e| format!("Encryption failed: {}", e))?;
        let mut output = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if data.len() < NONCE_SIZE + 16 {
            return Err("Data too short for decryption".to_string());
        }
        let nonce = Nonce::from_slice(&data[..NONCE_SIZE]);
        let ciphertext = &data[NONCE_SIZE..];
        self.cipher.decrypt(nonce, ciphertext).map_err(|e| format!("Decryption failed: {}", e))
    }

    fn next_nonce(&mut self) -> [u8; NONCE_SIZE] {
        self.nonce_counter += 1;
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[4..12].copy_from_slice(&self.nonce_counter.to_le_bytes());
        nonce
    }
}

pub struct KeyExchange {
    secret: Option<EphemeralSecret>,
    pub local_public: PublicKey,
}

impl KeyExchange {
    pub fn new() -> Self {
        let secret = EphemeralSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Self { secret: Some(secret), local_public: public }
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        *self.local_public.as_bytes()
    }

    pub fn complete(mut self, remote_public_bytes: &[u8; 32]) -> Result<CryptoSession, String> {
        let remote_public = PublicKey::from(*remote_public_bytes);
        let secret = self.secret.take().ok_or("Key exchange already completed")?;
        let shared = secret.diffie_hellman(&remote_public);
        info!("Key exchange completed");
        Ok(CryptoSession::from_shared_secret(&shared))
    }
}

pub fn generate_psk() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}
