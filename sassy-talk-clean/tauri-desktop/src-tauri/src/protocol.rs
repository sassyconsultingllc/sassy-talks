/// Protocol - Packet Format and Serialization
/// 
/// Defines the wire protocol for SassyTalkie UDP multicast

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
}

/// Packet types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PacketType {
    /// Discovery beacon (announces presence)
    Discovery {
        device_name: String,
        channel: u8,
    },
    
    /// Audio data
    Audio {
        channel: u8,
        data: Vec<u8>,
    },
    
    /// Keep-alive (maintains connection)
    KeepAlive,
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
    const VERSION: u8 = 1;
    
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
            PacketType::Audio { channel, data } => {
                hasher.update(&[1u8]); // Type discriminant
                hasher.update(&[*channel]);
                hasher.update(data);
            }
            PacketType::KeepAlive => {
                hasher.update(&[2u8]); // Type discriminant
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
        
        // Verify version
        if packet.version != Self::VERSION {
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
            PacketType::Audio { data, .. } => 50 + data.len(),
            PacketType::KeepAlive => 50,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_packet() {
        let packet = Packet::discovery(0x12345678, "Test Device".to_string(), 42);
        
        assert_eq!(packet.version, 1);
        assert_eq!(packet.device_id, 0x12345678);
        assert!(packet.verify_checksum());
        
        // Serialize and deserialize
        let serialized = packet.serialize().unwrap();
        let deserialized = Packet::deserialize(&serialized).unwrap();
        
        assert_eq!(deserialized.device_id, packet.device_id);
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
}
