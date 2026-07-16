// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-A6XSZ4VDBSXO
/// Transport Manager - UDP Multicast Communication with Encryption
/// 
/// Features:
/// - Random port selection per session
/// - X25519 key exchange with peers
/// - AES-256-GCM encrypted audio packets
/// - Automatic peer discovery
/// 
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use super::{TransportError, MAX_PACKET_SIZE, PEER_TIMEOUT_SECS, AudioFrame};
use super::liveness::LivenessTracker;
use super::control;
use crate::constants::{DEFAULT_MULTICAST_ADDR, DEFAULT_MULTICAST_PORT, PORT_RANGE_START, PORT_RANGE_END, KEEPALIVE_INTERVAL_SECS};
use crate::protocol::{Packet, PacketType};
use crate::security::CryptoEngine;
// Shared cross-platform crypto + audio wire frame. The desktop audio plane now
// matches Android/iOS: the WHOLE core::wire frame is sealed with the QR-PSK
// CryptoSession (AES-256-GCM, nonce||ct||tag) and multicast on the shared group,
// instead of the legacy per-peer X25519 ECDH + bincode Packet path (kept only as
// a desktop↔desktop fallback when no QR session is installed).
use sassytalkie_core::crypto::CryptoSession;
use sassytalkie_core::wire;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
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
            // MUST be the shared fixed port (5555) to interoperate with the
            // Android/iOS phones — UDP multicast delivery is port-specific, so a
            // random port silently isolates the desktop from the phone group.
            // App-layer AES-256-GCM is the security boundary, not port obscurity.
            use_random_port: false,
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
            .unwrap_or_default()
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
    
    // Encryption engines per peer (legacy desktop↔desktop X25519 ECDH path).
    crypto_engines: Arc<RwLock<HashMap<u32, CryptoEngine>>>,

    // Shared GROUP session installed from the QR PSK (core::crypto). When present
    // this is the canonical audio cipher: the whole core::wire frame is sealed
    // with it and multicast to the group, byte-identical to Android/iOS. `None`
    // until a QR session is imported (then the legacy per-peer path is bypassed).
    group_crypto: Arc<RwLock<Option<CryptoSession>>>,

    // Stable per-install sender id placed in every core::wire frame (<= 32 bytes).
    sender_id: String,
    
    // Our public key for sharing
    our_public_key: Arc<RwLock<Option<[u8; 32]>>>,

    // Our session identity engine — holds the static secret behind our_public_key.
    // Reused to derive EVERY per-peer session key (derive_peer_session), so both
    // sides agree on the shared key. Immutable for the session (no lock needed).
    identity: Arc<CryptoEngine>,

    // Control
    running: Arc<AtomicBool>,

    // Channels for audio data
    audio_tx: mpsc::UnboundedSender<AudioFrame>,
    audio_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<AudioFrame>>>>,

    // Liveness tracking
    liveness: Arc<Mutex<LivenessTracker>>,

    // Session epoch (generated once at start, used in heartbeats)
    session_epoch: u64,

    // Heartbeat sequence counter
    heartbeat_seq: Arc<AtomicU32>,
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
        
        // Determine port and create bound socket.
        // For random ports, bind_random_udp() atomically selects and binds to avoid
        // the TOCTOU race that existed in the old probe-drop-rebind pattern.
        let (socket, port) = if config.use_random_port {
            Self::bind_random_udp()?
        } else {
            let port = config.fixed_port;
            let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
                .map_err(|e| TransportError::BindError(e.to_string()))?;
            sock.set_reuse_address(true)
                .map_err(|e| TransportError::BindError(e.to_string()))?;
            #[cfg(not(target_os = "windows"))]
            sock.set_reuse_port(true)
                .map_err(|e| TransportError::BindError(e.to_string()))?;
            let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
            sock.bind(&bind_addr.into())
                .map_err(|e| TransportError::BindError(format!("Port {}: {}", port, e)))?;
            (sock, port)
        };

        info!("Using port: {}", port);
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        
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
        
        // Generate our session keypair for encryption. We KEEP this engine (as
        // `identity`) — it holds the static secret behind `our_public` and is reused
        // to derive every per-peer session key.
        let mut crypto = CryptoEngine::new();
        let our_public = crypto.generate_keypair();

        // Generate session epoch once for this run
        let session_epoch = control::new_session_epoch();

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
            group_crypto: Arc::new(RwLock::new(None)),
            sender_id: format!("desk-{:08x}", device_id),
            our_public_key: Arc::new(RwLock::new(Some(our_public))),
            identity: Arc::new(crypto),
            running: Arc::new(AtomicBool::new(false)),
            audio_tx,
            audio_rx: Arc::new(RwLock::new(Some(audio_rx))),
            liveness: Arc::new(Mutex::new(LivenessTracker::new())),
            session_epoch,
            heartbeat_seq: Arc::new(AtomicU32::new(0)),
        })
    }
    
    /// Bind a UDP socket to a random available port in the dynamic range.
    ///
    /// SECURITY FIX (CRITICAL-4): Previously this function used a probe-drop-rebind
    /// pattern (bind a test socket, drop it to release the port, return the port number,
    /// then bind the real socket). That pattern has a TOCTOU race: another process can
    /// claim the port between the drop and the real bind. The fix binds the real socket
    /// directly in the loop and returns it, eliminating the race entirely.
    fn bind_random_udp() -> Result<(Socket, u16), TransportError> {
        let mut rng = rand::thread_rng();

        for _ in 0..100 {
            let port = rng.gen_range(PORT_RANGE_START..=PORT_RANGE_END);
            let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);

            match Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) {
                Ok(sock) => {
                    // Allow multiple processes to bind to same port (set before bind)
                    let _ = sock.set_reuse_address(true);
                    #[cfg(not(target_os = "windows"))]
                    let _ = sock.set_reuse_port(true);

                    match sock.bind(&addr.into()) {
                        Ok(()) => {
                            info!("Selected random port: {}", port);
                            return Ok((sock, port));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
                        Err(e) => {
                            return Err(TransportError::BindError(
                                format!("Port {}: {}", port, e),
                            ));
                        }
                    }
                }
                Err(e) => return Err(TransportError::BindError(e.to_string())),
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
        
        // Start heartbeat emit loop (every 2 seconds)
        let socket_hb = Arc::clone(&self.socket);
        let multicast_addr_hb = self.multicast_addr;
        let running_hb = Arc::clone(&self.running);
        let liveness_hb = Arc::clone(&self.liveness);
        let hb_seq = Arc::clone(&self.heartbeat_seq);
        let hb_epoch = self.session_epoch;
        let device_id_hb = self.device_id;

        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(2));
            let peer_key = format!("{:08X}", device_id_hb);

            while running_hb.load(Ordering::Relaxed) {
                interval.tick().await;

                let seq = hb_seq.fetch_add(1, Ordering::Relaxed);
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                // Record that we sent this heartbeat (for RTT calculation when echoed back)
                liveness_hb.lock().unwrap()
                    .on_heartbeat_sent(&peer_key, hb_epoch, seq, now_ms);

                let frame = control::encode_heartbeat(
                    hb_epoch,
                    seq,
                    now_ms,
                    control::PresenceState::Idle,
                    0,
                );

                if let Err(e) = socket_hb.send_to(&frame, &multicast_addr_hb.into()) {
                    warn!("Failed to send heartbeat: {}", e);
                } else {
                    debug!("Sent heartbeat seq={} epoch={:#x}", seq, hb_epoch);
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
        let liveness_rx = Arc::clone(&self.liveness);
        let identity_rx = Arc::clone(&self.identity);
        let group_crypto_rx = Arc::clone(&self.group_crypto);
        let sender_id_rx = self.sender_id.clone();

        // Dedicated OS thread, NOT a tokio task: recv_from on the socket2 socket
        // is a blocking syscall (the socket is non-blocking, so it returns
        // WouldBlock and we poll). Running that poll on a tokio worker monopolized
        // a runtime thread and forced a busy-sleep. On its own thread it can block
        // freely without starving the async runtime. Everything it touches
        // (RwLocks, the unbounded mpsc sender) is sync, so no runtime is needed.
        std::thread::Builder::new()
            .name("transport-rx".into())
            .spawn(move || {
            let mut buf = vec![std::mem::MaybeUninit::<u8>::uninit(); MAX_PACKET_SIZE];

            while running_rx.load(Ordering::Relaxed) {
                match socket_rx.recv_from(&mut buf) {
                    Ok((size, src_addr)) => {
                        // SAFETY: recv_from initializes the buffer up to `size` bytes
                        let received = unsafe {
                            std::slice::from_raw_parts(buf.as_ptr() as *const u8, size)
                        };

                        // ── Cross-platform GROUP audio plane (Android/iOS) ──
                        // With a QR session PSK installed, a datagram that decrypts
                        // under it (valid GCM tag) IS a shared core::wire frame.
                        // Try this FIRST: sealed audio has a random first byte and
                        // would otherwise be misclassified as a control frame. The
                        // 128-bit GCM tag makes false positives (a legacy/control
                        // frame "decrypting") astronomically unlikely.
                        let group_plain = {
                            let guard = group_crypto_rx.read().unwrap();
                            match guard.as_ref() {
                                Some(crypto) => crypto.decrypt(received).ok(),
                                None => None,
                            }
                        };
                        if let Some(plain) = group_plain {
                            if let Ok((_ch, _sub, sender, _name, ts, opus)) =
                                wire::unpack_wire_frame(&plain)
                            {
                                // Skip our own multicast loopback.
                                if sender != sender_id_rx {
                                    let _ = audio_tx.send(AudioFrame {
                                        sender,
                                        timestamp: ts,
                                        opus,
                                    });
                                }
                            }
                            continue;
                        }

                        // Control frames have first byte >= 0x10; handle before Packet deserialize
                        if received.first().copied().unwrap_or(0) >= 0x10 {
                            if let Some(decoded) = control::decode(received) {
                                if decoded.opcode == control::OP_HEARTBEAT {
                                    if let Some(hb) = control::parse_heartbeat(&decoded.payload) {
                                        let now_ms = SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis() as u64;
                                        let peer_key = src_addr
                                            .as_socket()
                                            .map(|a| a.to_string())
                                            .unwrap_or_else(|| "unknown".to_string());
                                        liveness_rx.lock().unwrap().on_heartbeat(
                                            &peer_key,
                                            hb.epoch,
                                            hb.seq,
                                            hb.ts_ms,
                                            now_ms,
                                            hb.state.as_byte(),
                                        );
                                        debug!("Received heartbeat from {} seq={} epoch={:#x}",
                                               peer_key, hb.seq, hb.epoch);
                                    }
                                }
                            }
                            continue;
                        }

                        if let Ok(packet) = Packet::deserialize(received) {
                            // Ignore own packets
                            if packet.device_id == device_id_rx {
                                continue;
                            }
                            
                            match packet.packet_type {
                                PacketType::Discovery { device_name, channel } => {
                                    // Skip (don't panic) if the source isn't an IP
                                    // socket — recv_from on our UDP socket always
                                    // yields one, but as_socket() returns Option.
                                    let src = match src_addr.as_socket() {
                                        Some(a) => a,
                                        None => continue,
                                    };
                                    // Update peer info
                                    let peer = PeerInfo {
                                        device_id: packet.device_id,
                                        device_name,
                                        address: src,
                                        last_seen: SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs(),
                                        channel,
                                        public_key: None,
                                        key_exchanged: false,
                                    };
                                    
                                    peers.write().unwrap().insert(packet.device_id, peer);
                                }
                                PacketType::DiscoveryWithKey { device_name, channel, public_key } => {
                                    let src = match src_addr.as_socket() {
                                        Some(a) => a,
                                        None => continue,
                                    };
                                    // Handle discovery with public key
                                    let mut peer = PeerInfo {
                                        device_id: packet.device_id,
                                        device_name,
                                        address: src,
                                        last_seen: SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs(),
                                        channel,
                                        public_key: public_key.map(|k| hex::encode(k)),
                                        key_exchanged: false,
                                    };
                                    
                                    // Perform key exchange if we have their public key
                                    if let Some(peer_public) = peer.get_public_key_bytes() {
                                        let encryption_enabled = config.read().unwrap().encryption_enabled;
                                        
                                        if encryption_enabled {
                                            // Lock ordering: always acquire crypto_engines BEFORE peers.
                                            // Complete all crypto_engines mutations first, then drop the
                                            // guard, and only then acquire peers — this prevents a
                                            // lock-ordering inversion if any future path ever holds
                                            // peers while trying to acquire crypto_engines.
                                            let key_exchange_succeeded = {
                                                let mut engines = crypto_engines.write().unwrap();
                                                if !engines.contains_key(&packet.device_id) {
                                                    // Derive the per-peer key from OUR advertised identity
                                                    // secret + the peer's public key (symmetric ECDH).
                                                    match identity_rx.derive_peer_session(&peer_public) {
                                                        Ok(engine) => {
                                                            info!("Key exchange completed with peer {:08X}", packet.device_id);
                                                            engines.insert(packet.device_id, engine);
                                                            true
                                                        }
                                                        Err(e) => {
                                                            warn!("Key derivation failed for peer {:08X}: {:?}", packet.device_id, e);
                                                            false
                                                        }
                                                    }
                                                } else {
                                                    false
                                                }
                                                // `engines` (crypto_engines write guard) is dropped here
                                            };

                                            // Mark the peer as encrypted iff we now hold a ready
                                            // engine for it. The OLD code did get_mut() BEFORE the
                                            // peer was inserted below (always a no-op on first
                                            // contact), and the unconditional insert at the end of
                                            // this arm then overwrote key_exchanged with false — so
                                            // it was never actually true. Set it on the `peer` we're
                                            // about to insert instead. (engines write guard already
                                            // dropped above, so this read can't self-deadlock.)
                                            peer.key_exchanged = key_exchange_succeeded
                                                || crypto_engines.read().unwrap().contains_key(&packet.device_id);
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
                                    
                                    // Forward audio to audio channel (legacy
                                    // per-peer path — no wire timestamp).
                                    let _ = audio_tx.send(AudioFrame {
                                        sender: format!("legacy-{:08X}", packet.device_id),
                                        timestamp: 0,
                                        opus: decrypted_data,
                                    });
                                }
                                PacketType::EncryptedAudio { channel, nonce, auth_tag, data } => {
                                    // Already-structured encrypted audio
                                    if let Some(engine) = crypto_engines.read().unwrap().get(&packet.device_id) {
                                        if engine.is_ready() {
                                            match engine.decrypt(&data, &nonce, &auth_tag) {
                                                Ok(decrypted) => {
                                                    let _ = audio_tx.send(AudioFrame {
                                                        sender: format!("legacy-{:08X}", packet.device_id),
                                                        timestamp: 0,
                                                        opus: decrypted,
                                                    });
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
                                            .unwrap_or_default()
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
                                            match identity_rx.derive_peer_session(&public_key) {
                                                Ok(engine) => {
                                                    info!("Key exchange completed with peer {:08X}", packet.device_id);
                                                    engines.insert(packet.device_id, engine);
                                                }
                                                Err(e) => warn!("Key derivation failed for peer {:08X}: {:?}", packet.device_id, e),
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
                                                if let Ok(engine) = identity_rx.derive_peer_session(&public_key) {
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
                        // No data available — blocking sleep on this dedicated
                        // thread (does not occupy a tokio worker). 2ms keeps
                        // inbound latency low without spinning the CPU.
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => {
                        error!("Receive error: {}", e);
                    }
                }
            }
        })
        .map_err(|e| TransportError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        info!("✓ Transport manager started");
        Ok(())
    }
    
    /// Stop transport
    pub fn stop(&self) {
        info!("Stopping transport manager");
        self.running.store(false, Ordering::Relaxed);
    }

    /// Install the shared GROUP session key (the QR PSK), switching the audio
    /// plane to the cross-platform `core::wire` + `core::crypto` path. After this,
    /// `send_audio` seals whole wire frames with this key and multicasts them, and
    /// the RX loop decrypts inbound group frames — byte-identical to Android/iOS.
    pub fn set_session_psk(&self, key: &[u8; 32]) {
        *self.group_crypto.write().unwrap() = Some(CryptoSession::from_psk(key));
        info!("Transport: group session installed (core::wire audio plane active)");
    }

    /// Send audio data (with encryption if enabled)
    pub fn send_audio(&self, audio_data: &[u8]) -> Result<(), TransportError> {
        let channel = *self.current_channel.read().unwrap();

        // ── Cross-platform GROUP path (matches Android/iOS) ──
        // With a QR session PSK installed, seal the WHOLE core::wire frame with the
        // group CryptoSession and multicast ONE copy to the group — byte-identical
        // to the Android/iOS multicast audio plane. Bypasses the legacy per-peer
        // X25519 path entirely.
        let group_sealed = {
            let mut guard = self.group_crypto.write().unwrap();
            match guard.as_mut() {
                Some(crypto) => {
                    let frame = wire::pack_wire_frame(
                        channel,
                        wire::SUBCH_MAIN,
                        &self.sender_id,
                        &self.device_name,
                        wire::now_ms(),
                        audio_data,
                    );
                    Some(crypto.encrypt(&frame).map_err(TransportError::EncryptionError)?)
                }
                None => None,
            }
        };
        if let Some(sealed) = group_sealed {
            self.socket
                .send_to(&sealed, &self.multicast_addr.into())
                .map_err(TransportError::IoError)?;
            return Ok(());
        }

        let encryption_enabled = self.config.read().unwrap().encryption_enabled;

        if !encryption_enabled {
            // Plaintext mode keeps the single multicast send.
            let packet = Packet::audio(self.device_id, channel, audio_data.to_vec());
            let data = packet.serialize().map_err(TransportError::SerializationError)?;
            self.socket.send_to(&data, &self.multicast_addr.into())
                .map_err(TransportError::IoError)?;
            return Ok(());
        }

        // Per-peer unicast (fixes CRITICAL-3): each peer derived a DIFFERENT
        // pairwise session key, so a single multicast packet encrypted under one
        // peer's key was undecryptable by everyone else. We now encrypt one copy
        // per ready peer with THAT peer's engine + a unique counter nonce and
        // unicast it to the peer's address. Side benefit: a compromised peer's
        // key no longer exposes the whole room's audio.
        let engines = self.crypto_engines.read().unwrap();
        let peers = self.peers.read().unwrap();

        let mut sent_any = false;
        let mut last_err: Option<TransportError> = None;
        let mut had_ready_peer = false;

        for (peer_id, engine) in engines.iter() {
            if !engine.is_ready() {
                continue;
            }
            // Need the peer's address to unicast. An engine without a known
            // address (key exchanged but no discovery address yet) is skipped.
            let addr = match peers.get(peer_id) {
                Some(p) => p.address,
                None => continue,
            };
            had_ready_peer = true;

            // Unique (key, nonce) per frame for THIS engine — never random-reuse.
            let nonce = match engine.next_nonce() {
                Ok(n) => n,
                Err(_) => {
                    last_err = Some(TransportError::EncryptionError(
                        "nonce counter exhausted for peer".into(),
                    ));
                    continue;
                }
            };
            let (ciphertext, auth_tag) = match engine.encrypt(audio_data, &nonce) {
                Ok(v) => v,
                Err(_) => {
                    last_err = Some(TransportError::EncryptionError("encrypt failed".into()));
                    continue;
                }
            };

            // Pack: nonce (12) + auth_tag (16) + ciphertext
            let mut payload = Vec::with_capacity(28 + ciphertext.len());
            payload.extend_from_slice(&nonce);
            payload.extend_from_slice(&auth_tag);
            payload.extend_from_slice(&ciphertext);

            let packet = Packet::audio(self.device_id, channel, payload);
            let data = match packet.serialize() {
                Ok(d) => d,
                Err(e) => {
                    last_err = Some(TransportError::SerializationError(e));
                    continue;
                }
            };

            match self.socket.send_to(&data, &addr.into()) {
                Ok(_) => sent_any = true,
                Err(e) => last_err = Some(TransportError::IoError(e)),
            }
        }

        if sent_any {
            Ok(())
        } else if !had_ready_peer {
            // Encryption is ENABLED but no peer has a completed key exchange yet.
            // Do NOT fall back to plaintext — drop the frame and signal the caller.
            warn!("send_audio: encryption enabled but no ready peer — refusing plaintext fallback");
            Err(TransportError::EncryptionError(
                "no completed key exchange yet — not sending plaintext".into(),
            ))
        } else {
            // Had ready peers but every send/encrypt failed — surface the last error.
            Err(last_err.unwrap_or_else(|| {
                TransportError::EncryptionError("encrypted send failed for all peers".into())
            }))
        }
    }
    
    /// Get received audio receiver
    pub fn take_audio_receiver(&self) -> Option<mpsc::UnboundedReceiver<AudioFrame>> {
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

    /// Get connection quality for each known peer.
    /// Returns a vec of (peer_id_hex, health_str, rtt_ms_option, transport_str).
    pub fn get_connection_quality(&self) -> Vec<(String, String, Option<u32>, String)> {
        let peers = self.peers.read().unwrap();
        let liveness = self.liveness.lock().unwrap();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        peers.values()
            .filter(|p| p.is_active())
            .map(|p| {
                // Liveness is keyed by socket address string
                let addr_key = p.address.to_string();
                let health = match liveness.health(&addr_key, now_ms) {
                    super::liveness::PeerHealth::Healthy  => "healthy",
                    super::liveness::PeerHealth::Degraded => "degraded",
                    super::liveness::PeerHealth::Stale    => "stale",
                };
                let rtt = liveness.rtt_ms(&addr_key);
                let peer_id_hex = format!("{:08X}", p.device_id);
                // All peers on desktop are reached via UDP multicast
                let transport = "UDP".to_string();
                (peer_id_hex, health.to_string(), rtt, transport)
            })
            .collect()
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
