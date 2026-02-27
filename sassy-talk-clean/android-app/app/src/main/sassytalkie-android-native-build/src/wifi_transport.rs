#![allow(dead_code)]
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;
use socket2::{Domain, Protocol, Socket, Type, SockAddr};

const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 77, 83, 84);
const MULTICAST_PORT: u16 = 5354;
const DISCOVERY_PORT: u16 = 5355;
const MAX_PACKET_SIZE: usize = 1400;

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum WifiState { Inactive, Discovering, Active, Error }

#[repr(u8)]
enum DiscoveryMsgType { Announce = 0x01, Response = 0x02, Goodbye = 0x03 }

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WifiPeer { pub address: Ipv4Addr, pub device_name: String, pub channel: u8 }

pub struct WifiTransport {
    audio_socket: Option<UdpSocket>, discovery_socket: Option<UdpSocket>,
    state: WifiState, local_name: String, peers: Vec<WifiPeer>,
}

impl WifiTransport {
    pub fn new(device_name: &str) -> Self {
        Self { audio_socket: None, discovery_socket: None, state: WifiState::Inactive, local_name: device_name.to_string(), peers: Vec::new() }
    }
    pub fn init(&mut self) -> Result<(), String> {
        let audio_sock = Self::create_multicast_socket(MULTICAST_PORT)?;
        audio_sock.set_read_timeout(Some(Duration::from_millis(10))).map_err(|e| format!("{}", e))?;
        self.audio_socket = Some(audio_sock);
        let disc_sock = Self::create_multicast_socket(DISCOVERY_PORT)?;
        disc_sock.set_read_timeout(Some(Duration::from_millis(100))).map_err(|e| format!("{}", e))?;
        self.discovery_socket = Some(disc_sock);
        self.state = WifiState::Discovering;
        Ok(())
    }
    fn create_multicast_socket(port: u16) -> Result<UdpSocket, String> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|e| format!("{}", e))?;
        socket.set_reuse_address(true).map_err(|e| format!("{}", e))?;
        let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket.bind(&SockAddr::from(addr)).map_err(|e| format!("{}", e))?;
        socket.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED).map_err(|e| format!("{}", e))?;
        socket.set_multicast_loop_v4(false).map_err(|e| format!("{}", e))?;
        socket.set_multicast_ttl_v4(1).map_err(|e| format!("{}", e))?;
        socket.set_nonblocking(false).map_err(|e| format!("{}", e))?;
        Ok(socket.into())
    }
    pub fn send_audio(&self, data: &[u8]) -> Result<usize, String> {
        let s = self.audio_socket.as_ref().ok_or("No audio socket")?;
        if data.len() > MAX_PACKET_SIZE { return Err("Packet too large".into()); }
        s.send_to(data, SocketAddrV4::new(MULTICAST_GROUP, MULTICAST_PORT)).map_err(|e| format!("{}", e))
    }
    pub fn receive_audio(&self, buffer: &mut [u8]) -> Result<usize, String> {
        let s = self.audio_socket.as_ref().ok_or("No audio socket")?;
        match s.recv_from(buffer) {
            Ok((size, _)) => Ok(size),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(format!("{}", e)),
        }
    }
    pub fn announce(&self, channel: u8) -> Result<(), String> {
        let s = self.discovery_socket.as_ref().ok_or("No discovery socket")?;
        let name_bytes = self.local_name.as_bytes();
        let mut packet = Vec::with_capacity(3 + name_bytes.len());
        packet.push(DiscoveryMsgType::Announce as u8); packet.push(channel); packet.push(name_bytes.len() as u8);
        packet.extend_from_slice(name_bytes);
        s.send_to(&packet, SocketAddrV4::new(MULTICAST_GROUP, DISCOVERY_PORT)).map_err(|e| format!("{}", e))?;
        Ok(())
    }
    pub fn check_discovery(&mut self) -> Vec<WifiPeer> {
        let s = match self.discovery_socket.as_ref() { Some(s) => s, None => return Vec::new() };
        let mut buffer = [0u8; 256]; let mut new_peers = Vec::new();
        loop {
            match s.recv_from(&mut buffer) {
                Ok((size, addr)) => {
                    if size < 3 { continue; }
                    let msg_type = buffer[0]; let channel = buffer[1]; let name_len = buffer[2] as usize;
                    if size < 3 + name_len { continue; }
                    let name = String::from_utf8_lossy(&buffer[3..3+name_len]).to_string();
                    if msg_type == DiscoveryMsgType::Announce as u8 || msg_type == DiscoveryMsgType::Response as u8 {
                        if let std::net::SocketAddr::V4(v4) = addr {
                            let ip = *v4.ip();
                            if !self.peers.iter().any(|p| p.address == ip) {
                                let peer = WifiPeer { address: ip, device_name: name, channel };
                                new_peers.push(peer.clone()); self.peers.push(peer);
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
        new_peers
    }
    pub fn goodbye(&self) {
        if let Some(s) = &self.discovery_socket {
            let nb = self.local_name.as_bytes();
            let mut p = Vec::with_capacity(3+nb.len());
            p.push(DiscoveryMsgType::Goodbye as u8); p.push(0); p.push(nb.len() as u8); p.extend_from_slice(nb);
            let _ = s.send_to(&p, SocketAddrV4::new(MULTICAST_GROUP, DISCOVERY_PORT));
        }
    }
    pub fn get_peers(&self) -> &[WifiPeer] { &self.peers }
    pub fn has_peers(&self) -> bool { !self.peers.is_empty() }
    pub fn get_state(&self) -> WifiState { self.state }
    pub fn activate(&mut self) { self.state = WifiState::Active; }
    pub fn shutdown(&mut self) { self.goodbye(); self.state = WifiState::Inactive; self.audio_socket = None; self.discovery_socket = None; self.peers.clear(); }
}
impl Drop for WifiTransport { fn drop(&mut self) { self.shutdown(); } }
