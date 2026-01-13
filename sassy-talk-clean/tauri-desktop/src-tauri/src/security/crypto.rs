// Crypto Engine - AES-256-GCM encryption for audio
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
//
// Key exchange: X25519 ECDH
// Symmetric: AES-256-GCM
// Nonce: 96-bit random per packet

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};
use sha2::{Sha256, Digest};
use rand::RngCore;
use thiserror::Error;
use tracing::{debug, error};

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Encryption failed")]
    EncryptionFailed,
    #[error("Decryption failed - authentication error")]
    DecryptionFailed,
    #[error("Key derivation failed")]
    KeyDerivationFailed,
    #[error("Invalid key length")]
    InvalidKeyLength,
}

/// Crypto engine handles all encryption/decryption
pub struct CryptoEngine {
    // Our ephemeral keypair
    secret_key: Option<EphemeralSecret>,
    public_key: Option<PublicKey>,
    
    // Derived symmetric key (from ECDH)
    symmetric_key: Option<Key<Aes256Gcm>>,
    
    // Cipher instance
    cipher: Option<Aes256Gcm>,
    
    // Key rotation timestamp
    key_created_at: Option<std::time::Instant>,
}

impl CryptoEngine {
    pub fn new() -> Self {
        Self {
            secret_key: None,
            public_key: None,
            symmetric_key: None,
            cipher: None,
            key_created_at: None,
        }
    }

    /// Generate new keypair for key exchange
    pub fn generate_keypair(&mut self) -> [u8; 32] {
        let secret = EphemeralSecret::random_from_rng(rand::thread_rng());
        let public = PublicKey::from(&secret);
        
        let public_bytes = *public.as_bytes();
        
        self.secret_key = Some(secret);
        self.public_key = Some(public);
        self.key_created_at = Some(std::time::Instant::now());
        
        debug!("Generated new X25519 keypair");
        
        public_bytes
    }

    /// Perform key exchange with peer's public key
    pub fn key_exchange(&mut self, peer_public: &[u8; 32]) -> Result<(), CryptoError> {
        let secret = self.secret_key.take()
            .ok_or(CryptoError::KeyDerivationFailed)?;
        
        let peer_public = PublicKey::from(*peer_public);
        let shared_secret = secret.diffie_hellman(&peer_public);
        
        // Derive symmetric key using HKDF-like derivation
        let key_bytes = self.derive_key(shared_secret.as_bytes());
        
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        
        self.symmetric_key = Some(*key);
        self.cipher = Some(cipher);
        
        debug!("Key exchange complete");
        
        Ok(())
    }

    /// Set symmetric key directly (for pre-shared key mode)
    pub fn set_key(&mut self, key: &[u8]) -> Result<(), CryptoError> {
        if key.len() != 32 {
            return Err(CryptoError::InvalidKeyLength);
        }
        
        let key = Key::<Aes256Gcm>::from_slice(key);
        let cipher = Aes256Gcm::new(key);
        
        self.symmetric_key = Some(*key);
        self.cipher = Some(cipher);
        self.key_created_at = Some(std::time::Instant::now());
        
        Ok(())
    }

    /// Derive 256-bit key from shared secret
    fn derive_key(&self, shared_secret: &[u8]) -> [u8; 32] {
        // Simple key derivation - production should use HKDF
        let mut hasher = Sha256::new();
        hasher.update(b"SassyTalk-v1-");
        hasher.update(shared_secret);
        
        let result = hasher.finalize();
        let mut key = [0u8; 32];
        key.copy_from_slice(&result);
        key
    }

    /// Encrypt data with AES-256-GCM
    pub fn encrypt(&self, plaintext: &[u8], nonce: &[u8; 12]) -> Result<(Vec<u8>, [u8; 16]), CryptoError> {
        let cipher = self.cipher.as_ref()
            .ok_or(CryptoError::EncryptionFailed)?;
        
        let nonce = Nonce::from_slice(nonce);
        
        let ciphertext = cipher.encrypt(nonce, plaintext)
            .map_err(|_| CryptoError::EncryptionFailed)?;
        
        // Split ciphertext and auth tag
        // AES-GCM appends 16-byte tag
        let tag_start = ciphertext.len() - 16;
        let mut auth_tag = [0u8; 16];
        auth_tag.copy_from_slice(&ciphertext[tag_start..]);
        
        let encrypted = ciphertext[..tag_start].to_vec();
        
        Ok((encrypted, auth_tag))
    }

    /// Decrypt data with AES-256-GCM
    pub fn decrypt(&self, ciphertext: &[u8], nonce: &[u8; 12], auth_tag: &[u8; 16]) -> Result<Vec<u8>, CryptoError> {
        let cipher = self.cipher.as_ref()
            .ok_or(CryptoError::DecryptionFailed)?;
        
        let nonce = Nonce::from_slice(nonce);
        
        // Reconstruct ciphertext with tag
        let mut combined = ciphertext.to_vec();
        combined.extend_from_slice(auth_tag);
        
        cipher.decrypt(nonce, combined.as_slice())
            .map_err(|_| CryptoError::DecryptionFailed)
    }

    /// Generate random nonce
    pub fn generate_nonce() -> [u8; 12] {
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        nonce
    }

    /// Check if key needs rotation (older than 60 seconds)
    pub fn needs_key_rotation(&self) -> bool {
        match self.key_created_at {
            Some(created) => created.elapsed().as_secs() > 60,
            None => true, // No key yet
        }
    }

    /// Check if encryption is ready
    pub fn is_ready(&self) -> bool {
        self.cipher.is_some()
    }

    /// Get public key bytes (for sending to peer)
    pub fn get_public_key(&self) -> Option<[u8; 32]> {
        self.public_key.as_ref().map(|pk| *pk.as_bytes())
    }
}

impl Default for CryptoEngine {
    fn default() -> Self {
        Self::new()
    }
}

// Fast XOR encryption for low-latency scenarios (not cryptographically secure!)
// Kept for backwards compatibility with original implementation

/// XOR encrypt/decrypt (symmetric operation)
pub fn xor_cipher(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .zip(key.iter().cycle())
        .map(|(d, k)| d ^ k)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes_gcm_roundtrip() {
        let mut engine = CryptoEngine::new();
        
        // Set a test key
        let key = [0x42u8; 32];
        engine.set_key(&key).unwrap();
        
        let plaintext = b"Hello, Walkie-Talkie!";
        let nonce = CryptoEngine::generate_nonce();
        
        // Encrypt
        let (ciphertext, tag) = engine.encrypt(plaintext, &nonce).unwrap();
        
        // Verify ciphertext is different
        assert_ne!(ciphertext, plaintext);
        
        // Decrypt
        let decrypted = engine.decrypt(&ciphertext, &nonce, &tag).unwrap();
        
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_key_exchange() {
        let mut alice = CryptoEngine::new();
        let mut bob = CryptoEngine::new();
        
        // Generate keypairs
        let alice_public = alice.generate_keypair();
        let bob_public = bob.generate_keypair();
        
        // Exchange keys
        alice.key_exchange(&bob_public).unwrap();
        bob.key_exchange(&alice_public).unwrap();
        
        // Both should derive same key
        assert_eq!(alice.symmetric_key, bob.symmetric_key);
        
        // Test encryption
        let plaintext = b"Secret message";
        let nonce = CryptoEngine::generate_nonce();
        
        let (ciphertext, tag) = alice.encrypt(plaintext, &nonce).unwrap();
        let decrypted = bob.decrypt(&ciphertext, &nonce, &tag).unwrap();
        
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_xor_cipher() {
        let data = b"Test data";
        let key = b"secret";
        
        let encrypted = xor_cipher(data, key);
        let decrypted = xor_cipher(&encrypted, key);
        
        assert_eq!(decrypted, data);
    }
}
