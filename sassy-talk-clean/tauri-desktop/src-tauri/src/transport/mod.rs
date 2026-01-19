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

pub mod discovery;
pub mod manager;

pub use manager::{TransportManager, PeerInfo, TransportConfig};
pub use discovery::DiscoveryService;

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
