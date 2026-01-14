/// Protocol Module - Packet Format
/// 
/// Defines wire protocol for UDP multicast communication
/// (Same protocol as desktop version for cross-platform compatibility)

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    
    #[error("Checksum mismatch")]
    ChecksumMismatch,
}

/// Packet types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PacketType {
    /// Discovery beacon
    Discovery {
        device_name: String,
        channel: u8,
    },
    
    /// Audio data
    Audio {
        channel: u8,
        data: Vec<u8>,
    },
    
    /// Keep-alive
    KeepAlive,
}

/// Network packet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Packet {
    pub version: u8,
    pub device_id: u32,
    pub packet_type: PacketType,
    pub timestamp: u64,
    pub checksum: u32,
}

impl Packet {
    const VERSION: u8 = 1;
    
    /// Create discovery packet
    pub fn discovery(device_id: u32, device_name: String, channel: u8) -> Self {
        let timestamp = Self::current_timestamp();
        
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
        let timestamp = Self::current_timestamp();
        
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
        let timestamp = Self::current_timestamp();
        
        let packet = Self {
            version: Self::VERSION,
            device_id,
            packet_type: PacketType::KeepAlive,
            timestamp,
            checksum: 0,
        };
        
        packet.with_checksum()
    }
    
    /// Serialize to bytes
    pub fn serialize(&self) -> Result<Vec<u8>, ProtocolError> {
        bincode::serialize(self)
            .map_err(|e| ProtocolError::SerializationError(e.to_string()))
    }
    
    /// Deserialize from bytes
    pub fn deserialize(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let packet: Packet = bincode::deserialize(bytes)
            .map_err(|e| ProtocolError::DeserializationError(e.to_string()))?;
        
        // Verify checksum
        let expected_checksum = packet.checksum;
        let mut packet_for_check = packet.clone();
        packet_for_check.checksum = 0;
        let calculated_checksum = packet_for_check.calculate_checksum();
        
        if expected_checksum != calculated_checksum {
            return Err(ProtocolError::ChecksumMismatch);
        }
        
        Ok(packet)
    }
    
    /// Calculate CRC32 checksum
    fn calculate_checksum(&self) -> u32 {
        // Simple checksum (you can use crc crate for proper CRC32)
        let bytes = bincode::serialize(self).unwrap_or_default();
        let mut checksum: u32 = 0;
        for byte in bytes {
            checksum = checksum.wrapping_add(byte as u32);
        }
        checksum
    }
    
    /// Add checksum to packet
    fn with_checksum(mut self) -> Self {
        self.checksum = self.calculate_checksum();
        self
    }
    
    /// Get current timestamp
    fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
