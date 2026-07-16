// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-7MQV4VWOFS4I
/// Transport Module - UDP Multicast for Cross-Platform Audio
/// 
/// Uses UDP multicast for automatic peer discovery and audio transmission
/// Works on WiFi networks (all desktop platforms)
/// 
/// Features:
/// - Random port selection per session for security
/// - End-to-end encryption with X25519 key exchange
/// - Configurable multicast address
/// 
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.

pub mod cellular;
pub mod control;
pub mod discovery;
pub mod liveness;
pub mod manager;

pub use manager::{TransportManager, PeerInfo, TransportConfig};
pub use discovery::DiscoveryService;
pub use cellular::{CellularConfig, CellularState, CellularTransport};

use crate::constants;

// Re-export constants for backwards compatibility
pub use constants::{
    DEFAULT_MULTICAST_ADDR as MULTICAST_ADDR,
    DEFAULT_MULTICAST_PORT as MULTICAST_PORT,
    MAX_PACKET_SIZE,
    PEER_TIMEOUT_SECS,
    BEACON_INTERVAL_SECS,
    PORT_RANGE_START,
    PORT_RANGE_END,
};

/// One received audio frame handed to the decode pipeline. Carries the sender
/// identity + wire timestamp so the RX side can key a PER-SENDER Opus decoder
/// (Opus is stateful) and order frames by their real timestamp, instead of
/// decoding every sender through one decoder in raw arrival order.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// Wire `sender_id` (or a synthetic id for the legacy per-peer path).
    pub sender: String,
    /// Wire timestamp in ms; 0 when the transport carries none (legacy path),
    /// in which case the RX loop synthesizes a per-sender clock.
    pub timestamp: u64,
    /// Raw Opus payload.
    pub opus: Vec<u8>,
}

/// Transport error types
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Failed to bind socket: {0}")]
    BindError(String),
    
    #[error("Failed to join multicast: {0}")]
    MulticastError(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Invalid peer: {0}")]
    InvalidPeer(String),
    
    #[error("Encryption error: {0}")]
    EncryptionError(String),
    
    #[error("Key exchange failed: {0}")]
    KeyExchangeError(String),
    
    #[error("No port available in range")]
    NoPortAvailable,
}
