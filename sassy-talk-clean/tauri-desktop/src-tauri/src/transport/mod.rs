/// Transport Module - UDP Multicast for Cross-Platform Audio
/// 
/// Uses UDP multicast for automatic peer discovery and audio transmission
/// Works on WiFi networks (all desktop platforms)

pub mod discovery;
pub mod manager;

pub use manager::{TransportManager, PeerInfo};
pub use discovery::DiscoveryService;

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Multicast group address (239.255.42.42)
pub const MULTICAST_ADDR: &str = "239.255.42.42";

/// Multicast port
pub const MULTICAST_PORT: u16 = 5555;

/// Maximum UDP packet size
pub const MAX_PACKET_SIZE: usize = 1500;

/// Peer timeout (seconds)
pub const PEER_TIMEOUT_SECS: u64 = 30;

/// Discovery beacon interval (seconds)
pub const BEACON_INTERVAL_SECS: u64 = 5;

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
}
