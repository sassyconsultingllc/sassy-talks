/// Transport Module - Unified abstraction over WiFi Direct and WiFi Multicast
///
/// Transport priority:
/// 1. WiFi Direct (Android-to-Android, no router needed) + multicast on top
/// 2. WiFi Multicast (cross-platform: Android + iOS + Desktop, same WiFi network)
///
/// WiFi Direct creates an ad-hoc network between devices, then multicast runs
/// on that network. For cross-platform use, devices on the same WiFi use
/// multicast directly (no WiFi Direct needed since a router already provides
/// the shared network).

use log::{error, info, warn};

use crate::wifi_transport::{WifiTransport, WifiState, WifiPeer};
use crate::wifi_direct::{WifiDirectManager, WifiDirectState, WifiDirectPeer, GroupRole};
use crate::cellular_transport::{CellularTransport, CellularState};
use crate::crypto::CryptoSession;

/// Which transport is currently active for data
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveTransport {
    None,
    Wifi,
    WifiDirect,
    Cellular,
}

/// Unified transport manager
pub struct TransportManager {
    wifi: WifiTransport,
    wifi_direct: WifiDirectManager,
    cellular: CellularTransport,
    crypto: Option<CryptoSession>,
    active: ActiveTransport,
    device_name: String,
}

impl TransportManager {
    pub fn new(device_name: &str) -> Result<Self, String> {
        info!("TransportManager: initializing");

        let wifi = WifiTransport::new(device_name);
        let wifi_direct = WifiDirectManager::new();
        let cellular = CellularTransport::new(device_name);

        Ok(Self {
            wifi,
            wifi_direct,
            cellular,
            crypto: None,
            active: ActiveTransport::None,
            device_name: device_name.to_string(),
        })
    }

    /// Initialize WiFi multicast transport (call after permissions granted)
    pub fn init_wifi(&mut self) -> Result<(), String> {
        self.wifi.init()
    }

    /// Set encryption session (call after key exchange)
    pub fn set_crypto(&mut self, session: CryptoSession) {
        self.crypto = Some(session);
        info!("TransportManager: encryption enabled");
    }

    /// Set encryption from pre-shared key
    pub fn set_psk(&mut self, key: &[u8; 32]) {
        self.crypto = Some(CryptoSession::from_psk(key));
        info!("TransportManager: PSK encryption enabled");
    }

    // ── WiFi Direct operations ──

    /// Get WiFi Direct manager (mutable, for JNI callbacks)
    pub fn wifi_direct_mut(&mut self) -> &mut WifiDirectManager {
        &mut self.wifi_direct
    }

    /// Get WiFi Direct state
    pub fn wifi_direct_state(&self) -> WifiDirectState {
        self.wifi_direct.get_state()
    }

    /// Get WiFi Direct peers
    pub fn get_wifi_direct_peers(&self) -> &[WifiDirectPeer] {
        self.wifi_direct.get_peers()
    }

    /// Check if WiFi Direct has discovered peers
    pub fn has_wifi_direct_peers(&self) -> bool {
        self.wifi_direct.has_peers()
    }

    /// Get WiFi Direct group role
    pub fn wifi_direct_role(&self) -> GroupRole {
        self.wifi_direct.get_role()
    }

    /// Called when WiFi Direct group is formed — start multicast on the P2P network.
    /// This is the key integration point: WiFi Direct provides the network,
    /// multicast provides the audio transport running on that network.
    pub fn on_wifi_direct_connected(&mut self) -> Result<(), String> {
        info!("TransportManager: WiFi Direct group formed, starting multicast transport");

        // Initialize multicast on the WiFi Direct network interface
        self.wifi.init()?;
        self.wifi.activate();
        self.active = ActiveTransport::WifiDirect;

        info!("TransportManager: active transport = WifiDirect (multicast on P2P network)");
        Ok(())
    }

    /// Called when WiFi Direct group is dissolved
    pub fn on_wifi_direct_disconnected(&mut self) {
        info!("TransportManager: WiFi Direct group dissolved");
        self.wifi.shutdown();

        if self.active == ActiveTransport::WifiDirect {
            self.active = ActiveTransport::None;
        }
    }

    // ── WiFi Multicast operations (cross-platform, shared WiFi network) ──

    /// Start multicast transport directly (for cross-platform use on shared WiFi)
    pub fn connect_wifi_multicast(&mut self) -> Result<(), String> {
        info!("TransportManager: starting WiFi multicast (cross-platform mode)");
        self.wifi.init()?;
        self.wifi.activate();
        self.active = ActiveTransport::Wifi;
        info!("TransportManager: active transport = WiFi multicast");
        Ok(())
    }

    /// Start WiFi peer discovery (sends periodic announcements)
    pub fn announce_wifi(&self, channel: u8) {
        if let Err(e) = self.wifi.announce(channel) {
            // Non-fatal: WiFi may not be available
            warn!("WiFi announce failed: {}", e);
        }
    }

    pub fn wifi_state(&self) -> WifiState {
        self.wifi.get_state()
    }

    pub fn get_wifi_peers(&self) -> &[WifiPeer] {
        self.wifi.get_peers()
    }

    pub fn has_wifi_peers(&self) -> bool {
        self.wifi.has_peers()
    }

    // ── Unified send/receive ──

    /// Check if encryption is enabled (valid crypto session exists)
    pub fn is_encrypted(&self) -> bool {
        self.crypto.is_some()
    }

    /// Send data through the active transport with encryption
    /// SECURITY: Refuses to send if no encryption session is active.
    pub fn send(&mut self, data: &[u8]) -> Result<usize, String> {
        // MANDATORY ENCRYPTION: refuse to transmit cleartext
        let payload = if let Some(ref mut crypto) = self.crypto {
            crypto.encrypt(data)?
        } else {
            return Err("Encryption required: authenticate via QR code first".to_string());
        };

        match self.active {
            ActiveTransport::WifiDirect | ActiveTransport::Wifi => {
                self.wifi.send_audio(&payload)
            }
            ActiveTransport::Cellular => {
                self.cellular.send_audio(&payload)
            }
            ActiveTransport::None => {
                Err("No active transport".to_string())
            }
        }
    }

    /// Receive data from active transport with decryption
    pub fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        let raw_data = match self.active {
            ActiveTransport::Wifi | ActiveTransport::WifiDirect => {
                let mut wifi_buf = vec![0u8; buffer.len() + 128]; // extra for crypto overhead
                match self.wifi.receive_audio(&mut wifi_buf) {
                    Ok(n) if n > 0 => wifi_buf[..n].to_vec(),
                    Ok(_) => return Ok(0),
                    Err(e) => {
                        if !e.contains("would block") && !e.contains("timed out") {
                            warn!("WiFi receive failed: {}", e);
                        }
                        return Ok(0);
                    }
                }
            }
            ActiveTransport::Cellular => {
                let mut cell_buf = vec![0u8; buffer.len() + 128];
                match self.cellular.receive_audio(&mut cell_buf) {
                    Ok(n) if n > 0 => cell_buf[..n].to_vec(),
                    Ok(_) => return Ok(0),
                    Err(e) => {
                        warn!("Cellular receive failed: {}", e);
                        return Ok(0);
                    }
                }
            }
            ActiveTransport::None => {
                return Ok(0);
            }
        };

        // MANDATORY DECRYPTION: drop unencrypted or tampered packets
        if let Some(ref crypto) = self.crypto {
            match crypto.decrypt(&raw_data) {
                Ok(plaintext) => {
                    let copy_len = plaintext.len().min(buffer.len());
                    buffer[..copy_len].copy_from_slice(&plaintext[..copy_len]);
                    Ok(copy_len)
                }
                Err(e) => {
                    error!("Decryption failed (dropping packet): {}", e);
                    Ok(0) // Drop packet silently instead of propagating error
                }
            }
        } else {
            // No crypto session — drop all incoming data
            warn!("RX: No encryption session, dropping {} bytes", raw_data.len());
            Ok(0)
        }
    }

    // ── Cellular operations (WebSocket relay, works anywhere with internet) ──

    /// Get mutable reference to cellular transport (for JNI callbacks)
    pub fn cellular_mut(&mut self) -> &mut CellularTransport {
        &mut self.cellular
    }

    /// Get cellular state
    pub fn cellular_state(&self) -> CellularState {
        self.cellular.get_state()
    }

    /// Set cellular room ID (from QR session_id)
    pub fn set_cellular_room(&mut self, room_id: String) {
        self.cellular.set_room_id(room_id);
    }

    /// Get WebSocket URL for Kotlin to connect to
    pub fn get_cellular_ws_url(&self) -> String {
        self.cellular.get_ws_url()
    }

    /// Called by Kotlin when WebSocket connects successfully
    pub fn on_cellular_connected(&mut self) -> Result<(), String> {
        info!("TransportManager: cellular WebSocket connected");
        self.cellular.on_connected();
        self.active = ActiveTransport::Cellular;
        info!("TransportManager: active transport = Cellular");
        Ok(())
    }

    /// Called by Kotlin when WebSocket disconnects
    pub fn on_cellular_disconnected(&mut self, reason: &str) {
        info!("TransportManager: cellular disconnected: {}", reason);
        self.cellular.on_disconnected(reason);

        if self.active == ActiveTransport::Cellular {
            self.active = ActiveTransport::None;
        }
    }

    /// Called by Kotlin when WebSocket receives a binary message
    pub fn on_cellular_message(&mut self, data: Vec<u8>) {
        self.cellular.on_message_received(data);
    }

    /// Called by Kotlin when WebSocket has an error
    pub fn on_cellular_error(&mut self, error: &str) {
        self.cellular.on_error(error);
    }

    /// Poll outbound queue (called by Kotlin to get packets to send via WS)
    pub fn poll_cellular_outbound(&self) -> Option<Vec<u8>> {
        self.cellular.poll_outbound()
    }

    /// Get cellular stats JSON
    pub fn get_cellular_stats(&self) -> String {
        self.cellular.get_stats()
    }

    /// Get which transport is currently active
    pub fn active_transport(&self) -> ActiveTransport {
        self.active
    }

    /// Get the local device name
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Set the local device name
    pub fn set_device_name(&mut self, name: &str) {
        self.device_name = name.to_string();
    }

    /// Disconnect all transports
    pub fn disconnect(&mut self) -> Result<(), String> {
        info!("TransportManager: disconnecting all");

        self.wifi.shutdown();
        self.wifi_direct.reset();
        self.cellular.shutdown();
        self.crypto = None;
        self.active = ActiveTransport::None;

        Ok(())
    }

    /// Shutdown everything
    pub fn shutdown(&mut self) -> Result<(), String> {
        self.disconnect()
    }
}

impl Drop for TransportManager {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
