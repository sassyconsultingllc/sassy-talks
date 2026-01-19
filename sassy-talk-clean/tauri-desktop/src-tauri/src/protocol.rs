/// Protocol - Packet Format and Serialization
/// 
/// Defines the wire protocol for SassyTalkie UDP multicast
/// Supports encrypted audio and key exchange
/// 
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Protocol error types
#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    
    #[error("Invalid packet type: {0}")]
    InvalidPacketType(u8),
    
    #[error("Checksum mismatch")]
    ChecksumMismatch,
    
    #[error("Invalid key length")]
    InvalidKeyLength,
}

/// Packet types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PacketType {
    /// Discovery beacon (announces presence)
    Discovery {
        device_name: String,
        channel: u8,
    },
    
    /// Discovery with public key for encryption
    DiscoveryWithKey {
        device_name: String,
        channel: u8,
        /// X25519 public key (32 bytes)
        public_key: Option<[u8; 32]>,
    },
    
    /// Audio data (may be encrypted or plain)
    Audio {
        channel: u8,
        data: Vec<u8>,
    },
    
    /// Encrypted audio with explicit nonce and tag
    EncryptedAudio {
        channel: u8,
        /// 96-bit nonce
        nonce: [u8; 12],
        /// 128-bit auth tag
        auth_tag: [u8; 16],
        /// Ciphertext
        data: Vec<u8>,
    },
    
    /// Keep-alive (maintains connection)
    KeepAlive,
    
    /// Key exchange request
    KeyExchange {
        /// Our public key
        public_key: [u8; 32],
    },
    
    /// Key exchange response
    KeyExchangeResponse {
        /// Their public key
        public_key: [u8; 32],
        /// Whether exchange was successful
        success: bool,
    },
}

/// Network packet structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Packet {
    /// Protocol version
    pub version: u8,
    
    /// Device ID (unique identifier)
    pub device_id: u32,
    
    /// Packet type
    pub packet_type: PacketType,
    
    /// Timestamp (Unix epoch milliseconds)
    pub timestamp: u64,
    
    /// Checksum (CRC32)
    pub checksum: u32,
}

impl Packet {
    /// Protocol version
    const VERSION: u8 = 2;  // Bumped for encryption support
    
    /// Create discovery packet
    pub fn discovery(device_id: u32, device_name: String, channel: u8) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let packet = Self {
            version: Self::VERSION,
            device_id,
            packet_type: PacketType::Discovery { device_name, channel },
            timestamp,
            checksum: 0,
        };
        
        packet.with_checksum()
    }
    
    /// Create discovery packet with public key
    pub fn discovery_with_key(device_id: u32, device_name: String, channel: u8, public_key: Option<[u8; 32]>) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let packet = Self {
            version: Self::VERSION,
            device_id,
            packet_type: PacketType::DiscoveryWithKey { device_name, channel, public_key },
            timestamp,
            checksum: 0,
        };
        
        packet.with_checksum()
    }
    
    /// Create audio packet
    pub fn audio(device_id: u32, channel: u8, data: Vec<u8>) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let packet = Self {
            version: Self::VERSION,
            device_id,
            packet_type: PacketType::Audio { channel, data },
            timestamp,
            checksum: 0,
        };
        
        packet.with_checksum()
    }
    
    /// Create encrypted audio packet
    pub fn encrypted_audio(device_id: u32, channel: u8, nonce: [u8; 12], auth_tag: [u8; 16], data: Vec<u8>) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let packet = Self {
            version: Self::VERSION,
            device_id,
            packet_type: PacketType::EncryptedAudio { channel, nonce, auth_tag, data },
            timestamp,
            checksum: 0,
        };
        
        packet.with_checksum()
    }
    
    /// Create keep-alive packet
    pub fn keep_alive(device_id: u32) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let packet = Self {
            version: Self::VERSION,
            device_id,
            packet_type: PacketType::KeepAlive,
            timestamp,
            checksum: 0,
        };
        
        packet.with_checksum()
    }
    
    /// Create key exchange request
    pub fn key_exchange(device_id: u32, public_key: [u8; 32]) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let packet = Self {
            version: Self::VERSION,
            device_id,
            packet_type: PacketType::KeyExchange { public_key },
            timestamp,
            checksum: 0,
        };
        
        packet.with_checksum()
    }
    
    /// Calculate checksum
    fn calculate_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&self.device_id.to_le_bytes());
        hasher.update(&self.timestamp.to_le_bytes());
        
        // Hash packet type
        match &self.packet_type {
            PacketType::Discovery { device_name, channel } => {
                hasher.update(&[0u8]); // Type discriminant
                hasher.update(device_name.as_bytes());
                hasher.update(&[*channel]);
            }
            PacketType::DiscoveryWithKey { device_name, channel, public_key } => {
                hasher.update(&[1u8]); // Type discriminant
                hasher.update(device_name.as_bytes());
                hasher.update(&[*channel]);
                if let Some(key) = public_key {
                    hasher.update(key);
                }
            }
            PacketType::Audio { channel, data } => {
                hasher.update(&[2u8]); // Type discriminant
                hasher.update(&[*channel]);
                hasher.update(data);
            }
            PacketType::EncryptedAudio { channel, nonce, auth_tag, data } => {
                hasher.update(&[3u8]); // Type discriminant
                hasher.update(&[*channel]);
                hasher.update(nonce);
                hasher.update(auth_tag);
                hasher.update(data);
            }
            PacketType::KeepAlive => {
                hasher.update(&[4u8]); // Type discriminant
            }
            PacketType::KeyExchange { public_key } => {
                hasher.update(&[5u8]); // Type discriminant
                hasher.update(public_key);
            }
            PacketType::KeyExchangeResponse { public_key, success } => {
                hasher.update(&[6u8]); // Type discriminant
                hasher.update(public_key);
                hasher.update(&[*success as u8]);
            }
        }
        
        hasher.finalize()
    }
    
    /// Add checksum to packet
    fn with_checksum(mut self) -> Self {
        self.checksum = self.calculate_checksum();
        self
    }
    
    /// Verify checksum
    pub fn verify_checksum(&self) -> bool {
        let calculated = self.calculate_checksum();
        calculated == self.checksum
    }
    
    /// Serialize packet to bytes
    pub fn serialize(&self) -> Result<Vec<u8>, String> {
        bincode::serialize(self)
            .map_err(|e| format!("Serialization failed: {}", e))
    }
    
    /// Deserialize packet from bytes
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        let packet: Packet = bincode::deserialize(data)
            .map_err(|e| format!("Deserialization failed: {}", e))?;
        
        // Verify version (allow both v1 and v2 for compatibility)
        if packet.version < 1 || packet.version > Self::VERSION {
            return Err(format!("Invalid protocol version: {}", packet.version));
        }
        
        // Verify checksum
        if !packet.verify_checksum() {
            return Err("Checksum verification failed".to_string());
        }
        
        Ok(packet)
    }
    
    /// Get packet size estimate
    pub fn estimate_size(&self) -> usize {
        match &self.packet_type {
            PacketType::Discovery { device_name, .. } => 50 + device_name.len(),
            PacketType::DiscoveryWithKey { device_name, .. } => 82 + device_name.len(),
            PacketType::Audio { data, .. } => 50 + data.len(),
            PacketType::EncryptedAudio { data, .. } => 78 + data.len(),
            PacketType::KeepAlive => 50,
            PacketType::KeyExchange { .. } => 82,
            PacketType::KeyExchangeResponse { .. } => 83,
        }
    }
    
    /// Check if packet is encrypted
    pub fn is_encrypted(&self) -> bool {
        matches!(self.packet_type, PacketType::EncryptedAudio { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_packet() {
        let packet = Packet::discovery(0x12345678, "Test Device".to_string(), 42);
        
        assert_eq!(packet.version, 2);
        assert_eq!(packet.device_id, 0x12345678);
        assert!(packet.verify_checksum());
        
        // Serialize and deserialize
        let serialized = packet.serialize().unwrap();
        let deserialized = Packet::deserialize(&serialized).unwrap();
        
        assert_eq!(deserialized.device_id, packet.device_id);
        assert!(deserialized.verify_checksum());
    }
    
    #[test]
    fn test_discovery_with_key() {
        let key = [0x42u8; 32];
        let packet = Packet::discovery_with_key(0x12345678, "Test".to_string(), 1, Some(key));
        
        assert!(packet.verify_checksum());
        
        if let PacketType::DiscoveryWithKey { public_key, .. } = &packet.packet_type {
            assert_eq!(*public_key, Some(key));
        } else {
            panic!("Wrong packet type");
        }
        
        // Roundtrip
        let serialized = packet.serialize().unwrap();
        let deserialized = Packet::deserialize(&serialized).unwrap();
        assert!(deserialized.verify_checksum());
    }

    #[test]
    fn test_audio_packet() {
        let audio_data = vec![1, 2, 3, 4, 5];
        let packet = Packet::audio(0xABCDEF00, 1, audio_data.clone());
        
        assert!(packet.verify_checksum());
        
        if let PacketType::Audio { channel, data } = &packet.packet_type {
            assert_eq!(*channel, 1);
            assert_eq!(data, &audio_data);
        } else {
            panic!("Wrong packet type");
        }
    }
    
    #[test]
    fn test_encrypted_audio_packet() {
        let nonce = [0x11u8; 12];
        let tag = [0x22u8; 16];
        let data = vec![0x33, 0x44, 0x55];
        
        let packet = Packet::encrypted_audio(0x12345678, 5, nonce, tag, data.clone());
        
        assert!(packet.verify_checksum());
        assert!(packet.is_encrypted());
        
        if let PacketType::EncryptedAudio { channel, nonce: n, auth_tag, data: d } = &packet.packet_type {
            assert_eq!(*channel, 5);
            assert_eq!(*n, nonce);
            assert_eq!(*auth_tag, tag);
            assert_eq!(*d, data);
        } else {
            panic!("Wrong packet type");
        }
    }

    #[test]
    fn test_checksum_verification() {
        let mut packet = Packet::discovery(0x11111111, "Device".to_string(), 1);
        
        // Tamper with data
        packet.device_id = 0x22222222;
        
        // Checksum should fail
        assert!(!packet.verify_checksum());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let original = Packet::keep_alive(0xFFFFFFFF);
        let serialized = original.serialize().unwrap();
        let deserialized = Packet::deserialize(&serialized).unwrap();
        
        assert_eq!(deserialized.device_id, original.device_id);
        assert_eq!(deserialized.version, original.version);
    }
    
    #[test]
    fn test_key_exchange_packet() {
        let key = [0xAB; 32];
        let packet = Packet::key_exchange(0x12345678, key);
        
        assert!(packet.verify_checksum());
        
        let serialized = packet.serialize().unwrap();
        let deserialized = Packet::deserialize(&serialized).unwrap();
        
        if let PacketType::KeyExchange { public_key } = deserialized.packet_type {
            assert_eq!(public_key, key);
        } else {
            panic!("Wrong packet type");
        }
    }
}
