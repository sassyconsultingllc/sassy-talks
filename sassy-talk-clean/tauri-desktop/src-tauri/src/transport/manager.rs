/// Transport Manager - UDP Multicast Communication with Encryption
/// 
/// Features:
/// - Random port selection per session
/// - X25519 key exchange with peers
/// - AES-256-GCM encrypted audio packets
/// - Automatic peer discovery
/// 
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use super::{TransportError, MAX_PACKET_SIZE, PEER_TIMEOUT_SECS};
use crate::constants::{DEFAULT_MULTICAST_ADDR, DEFAULT_MULTICAST_PORT, PORT_RANGE_START, PORT_RANGE_END, KEEPALIVE_INTERVAL_SECS};
use crate::protocol::{Packet, PacketType};
use crate::security::CryptoEngine;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use tokio::sync::mpsc;
use tokio::time;
use rand::Rng;
use tracing::{error, info, warn, debug};

/// Transport configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransportConfig {
    /// Multicast address
    pub multicast_addr: String,
    /// Use random port each session
    pub use_random_port: bool,
    /// Fixed port (used if use_random_port is false)
    pub fixed_port: u16,
    /// Enable encryption
    pub encryption_enabled: bool,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            multicast_addr: DEFAULT_MULTICAST_ADDR.to_string(),
            use_random_port: true,  // Default to random port for security
            fixed_port: DEFAULT_MULTICAST_PORT,
            encryption_enabled: true,  // Default to encrypted
        }
    }
}

/// Peer information with encryption state
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PeerInfo {
    pub device_id: u32,
    pub device_name: String,
    pub address: SocketAddr,
    pub last_seen: u64,
    pub channel: u8,
    /// Peer's public key for encryption (32 bytes, hex encoded for serialization)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    /// Whether we have completed key exchange with this peer
    #[serde(default)]
    pub key_exchanged: bool,
}

impl PeerInfo {
    pub fn is_active(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.last_seen < PEER_TIMEOUT_SECS
    }
    
    /// Get public key as bytes
    pub fn get_public_key_bytes(&self) -> Option<[u8; 32]> {
        self.public_key.as_ref().and_then(|hex| {
            let bytes = hex::decode(hex).ok()?;
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Some(arr)
            } else {
                None
            }
        })
    }
}

/// Transport manager for UDP multicast with encryption
pub struct TransportManager {
    // Socket for sending/receiving
    socket: Arc<Socket>,
    multicast_addr: SocketAddr,
    
    // Actual bound port (may be random)
    bound_port: Arc<AtomicU16>,
    
    // Local device info
    device_id: u32,
    device_name: String,
    current_channel: Arc<RwLock<u8>>,
    
    // Configuration
    config: Arc<RwLock<TransportConfig>>,
    
    // Peer tracking
    peers: Arc<RwLock<HashMap<u32, PeerInfo>>>,
    
    // Encryption engines per peer
    crypto_engines: Arc<RwLock<HashMap<u32, CryptoEngine>>>,
    
    // Our public key for sharing
    our_public_key: Arc<RwLock<Option<[u8; 32]>>>,
    
    // Control
    running: Arc<AtomicBool>,
    
    // Channels for audio data
    audio_tx: mpsc::UnboundedSender<Vec<u8>>,
    audio_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<Vec<u8>>>>>,
}

impl TransportManager {
    /// Create new transport manager with configuration
    pub fn new(device_id: u32, device_name: String) -> Result<Self, TransportError> {
        Self::with_config(device_id, device_name, TransportConfig::default())
    }
    
    /// Create with custom configuration
    pub fn with_config(device_id: u32, device_name: String, config: TransportConfig) -> Result<Self, TransportError> {
        info!("Initializing transport manager");
        info!("Device ID: {:08X}", device_id);
        info!("Device Name: {}", device_name);
        info!("Config: {:?}", config);
        
        // Determine port to use
        let port = if config.use_random_port {
            Self::find_random_port()?
        } else {
            config.fixed_port
        };
        
        info!("Using port: {}", port);
        
        // Create UDP socket
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        // Allow multiple processes to bind to same port
        socket.set_reuse_address(true)
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        #[cfg(not(target_os = "windows"))]
        socket.set_reuse_port(true)
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        // Bind to selected port
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        socket.bind(&bind_addr.into())
            .map_err(|e| TransportError::BindError(format!("Port {}: {}", port, e)))?;
        
        // Parse and join multicast group
        let multicast_ip: Ipv4Addr = config.multicast_addr.parse()
            .map_err(|e| TransportError::MulticastError(format!("Invalid multicast address: {}", e)))?;
        
        socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| TransportError::MulticastError(e.to_string()))?;
        
        // Set multicast TTL
        socket.set_multicast_ttl_v4(32)
            .map_err(|e| TransportError::MulticastError(e.to_string()))?;
        
        // Set non-blocking
        socket.set_nonblocking(true)
            .map_err(|e| TransportError::BindError(e.to_string()))?;
        
        // Multicast sends go to this address
        let multicast_addr = SocketAddr::new(IpAddr::V4(multicast_ip), port);
        
        info!("✓ UDP socket bound to {}", bind_addr);
        info!("✓ Joined multicast group {}", multicast_addr);
        
        // Create audio channel
        let (audio_tx, audio_rx) = mpsc::unbounded_channel();
        
        // Generate our keypair for encryption
        let mut crypto = CryptoEngine::new();
        let our_public = crypto.generate_keypair();
        
        Ok(Self {
            socket: Arc::new(socket),
            multicast_addr,
            bound_port: Arc::new(AtomicU16::new(port)),
            device_id,
            device_name,
            current_channel: Arc::new(RwLock::new(1)),
            config: Arc::new(RwLock::new(config)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            crypto_engines: Arc::new(RwLock::new(HashMap::new())),
            our_public_key: Arc::new(RwLock::new(Some(our_public))),
            running: Arc::new(AtomicBool::new(false)),
            audio_tx,
            audio_rx: Arc::new(RwLock::new(Some(audio_rx))),
        })
    }
    
    /// Find a random available port in the dynamic range
    fn find_random_port() -> Result<u16, TransportError> {
        let mut rng = rand::thread_rng();
        
        for _ in 0..100 {  // Try up to 100 times
            let port = rng.gen_range(PORT_RANGE_START..=PORT_RANGE_END);
            
            // Test if port is available
            if let Ok(test_socket) = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) {
                let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
                if test_socket.bind(&addr.into()).is_ok() {
                    // Port is available - socket will be dropped and port released
                    info!("Selected random port: {}", port);
                    return Ok(port);
                }
            }
        }
        
        Err(TransportError::NoPortAvailable)
    }
    
    /// Start transport (discovery + receive loop)
    pub async fn start(&self) -> Result<(), TransportError> {
        info!("Starting transport manager");
        
        self.running.store(true, Ordering::Relaxed);
        
        // Start discovery beacon with our public key
        let device_id = self.device_id;
        let device_name = self.device_name.clone();
        let socket = Arc::clone(&self.socket);
        let multicast_addr = self.multicast_addr;
        let channel = Arc::clone(&self.current_channel);
        let running = Arc::clone(&self.running);
        let our_public_key = Arc::clone(&self.our_public_key);
        let config = Arc::clone(&self.config);
        
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(super::BEACON_INTERVAL_SECS));
            
            while running.load(Ordering::Relaxed) {
                interval.tick().await;
                
                let current_channel = *channel.read().unwrap();
                let public_key = our_public_key.read().unwrap().clone();
                let encryption_enabled = config.read().unwrap().encryption_enabled;
                
                // Include our public key in discovery for key exchange
                let packet = if encryption_enabled {
                    Packet::discovery_with_key(device_id, device_name.clone(), current_channel, public_key)
                } else {
                    Packet::discovery(device_id, device_name.clone(), current_channel)
                };
                
                if let Ok(data) = packet.serialize() {
                    if let Err(e) = socket.send_to(&data, &multicast_addr.into()) {
                        error!("Failed to send discovery: {}", e);
                    }
                }
            }
        });
        
        // Start keepalive loop (sends between discovery beacons to maintain peer connections)
        let device_id_ka = self.device_id;
        let socket_ka = Arc::clone(&self.socket);
        let multicast_addr_ka = self.multicast_addr;
        let running_ka = Arc::clone(&self.running);
        let peers_ka = Arc::clone(&self.peers);
        
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(KEEPALIVE_INTERVAL_SECS));
            
            while running_ka.load(Ordering::Relaxed) {
                interval.tick().await;
                
                // Only send keepalive if we have active peers
                let has_peers = !peers_ka.read().unwrap().is_empty();
                if has_peers {
                    let packet = Packet::keep_alive(device_id_ka);
                    
                    if let Ok(data) = packet.serialize() {
                        if let Err(e) = socket_ka.send_to(&data, &multicast_addr_ka.into()) {
                            warn!("Failed to send keepalive: {}", e);
                        } else {
                            debug!("Sent keepalive to multicast group");
                        }
                    }
                }
            }
        });
        
        // Start receive loop
        let socket_rx = Arc::clone(&self.socket);
        let peers = Arc::clone(&self.peers);
        let crypto_engines = Arc::clone(&self.crypto_engines);
        let our_public_key = Arc::clone(&self.our_public_key);
        let audio_tx = self.audio_tx.clone();
        let running_rx = Arc::clone(&self.running);
        let device_id_rx = self.device_id;
        let config = Arc::clone(&self.config);
        
        tokio::spawn(async move {
            let mut buf = vec![std::mem::MaybeUninit::<u8>::uninit(); MAX_PACKET_SIZE];
            
            while running_rx.load(Ordering::Relaxed) {
                match socket_rx.recv_from(&mut buf) {
                    Ok((size, src_addr)) => {
                        // SAFETY: recv_from initializes the buffer up to `size` bytes
                        let received = unsafe {
                            std::slice::from_raw_parts(buf.as_ptr() as *const u8, size)
                        };
                        if let Ok(packet) = Packet::deserialize(received) {
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
                                        public_key: None,
                                        key_exchanged: false,
                                    };
                                    
                                    peers.write().unwrap().insert(packet.device_id, peer);
                                }
                                PacketType::DiscoveryWithKey { device_name, channel, public_key } => {
                                    // Handle discovery with public key
                                    let peer = PeerInfo {
                                        device_id: packet.device_id,
                                        device_name,
                                        address: src_addr.as_socket().unwrap(),
                                        last_seen: SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap()
                                            .as_secs(),
                                        channel,
                                        public_key: public_key.map(|k| hex::encode(k)),
                                        key_exchanged: false,
                                    };
                                    
                                    // Perform key exchange if we have their public key
                                    if let Some(peer_public) = peer.get_public_key_bytes() {
                                        let encryption_enabled = config.read().unwrap().encryption_enabled;
                                        
                                        if encryption_enabled {
                                            let mut engines = crypto_engines.write().unwrap();
                                            if !engines.contains_key(&packet.device_id) {
                                                let mut engine = CryptoEngine::new();
                                                let _ = engine.generate_keypair();
                                                if engine.key_exchange(&peer_public).is_ok() {
                                                    info!("Key exchange completed with peer {:08X}", packet.device_id);
                                                    engines.insert(packet.device_id, engine);
                                                    
                                                    // Mark peer as key exchanged
                                                    let mut peers_w = peers.write().unwrap();
                                                    if let Some(p) = peers_w.get_mut(&packet.device_id) {
                                                        p.key_exchanged = true;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    
                                    peers.write().unwrap().insert(packet.device_id, peer);
                                }
                                PacketType::Audio { channel, data } => {
                                    // Decrypt if encryption is enabled
                                    let encryption_enabled = config.read().unwrap().encryption_enabled;
                                    
                                    let decrypted_data = if encryption_enabled {
                                        if let Some(engine) = crypto_engines.read().unwrap().get(&packet.device_id) {
                                            if engine.is_ready() && data.len() > 28 {
                                                // Extract nonce and auth tag
                                                let nonce: [u8; 12] = data[..12].try_into().unwrap_or([0u8; 12]);
                                                let tag: [u8; 16] = data[12..28].try_into().unwrap_or([0u8; 16]);
                                                let ciphertext = &data[28..];
                                                
                                                match engine.decrypt(ciphertext, &nonce, &tag) {
                                                    Ok(decrypted) => decrypted,
                                                    Err(e) => {
                                                        warn!("Decryption failed from {:08X}: {:?}", packet.device_id, e);
                                                        continue;
                                                    }
                                                }
                                            } else {
                                                // No key exchange yet, skip encrypted packet
                                                debug!("Skipping encrypted packet - no key for {:08X}", packet.device_id);
                                                continue;
                                            }
                                        } else {
                                            debug!("No crypto engine for peer {:08X}", packet.device_id);
                                            continue;
                                        }
                                    } else {
                                        data.clone()
                                    };
                                    
                                    // Forward audio to audio channel
                                    let _ = audio_tx.send(decrypted_data);
                                }
                                PacketType::EncryptedAudio { channel, nonce, auth_tag, data } => {
                                    // Already-structured encrypted audio
                                    if let Some(engine) = crypto_engines.read().unwrap().get(&packet.device_id) {
                                        if engine.is_ready() {
                                            match engine.decrypt(&data, &nonce, &auth_tag) {
                                                Ok(decrypted) => {
                                                    let _ = audio_tx.send(decrypted);
                                                }
                                                Err(e) => {
                                                    warn!("Decryption failed: {:?}", e);
                                                }
                                            }
                                        }
                                    }
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
                                PacketType::KeyExchange { public_key } => {
                                    // Handle explicit key exchange request
                                    debug!("Received key exchange from {:08X}", packet.device_id);
                                    let encryption_enabled = config.read().unwrap().encryption_enabled;
                                    
                                    if encryption_enabled {
                                        let mut engines = crypto_engines.write().unwrap();
                                        if !engines.contains_key(&packet.device_id) {
                                            let mut engine = CryptoEngine::new();
                                            let _ = engine.generate_keypair();
                                            if engine.key_exchange(&public_key).is_ok() {
                                                info!("Key exchange completed with peer {:08X}", packet.device_id);
                                                engines.insert(packet.device_id, engine);
                                            }
                                        }
                                    }
                                }
                                PacketType::KeyExchangeResponse { public_key, success } => {
                                    // Handle key exchange response
                                    if success {
                                        debug!("Key exchange response (success) from {:08X}", packet.device_id);
                                        let encryption_enabled = config.read().unwrap().encryption_enabled;
                                        
                                        if encryption_enabled {
                                            let mut engines = crypto_engines.write().unwrap();
                                            if !engines.contains_key(&packet.device_id) {
                                                let mut engine = CryptoEngine::new();
                                                let _ = engine.generate_keypair();
                                                if engine.key_exchange(&public_key).is_ok() {
                                                    engines.insert(packet.device_id, engine);
                                                }
                                            }
                                        }
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
    
    /// Send audio data (with encryption if enabled)
    pub fn send_audio(&self, audio_data: &[u8]) -> Result<(), TransportError> {
        let channel = *self.current_channel.read().unwrap();
        let config = self.config.read().unwrap();
        
        let packet = if config.encryption_enabled {
            // Encrypt audio for all known peers
            let nonce = CryptoEngine::generate_nonce();
            
            // For multicast, we use a shared session key derived from our device ID
            // In practice, we encrypt once and broadcast (all peers with our key can decrypt)
            let engines = self.crypto_engines.read().unwrap();
            
            if let Some((_, engine)) = engines.iter().next() {
                if engine.is_ready() {
                    match engine.encrypt(audio_data, &nonce) {
                        Ok((ciphertext, auth_tag)) => {
                            // Pack: nonce (12) + auth_tag (16) + ciphertext
                            let mut encrypted_payload = Vec::with_capacity(28 + ciphertext.len());
                            encrypted_payload.extend_from_slice(&nonce);
                            encrypted_payload.extend_from_slice(&auth_tag);
                            encrypted_payload.extend_from_slice(&ciphertext);
                            
                            Packet::audio(self.device_id, channel, encrypted_payload)
                        }
                        Err(e) => {
                            warn!("Encryption failed, sending unencrypted: {:?}", e);
                            Packet::audio(self.device_id, channel, audio_data.to_vec())
                        }
                    }
                } else {
                    // No encryption ready, send plain
                    Packet::audio(self.device_id, channel, audio_data.to_vec())
                }
            } else {
                // No peers with keys yet
                Packet::audio(self.device_id, channel, audio_data.to_vec())
            }
        } else {
            Packet::audio(self.device_id, channel, audio_data.to_vec())
        };
        
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
        use crate::constants::{MIN_CHANNEL, MAX_CHANNEL};
        let clamped = channel.clamp(MIN_CHANNEL, MAX_CHANNEL);
        *self.current_channel.write().unwrap() = clamped;
        info!("Channel changed to {}", clamped);
    }
    
    /// Get current channel
    pub fn get_channel(&self) -> u8 {
        *self.current_channel.read().unwrap()
    }
    
    /// Get bound port
    pub fn get_port(&self) -> u16 {
        self.bound_port.load(Ordering::Relaxed)
    }
    
    /// Update configuration
    pub fn update_config(&self, new_config: TransportConfig) {
        *self.config.write().unwrap() = new_config;
    }
    
    /// Get current configuration
    pub fn get_config(&self) -> TransportConfig {
        self.config.read().unwrap().clone()
    }
    
    /// Check if running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
    
    /// Check if encryption is active with any peer
    pub fn is_encrypted(&self) -> bool {
        let engines = self.crypto_engines.read().unwrap();
        engines.values().any(|e| e.is_ready())
    }
    
    /// Get our public key (hex encoded)
    pub fn get_public_key(&self) -> Option<String> {
        self.our_public_key.read().unwrap().map(|k| hex::encode(k))
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
        // Should be clamped to MAX_CHANNEL (16)
        assert_eq!(transport.get_channel(), 16);
        
        transport.set_channel(8);
        assert_eq!(transport.get_channel(), 8);
    }
    
    #[tokio::test]
    async fn test_random_port() {
        let config = TransportConfig {
            use_random_port: true,
            ..Default::default()
        };
        
        let transport = TransportManager::with_config(0x12345678, "Test".to_string(), config).unwrap();
        let port = transport.get_port();
        
        assert!(port >= PORT_RANGE_START);
        assert!(port <= PORT_RANGE_END);
    }
    
    #[tokio::test]
    async fn test_encryption_disabled() {
        let config = TransportConfig {
            encryption_enabled: false,
            ..Default::default()
        };
        
        let transport = TransportManager::with_config(0x12345678, "Test".to_string(), config).unwrap();
        assert!(!transport.is_encrypted());
    }
}
