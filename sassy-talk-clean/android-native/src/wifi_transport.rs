/// WiFi Multicast Transport Module
///
/// UDP multicast transport for audio data when both peers are on WiFi.
/// Uses socket2 for multicast group management on Android.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;
use log::info;
use socket2::{Domain, Protocol, Socket, Type, SockAddr};

/// Multicast group address for SassyTalkie discovery + audio
/// Unified across all platforms (Android, iOS, Desktop)
const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 42);
const MULTICAST_PORT: u16 = 5555;
const DISCOVERY_PORT: u16 = 5556;

/// Max UDP payload (safe for most networks without fragmentation)
const MAX_PACKET_SIZE: usize = 1400;

/// WiFi transport state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WifiState {
    Inactive,
    Discovering,
    Active,
    Error,
}

/// Discovery message types
#[repr(u8)]
enum DiscoveryMsgType {
    Announce = 0x01,
    Response = 0x02,
    Goodbye = 0x03,
}

/// Peer discovered via WiFi multicast
#[derive(Debug, Clone)]
pub struct WifiPeer {
    pub address: Ipv4Addr,
    pub device_name: String,
    pub channel: u8,
}

/// WiFi multicast transport
pub struct WifiTransport {
    audio_socket: Option<UdpSocket>,
    discovery_socket: Option<UdpSocket>,
    state: WifiState,
    local_name: String,
    peers: Vec<WifiPeer>,
}

impl WifiTransport {
    pub fn new(device_name: &str) -> Self {
        Self {
            audio_socket: None,
            discovery_socket: None,
            state: WifiState::Inactive,
            local_name: device_name.to_string(),
            peers: Vec::new(),
        }
    }

    /// Initialize multicast sockets
    pub fn init(&mut self) -> Result<(), String> {
        info!("WiFi transport: initializing multicast sockets");

        // Audio socket - join multicast group
        let audio_sock = match Self::create_multicast_socket(MULTICAST_PORT) {
            Ok(s) => s,
            Err(e) => {
                self.state = WifiState::Error;
                return Err(e);
            }
        };
        if let Err(e) = audio_sock.set_read_timeout(Some(Duration::from_millis(10))) {
            self.state = WifiState::Error;
            return Err(format!("Failed to set read timeout: {}", e));
        }
        self.audio_socket = Some(audio_sock);

        // Discovery socket
        let disc_sock = match Self::create_multicast_socket(DISCOVERY_PORT) {
            Ok(s) => s,
            Err(e) => {
                self.state = WifiState::Error;
                return Err(e);
            }
        };
        if let Err(e) = disc_sock.set_read_timeout(Some(Duration::from_millis(100))) {
            self.state = WifiState::Error;
            return Err(format!("Failed to set discovery timeout: {}", e));
        }
        self.discovery_socket = Some(disc_sock);

        self.state = WifiState::Discovering;
        info!("WiFi transport: initialized on {}:{}", MULTICAST_GROUP, MULTICAST_PORT);
        Ok(())
    }

    /// Create a UDP socket joined to the multicast group
    fn create_multicast_socket(port: u16) -> Result<UdpSocket, String> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| format!("Failed to create socket: {}", e))?;

        socket.set_reuse_address(true)
            .map_err(|e| format!("Failed to set reuse: {}", e))?;

        // Bind to any interface on the port
        let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket.bind(&SockAddr::from(addr))
            .map_err(|e| format!("Failed to bind to port {}: {}", port, e))?;

        // Join multicast group on all interfaces
        socket.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| format!("Failed to join multicast: {}", e))?;

        // Enable multicast loopback for testing on same device
        socket.set_multicast_loop_v4(false)
            .map_err(|e| format!("Failed to set multicast loop: {}", e))?;

        // Set TTL for local network
        socket.set_multicast_ttl_v4(1)
            .map_err(|e| format!("Failed to set multicast TTL: {}", e))?;

        socket.set_nonblocking(false)
            .map_err(|e| format!("Failed to set blocking: {}", e))?;

        Ok(socket.into())
    }

    /// Send audio data via multicast
    pub fn send_audio(&self, data: &[u8]) -> Result<usize, String> {
        let socket = self.audio_socket.as_ref()
            .ok_or("Audio socket not initialized")?;

        if data.len() > MAX_PACKET_SIZE {
            return Err(format!("Packet too large: {} > {}", data.len(), MAX_PACKET_SIZE));
        }

        let dest = SocketAddrV4::new(MULTICAST_GROUP, MULTICAST_PORT);
        socket.send_to(data, dest)
            .map_err(|e| format!("Failed to send: {}", e))
    }

    /// Receive audio data from multicast
    pub fn receive_audio(&self, buffer: &mut [u8]) -> Result<usize, String> {
        let socket = self.audio_socket.as_ref()
            .ok_or("Audio socket not initialized")?;

        match socket.recv_from(buffer) {
            Ok((size, _addr)) => Ok(size),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
                {
                    Ok(0)
                } else {
                    Err(format!("Failed to receive: {}", e))
                }
            }
        }
    }

    /// Send discovery announcement
    pub fn announce(&self, channel: u8) -> Result<(), String> {
        let socket = self.discovery_socket.as_ref()
            .ok_or("Discovery socket not initialized")?;

        // Packet format: [type:1][channel:1][name_len:1][name:N]
        let name_bytes = self.local_name.as_bytes();
        let mut packet = Vec::with_capacity(3 + name_bytes.len());
        packet.push(DiscoveryMsgType::Announce as u8);
        packet.push(channel);
        packet.push(name_bytes.len() as u8);
        packet.extend_from_slice(name_bytes);

        let dest = SocketAddrV4::new(MULTICAST_GROUP, DISCOVERY_PORT);
        socket.send_to(&packet, dest)
            .map_err(|e| format!("Failed to announce: {}", e))?;

        Ok(())
    }

    /// Check for discovery messages, returns newly found peers
    pub fn check_discovery(&mut self) -> Vec<WifiPeer> {
        let socket = match self.discovery_socket.as_ref() {
            Some(s) => s,
            None => return Vec::new(),
        };

        let mut buffer = [0u8; 256];
        let mut new_peers = Vec::new();

        // Read all pending discovery messages
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((size, addr)) => {
                    if size < 3 { continue; }

                    let msg_type = buffer[0];
                    let channel = buffer[1];
                    let name_len = buffer[2] as usize;

                    if size < 3 + name_len { continue; }

                    let name = String::from_utf8_lossy(&buffer[3..3 + name_len]).to_string();

                    if msg_type == DiscoveryMsgType::Announce as u8
                        || msg_type == DiscoveryMsgType::Response as u8
                    {
                        if let std::net::SocketAddr::V4(v4) = addr {
                            let peer_ip = *v4.ip();

                            // Don't add duplicates
                            if !self.peers.iter().any(|p| p.address == peer_ip) {
                                let peer = WifiPeer {
                                    address: peer_ip,
                                    device_name: name,
                                    channel,
                                };
                                info!("WiFi: discovered peer {} at {}", peer.device_name, peer.address);
                                new_peers.push(peer.clone());
                                self.peers.push(peer);
                            }
                        }
                    }
                }
                Err(_) => break, // No more messages
            }
        }

        new_peers
    }

    /// Send goodbye message
    pub fn goodbye(&self) {
        if let Some(socket) = &self.discovery_socket {
            let name_bytes = self.local_name.as_bytes();
            let mut packet = Vec::with_capacity(3 + name_bytes.len());
            packet.push(DiscoveryMsgType::Goodbye as u8);
            packet.push(0);
            packet.push(name_bytes.len() as u8);
            packet.extend_from_slice(name_bytes);

            let dest = SocketAddrV4::new(MULTICAST_GROUP, DISCOVERY_PORT);
            let _ = socket.send_to(&packet, dest);
        }
    }

    /// Get discovered peers
    pub fn get_peers(&self) -> &[WifiPeer] {
        &self.peers
    }

    /// Check if WiFi transport has active peers
    pub fn has_peers(&self) -> bool {
        !self.peers.is_empty()
    }

    /// Get current state
    pub fn get_state(&self) -> WifiState {
        self.state
    }

    /// Activate transport (peer confirmed, ready for audio)
    pub fn activate(&mut self) {
        self.state = WifiState::Active;
        info!("WiFi transport: activated");
    }

    /// Shutdown transport
    pub fn shutdown(&mut self) {
        self.goodbye();
        self.state = WifiState::Inactive;
        self.audio_socket = None;
        self.discovery_socket = None;
        self.peers.clear();
        info!("WiFi transport: shut down");
    }
}

impl Drop for WifiTransport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wifi_transport_creation() {
        let transport = WifiTransport::new("TestDevice");
        assert_eq!(transport.get_state(), WifiState::Inactive);
        assert!(!transport.has_peers());
    }
}
