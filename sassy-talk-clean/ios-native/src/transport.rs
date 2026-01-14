/// Transport Module for iOS
/// 
/// UDP multicast for WiFi-based communication
/// Same approach as desktop version

use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use thiserror::Error;

/// Multicast address
pub const MULTICAST_ADDR: &str = "239.255.42.42";

/// Multicast port
pub const MULTICAST_PORT: u16 = 5555;

/// Peer timeout (30 seconds)
const PEER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Failed to bind socket: {0}")]
    BindError(String),
    
    #[error("Failed to join multicast: {0}")]
    MulticastError(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

/// Peer information
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub device_id: u32,
    pub device_name: String,
    pub address: SocketAddr,
    pub channel: u8,
    pub last_seen: SystemTime,
}

/// Transport manager
pub struct TransportManager {
    socket: Arc<Mutex<Option<Socket>>>,
    multicast_addr: SocketAddr,
    peers: Arc<Mutex<HashMap<u32, PeerInfo>>>,
}

impl TransportManager {
    /// Create new transport manager
    pub fn new() -> Result<Self, TransportError> {
        let multicast_addr = SocketAddr::new(
            IpAddr::V4(MULTICAST_ADDR.parse().unwrap()),
            MULTICAST_PORT,
        );
        
        Ok(Self {
            socket: Arc::new(Mutex::new(None)),
            multicast_addr,
            peers: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    
    /// Start transport
    pub fn start(&self) -> Result<(), TransportError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), MULTICAST_PORT);
        socket.bind(&bind_addr.into())?;
        
        // Join multicast group
        let multicast_ip: Ipv4Addr = MULTICAST_ADDR.parse().unwrap();
        socket
            .join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| TransportError::MulticastError(e.to_string()))?;
        
        *self.socket.lock().unwrap() = Some(socket);
        Ok(())
    }
    
    /// Stop transport
    pub fn stop(&self) {
        *self.socket.lock().unwrap() = None;
    }
    
    /// Send packet
    pub fn send(&self, data: &[u8]) -> Result<(), TransportError> {
        let socket = self.socket.lock().unwrap();
        if let Some(sock) = socket.as_ref() {
            sock.send_to(data, &self.multicast_addr.into())?;
        }
        Ok(())
    }
    
    /// Receive packet
    pub fn receive(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), TransportError> {
        let socket = self.socket.lock().unwrap();
        if let Some(sock) = socket.as_ref() {
            match sock.recv_from(buffer) {
                Ok((size, addr)) => {
                    let socket_addr = match addr.as_socket() {
                        Some(sa) => sa,
                        None => return Err(TransportError::IoError(
                            std::io::Error::new(std::io::ErrorKind::Other, "Invalid address")
                        )),
                    };
                    Ok((size, socket_addr))
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    Err(TransportError::IoError(
                        std::io::Error::new(std::io::ErrorKind::WouldBlock, "No data")
                    ))
                }
                Err(e) => Err(TransportError::IoError(e)),
            }
        } else {
            Err(TransportError::IoError(
                std::io::Error::new(std::io::ErrorKind::NotConnected, "Socket not initialized")
            ))
        }
    }
    
    /// Add or update peer
    pub fn update_peer(&self, peer: PeerInfo) {
        let mut peers = self.peers.lock().unwrap();
        peers.insert(peer.device_id, peer);
    }
    
    /// Get active peers
    pub fn get_peers(&self) -> Vec<PeerInfo> {
        let mut peers = self.peers.lock().unwrap();
        
        // Remove stale peers
        let now = SystemTime::now();
        peers.retain(|_, peer| {
            now.duration_since(peer.last_seen).unwrap_or(Duration::MAX) < PEER_TIMEOUT
        });
        
        peers.values().cloned().collect()
    }
    
    /// Remove peer
    pub fn remove_peer(&self, device_id: u32) {
        let mut peers = self.peers.lock().unwrap();
        peers.remove(&device_id);
    }
}

impl Default for TransportManager {
    fn default() -> Self {
        Self::new().expect("Failed to create transport manager")
    }
}
