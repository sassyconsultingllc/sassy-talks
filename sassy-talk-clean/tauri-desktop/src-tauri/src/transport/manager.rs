/// Transport Manager - UDP Multicast Communication
/// 
/// Handles peer discovery, audio transmission, and receiving

use super::{TransportError, MULTICAST_ADDR, MULTICAST_PORT, MAX_PACKET_SIZE, PEER_TIMEOUT_SECS};
use crate::protocol::{Packet, PacketType};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use tokio::sync::mpsc;
use tokio::time;
use tracing::{error, info, warn};

/// Peer information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PeerInfo {
    pub device_id: u32,
    pub device_name: String,
    pub address: SocketAddr,
    pub last_seen: u64,
    pub channel: u8,
}

impl PeerInfo {
    pub fn is_active(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.last_seen < PEER_TIMEOUT_SECS
    }
}

/// Transport manager for UDP multicast
pub struct TransportManager {
    // Socket for sending/receiving
    socket: Arc<Socket>,
    multicast_addr: SocketAddr,
    
    // Local device info
    device_id: u32,
    device_name: String,
    current_channel: Arc<RwLock<u8>>,
    
    // Peer tracking
    peers: Arc<RwLock<HashMap<u32, PeerInfo>>>,
    
    // Control
    running: Arc<AtomicBool>,
    
    // Channels for audio data
    audio_tx: mpsc::UnboundedSender<Vec<u8>>,
    audio_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<Vec<u8>>>>>,
}

impl TransportManager {
    /// Create new transport manager
    pub fn new(device_id: u32, device_name: String) -> Result<Self, TransportError> {
        info!("Initializing transport manager");
        info!("Device ID: {:08X}", device_id);
        info!("Device Name: {}", device_name);
        
        // Create UDP socket
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        // Allow multiple processes to bind to same port
        socket.set_reuse_address(true)
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        #[cfg(not(target_os = "windows"))]
        socket.set_reuse_port(true)
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        // Bind to multicast port
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), MULTICAST_PORT);
        socket.bind(&bind_addr.into())
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        // Join multicast group
        let multicast_ip: Ipv4Addr = MULTICAST_ADDR.parse()
            .map_err(|e| TransportError::MulticastError(format!("Invalid multicast address: {}", e)))?;
        
        socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| TransportError::MulticastError(e.to_string()))?;
        
        // Set multicast TTL
        socket.set_multicast_ttl_v4(32)
            .map_err(|e| TransportError::MulticastError(e.to_string()))?;
        
        // Set non-blocking
        socket.set_nonblocking(true)
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        let multicast_addr = SocketAddr::new(IpAddr::V4(multicast_ip), MULTICAST_PORT);
        
        info!("✓ UDP socket bound to {}", bind_addr);
        info!("✓ Joined multicast group {}", multicast_addr);
        
        // Create audio channel
        let (audio_tx, audio_rx) = mpsc::unbounded_channel();
        
        Ok(Self {
            socket: Arc::new(socket),
            multicast_addr,
            device_id,
            device_name,
            current_channel: Arc::new(RwLock::new(1)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(false)),
            audio_tx,
            audio_rx: Arc::new(RwLock::new(Some(audio_rx))),
        })
    }
    
    /// Start transport (discovery + receive loop)
    pub async fn start(&self) -> Result<(), TransportError> {
        info!("Starting transport manager");
        
        self.running.store(true, Ordering::Relaxed);
        
        // Start discovery beacon
        let device_id = self.device_id;
        let device_name = self.device_name.clone();
        let socket = Arc::clone(&self.socket);
        let multicast_addr = self.multicast_addr;
        let channel = Arc::clone(&self.current_channel);
        let running = Arc::clone(&self.running);
        
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(super::BEACON_INTERVAL_SECS));
            
            while running.load(Ordering::Relaxed) {
                interval.tick().await;
                
                let current_channel = *channel.read().unwrap();
                let packet = Packet::discovery(device_id, device_name.clone(), current_channel);
                
                if let Ok(data) = packet.serialize() {
                    if let Err(e) = socket.send_to(&data, &multicast_addr.into()) {
                        error!("Failed to send discovery: {}", e);
                    }
                }
            }
        });
        
        // Start receive loop
        let socket_rx = Arc::clone(&self.socket);
        let peers = Arc::clone(&self.peers);
        let audio_tx = self.audio_tx.clone();
        let running_rx = Arc::clone(&self.running);
        let device_id_rx = self.device_id;
        
        tokio::spawn(async move {
            let mut buf = vec![0u8; MAX_PACKET_SIZE];
            
            while running_rx.load(Ordering::Relaxed) {
                match socket_rx.recv_from(&mut buf) {
                    Ok((size, src_addr)) => {
                        if let Ok(packet) = Packet::deserialize(&buf[..size]) {
                            // Ignore own packets
                            if packet.device_id == device_id_rx {
                                continue;
                            }
                            
                            match packet.packet_type {
                                PacketType::Discovery { device_name, channel } => {
                                    // Update peer info
                                    let peer = PeerInfo {
                                        device_id: packet.device_id,
                                        device_name,
                                        address: src_addr.as_socket().unwrap(),
                                        last_seen: SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap()
                                            .as_secs(),
                                        channel,
                                    };
                                    
                                    peers.write().unwrap().insert(packet.device_id, peer);
                                }
                                PacketType::Audio { channel, data } => {
                                    // Forward audio to audio channel
                                    let _ = audio_tx.send(data);
                                }
                                PacketType::KeepAlive => {
                                    // Update last seen
                                    if let Some(peer) = peers.write().unwrap().get_mut(&packet.device_id) {
                                        peer.last_seen = SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap()
                                            .as_secs();
                                    }
                                }
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No data available, sleep briefly
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(e) => {
                        error!("Receive error: {}", e);
                    }
                }
            }
        });
        
        info!("✓ Transport manager started");
        Ok(())
    }
    
    /// Stop transport
    pub fn stop(&self) {
        info!("Stopping transport manager");
        self.running.store(false, Ordering::Relaxed);
    }
    
    /// Send audio data
    pub fn send_audio(&self, audio_data: &[u8]) -> Result<(), TransportError> {
        let channel = *self.current_channel.read().unwrap();
        let packet = Packet::audio(self.device_id, channel, audio_data.to_vec());
        
        let data = packet.serialize()
            .map_err(|e| TransportError::SerializationError(e))?;
        
        self.socket.send_to(&data, &self.multicast_addr.into())
            .map_err(|e| TransportError::IoError(e))?;
        
        Ok(())
    }
    
    /// Get received audio receiver
    pub fn take_audio_receiver(&self) -> Option<mpsc::UnboundedReceiver<Vec<u8>>> {
        self.audio_rx.write().unwrap().take()
    }
    
    /// Get list of active peers
    pub fn get_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().unwrap();
        peers.values()
            .filter(|p| p.is_active())
            .cloned()
            .collect()
    }
    
    /// Get peer count
    pub fn peer_count(&self) -> usize {
        self.get_peers().len()
    }
    
    /// Set channel
    pub fn set_channel(&self, channel: u8) {
        *self.current_channel.write().unwrap() = channel;
        info!("Channel changed to {}", channel);
    }
    
    /// Get current channel
    pub fn get_channel(&self) -> u8 {
        *self.current_channel.read().unwrap()
    }
    
    /// Check if running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for TransportManager {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_transport_creation() {
        let transport = TransportManager::new(0x12345678, "Test Device".to_string());
        assert!(transport.is_ok());
    }

    #[tokio::test]
    async fn test_channel_change() {
        let transport = TransportManager::new(0x12345678, "Test Device".to_string()).unwrap();
        
        assert_eq!(transport.get_channel(), 1);
        transport.set_channel(42);
        assert_eq!(transport.get_channel(), 42);
    }
}
