// Wire Protocol - Packet structure and serialization
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
//
// Packet Structure (v1):
// ┌────────────────────────────────────────────────────────────┐
// │ Byte 0     │ Version (0x01)                                │
// │ Byte 1     │ Type (Audio/Control/Discovery/Key)           │
// │ Byte 2-3   │ Sequence Number (u16 BE)                      │
// │ Byte 4-7   │ Timestamp (u32 BE, ms since epoch % 2^32)    │
// │ Byte 8-11  │ Sender ID (u32 BE)                            │
// │ Byte 12    │ Channel (1-16)                                │
// │ Byte 13-15 │ Reserved                                      │
// │ Byte 16-27 │ Nonce (12 bytes) - for encrypted packets     │
// │ Byte 28-29 │ Payload Length (u16 BE)                       │
// │ Byte 30+   │ Payload (encrypted for audio)                │
// │ Last 16    │ GCM Auth Tag (encrypted packets only)        │
// └────────────────────────────────────────────────────────────┘

use serde::{Serialize, Deserialize};

/// Protocol version
pub const PROTOCOL_VERSION: u8 = 0x01;

/// Packet types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum PacketType {
    Audio = 0x00,
    Control = 0x01,
    Discovery = 0x02,
    KeyExchange = 0x03,
    Heartbeat = 0x04,
}

impl TryFrom<u8> for PacketType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(PacketType::Audio),
            0x01 => Ok(PacketType::Control),
            0x02 => Ok(PacketType::Discovery),
            0x03 => Ok(PacketType::KeyExchange),
            0x04 => Ok(PacketType::Heartbeat),
            _ => Err(()),
        }
    }
}

/// Control message types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ControlType {
    PttStart = 0x01,
    PttEnd = 0x02,
    ChannelChange = 0x03,
    RogerBeep = 0x04,
    Disconnect = 0x05,
}

/// Network packet
#[derive(Debug, Clone)]
pub struct Packet {
    pub version: u8,
    pub packet_type: PacketType,
    pub sequence: u16,
    pub timestamp: u32,
    pub sender_id: u32,
    pub channel: u8,
    pub nonce: [u8; 12],
    pub payload: Vec<u8>,
    pub auth_tag: Option<[u8; 16]>,
}

impl Packet {
    /// Create new audio packet
    pub fn new_audio(sender_id: u32, channel: u8, audio_data: &[u8]) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            packet_type: PacketType::Audio,
            sequence: 0, // Will be set by transport
            timestamp: Self::current_timestamp(),
            sender_id,
            channel,
            nonce: Self::generate_nonce(),
            payload: audio_data.to_vec(),
            auth_tag: None, // Set after encryption
        }
    }

    /// Create discovery packet
    pub fn new_discovery(sender_id: u32, device_name: &str, channel: u8) -> Self {
        // Discovery payload: device name (null-terminated)
        let mut payload = device_name.as_bytes().to_vec();
        payload.push(0);

        Self {
            version: PROTOCOL_VERSION,
            packet_type: PacketType::Discovery,
            sequence: 0,
            timestamp: Self::current_timestamp(),
            sender_id,
            channel,
            nonce: [0; 12], // Not encrypted
            payload,
            auth_tag: None,
        }
    }

    /// Create control packet
    pub fn new_control(sender_id: u32, channel: u8, control_type: ControlType) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            packet_type: PacketType::Control,
            sequence: 0,
            timestamp: Self::current_timestamp(),
            sender_id,
            channel,
            nonce: Self::generate_nonce(),
            payload: vec![control_type as u8],
            auth_tag: None,
        }
    }

    /// Create heartbeat packet
    pub fn new_heartbeat(sender_id: u32, channel: u8) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            packet_type: PacketType::Heartbeat,
            sequence: 0,
            timestamp: Self::current_timestamp(),
            sender_id,
            channel,
            nonce: [0; 12],
            payload: Vec::new(),
            auth_tag: None,
        }
    }

    /// Serialize packet to bytes
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(30 + self.payload.len() + 16);

        // Header
        data.push(self.version);
        data.push(self.packet_type as u8);
        data.extend_from_slice(&self.sequence.to_be_bytes());
        data.extend_from_slice(&self.timestamp.to_be_bytes());
        data.extend_from_slice(&self.sender_id.to_be_bytes());
        data.push(self.channel);
        data.extend_from_slice(&[0, 0, 0]); // Reserved
        data.extend_from_slice(&self.nonce);
        data.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());

        // Payload
        data.extend_from_slice(&self.payload);

        // Auth tag (if present)
        if let Some(tag) = &self.auth_tag {
            data.extend_from_slice(tag);
        }

        data
    }

    /// Deserialize packet from bytes
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 30 {
            return None;
        }

        let version = data[0];
        if version != PROTOCOL_VERSION {
            return None;
        }

        let packet_type = PacketType::try_from(data[1]).ok()?;
        let sequence = u16::from_be_bytes([data[2], data[3]]);
        let timestamp = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let sender_id = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let channel = data[12];
        // data[13..16] reserved

        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&data[16..28]);

        let payload_len = u16::from_be_bytes([data[28], data[29]]) as usize;

        if data.len() < 30 + payload_len {
            return None;
        }

        let payload = data[30..30 + payload_len].to_vec();

        // Check for auth tag
        let auth_tag = if data.len() >= 30 + payload_len + 16 {
            let mut tag = [0u8; 16];
            tag.copy_from_slice(&data[30 + payload_len..30 + payload_len + 16]);
            Some(tag)
        } else {
            None
        };

        Some(Self {
            version,
            packet_type,
            sequence,
            timestamp,
            sender_id,
            channel,
            nonce,
            payload,
            auth_tag,
        })
    }

    /// Get current timestamp (ms since epoch, wrapping)
    fn current_timestamp() -> u32 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        (now.as_millis() % (u32::MAX as u128 + 1)) as u32
    }

    /// Generate random nonce
    fn generate_nonce() -> [u8; 12] {
        let mut nonce = [0u8; 12];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut nonce);
        nonce
    }

    /// Extract device name from discovery payload
    pub fn get_device_name(&self) -> Option<String> {
        if self.packet_type != PacketType::Discovery {
            return None;
        }

        // Find null terminator
        let end = self.payload.iter().position(|&b| b == 0)?;
        String::from_utf8(self.payload[..end].to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_packet_roundtrip() {
        let audio_data = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let packet = Packet::new_audio(0x12345678, 5, &audio_data);

        let serialized = packet.serialize();
        let deserialized = Packet::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.version, PROTOCOL_VERSION);
        assert_eq!(deserialized.packet_type, PacketType::Audio);
        assert_eq!(deserialized.sender_id, 0x12345678);
        assert_eq!(deserialized.channel, 5);
        assert_eq!(deserialized.payload, audio_data);
    }

    #[test]
    fn test_discovery_packet() {
        let packet = Packet::new_discovery(0xDEADBEEF, "Test Device", 1);

        let serialized = packet.serialize();
        let deserialized = Packet::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.packet_type, PacketType::Discovery);
        assert_eq!(deserialized.get_device_name(), Some("Test Device".to_string()));
    }
}
