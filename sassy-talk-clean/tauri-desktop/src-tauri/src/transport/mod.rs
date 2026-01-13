// Transport Layer - Cross-platform networking
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
//
// Strategy:
// - WiFi UDP Multicast: Primary transport (works on ALL platforms including iOS)
// - BLE: Discovery signaling only
// - Bluetooth Classic: Android-to-Android optimization

mod wifi;

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio::net::UdpSocket;
use socket2::{Socket, Domain, Type, Protocol};
use serde::{Serialize, Deserialize};
use thiserror::Error;
use tracing::{info, warn, error, debug};

use crate::{MULTICAST_ADDR, DISCOVERY_PORT, AUDIO_PORT, PEER_TIMEOUT_MS};
use crate::protocol::{Packet, PacketType};

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Socket error: {0}")]
    Socket(#[from] std::io::Error),
    #[error("Peer not found: {0:08X}")]
    PeerNotFound(u32),
    #[error("Already connected")]
    AlreadyConnected,
    #[error("Not connected")]
    NotConnected,
}

/// Information about a discovered peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub device_id: u32,
    pub device_name: String,
    pub address: String,
    pub channel: u8,
    pub signal_strength: i32,
    pub last_seen: u64, // Unix timestamp ms
}

/// Transport manager handles all networking
pub struct TransportManager {
    device_id: u32,
    device_name: String,
    channel: u8,
    
    // Discovered peers
    peers: HashMap<u32, PeerInfo>,
    
    // Sockets
    discovery_socket: Option<Arc<UdpSocket>>,
    audio_socket: Option<Arc<UdpSocket>>,
    
    // State
    is_discovering: bool,
    connected_peers: Vec<u32>,
    
    // Channels for audio data
    audio_tx: Option<mpsc::Sender<Vec<u8>>>,
    audio_rx: Option<mpsc::Receiver<Vec<u8>>>,
}

impl TransportManager {
    pub fn new(device_id: u32, device_name: String) -> Self {
        Self {
            device_id,
            device_name,
            channel: 1,
            peers: HashMap::new(),
            discovery_socket: None,
            audio_socket: None,
            is_discovering: false,
            connected_peers: Vec::new(),
            audio_tx: None,
            audio_rx: None,
        }
    }

    /// Start device discovery via UDP multicast
    pub async fn start_discovery(&mut self) -> Result<(), TransportError> {
        if self.is_discovering {
            return Ok(());
        }

        info!("Starting UDP multicast discovery on {}:{}", MULTICAST_ADDR, DISCOVERY_PORT);

        // Create multicast socket
        let socket = Self::create_multicast_socket(DISCOVERY_PORT)?;
        let socket = UdpSocket::from_std(socket.into())?;
        self.discovery_socket = Some(Arc::new(socket));

        self.is_discovering = true;

        // Spawn discovery broadcast task
        let socket = self.discovery_socket.clone().unwrap();
        let device_id = self.device_id;
        let device_name = self.device_name.clone();
        let channel = self.channel;

        tokio::spawn(async move {
            let multicast_addr: SocketAddr = format!("{}:{}", MULTICAST_ADDR, DISCOVERY_PORT)
                .parse()
                .unwrap();

            loop {
                // Create discovery packet
                let packet = Packet::new_discovery(device_id, &device_name, channel);
                let data = packet.serialize();

                if let Err(e) = socket.send_to(&data, multicast_addr).await {
                    warn!("Discovery broadcast failed: {}", e);
                }

                tokio::time::sleep(Duration::from_millis(crate::DISCOVERY_INTERVAL_MS)).await;
            }
        });

        // Spawn discovery receive task
        let socket = self.discovery_socket.clone().unwrap();
        let device_id = self.device_id;
        
        // This would normally update self.peers but we'd need interior mutability
        // For now, this is a stub
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        if let Some(packet) = Packet::deserialize(&buf[..len]) {
                            if packet.sender_id != device_id {
                                debug!("Discovered peer {:08X} at {}", packet.sender_id, addr);
                                // TODO: Update peers map via channel
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Discovery receive error: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop discovery
    pub async fn stop_discovery(&mut self) {
        self.is_discovering = false;
        self.discovery_socket = None;
        info!("Discovery stopped");
    }

    /// Create a UDP socket with multicast enabled
    fn create_multicast_socket(port: u16) -> Result<Socket, TransportError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

        // Allow address reuse
        socket.set_reuse_address(true)?;
        
        #[cfg(not(windows))]
        socket.set_reuse_port(true)?;

        // Bind to all interfaces
        let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket.bind(&addr.into())?;

        // Join multicast group
        let multicast_addr: Ipv4Addr = MULTICAST_ADDR.parse().unwrap();
        socket.join_multicast_v4(&multicast_addr, &Ipv4Addr::UNSPECIFIED)?;

        // Set multicast TTL (local network only)
        socket.set_multicast_ttl_v4(1)?;

        // Enable loopback for testing on same device
        socket.set_multicast_loop_v4(true)?;

        // Non-blocking
        socket.set_nonblocking(true)?;

        Ok(socket)
    }

    /// Connect to a specific peer
    pub async fn connect_to_peer(&mut self, peer_id: u32) -> Result<(), TransportError> {
        if self.connected_peers.contains(&peer_id) {
            return Err(TransportError::AlreadyConnected);
        }

        let peer = self.peers.get(&peer_id)
            .ok_or(TransportError::PeerNotFound(peer_id))?
            .clone();

        info!("Connecting to peer {} ({:08X})", peer.device_name, peer_id);

        // Create audio socket if not exists
        if self.audio_socket.is_none() {
            let socket = Self::create_audio_socket()?;
            let socket = UdpSocket::from_std(socket.into())?;
            self.audio_socket = Some(Arc::new(socket));
        }

        self.connected_peers.push(peer_id);

        Ok(())
    }

    /// Create audio data socket
    fn create_audio_socket() -> Result<Socket, TransportError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

        socket.set_reuse_address(true)?;
        
        // Bind to audio port
        let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, AUDIO_PORT);
        socket.bind(&addr.into())?;

        // Join multicast for receiving
        let multicast_addr: Ipv4Addr = MULTICAST_ADDR.parse().unwrap();
        socket.join_multicast_v4(&multicast_addr, &Ipv4Addr::UNSPECIFIED)?;

        socket.set_nonblocking(true)?;

        Ok(socket)
    }

    /// Disconnect from all peers
    pub async fn disconnect_all(&mut self) {
        self.connected_peers.clear();
        self.audio_socket = None;
        info!("Disconnected from all peers");
    }

    /// Send audio packet to connected peers
    pub async fn send_audio(&self, data: &[u8]) -> Result<(), TransportError> {
        let socket = self.audio_socket.as_ref()
            .ok_or(TransportError::NotConnected)?;

        let packet = Packet::new_audio(self.device_id, self.channel, data);
        let serialized = packet.serialize();

        // Multicast to all peers on this channel
        let addr: SocketAddr = format!("{}:{}", MULTICAST_ADDR, AUDIO_PORT)
            .parse()
            .unwrap();

        socket.send_to(&serialized, addr).await?;

        Ok(())
    }

    /// Get list of nearby peers
    pub fn get_nearby_peers(&self) -> Vec<PeerInfo> {
        self.peers.values()
            .filter(|p| p.channel == self.channel)
            .cloned()
            .collect()
    }

    /// Get specific peer info
    pub fn get_peer_info(&self, peer_id: u32) -> Option<PeerInfo> {
        self.peers.get(&peer_id).cloned()
    }

    /// Set current channel
    pub fn set_channel(&mut self, channel: u8) {
        self.channel = channel;
        info!("Transport channel set to {}", channel);
    }

    /// Clean up stale peers
    pub fn cleanup_stale_peers(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        self.peers.retain(|_, peer| {
            now - peer.last_seen < PEER_TIMEOUT_MS
        });
    }
}
