// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-ZIXFDMTBKCCQ
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
    Bluetooth,
}

/// Unified transport manager.
///
/// Encryption: a single active `CryptoSession` is used for the currently active
/// channel. We do not maintain per-channel keys here because the channel byte
/// lives *inside* the encrypted wire frame — a receiver can't pick the right
/// per-channel key without first decrypting (chicken-and-egg). Channel
/// switching is handled at the JNI layer by re-installing the session for the
/// newly-selected channel.
pub struct TransportManager {
    wifi: WifiTransport,
    wifi_direct: WifiDirectManager,
    cellular: CellularTransport,
    crypto: Option<CryptoSession>,
    active: ActiveTransport,
    /// True while at least one RFCOMM peer is connected (Kotlin-managed).
    /// Used to re-promote Bluetooth when IP paths die without a fresh
    /// `on_bluetooth_connected` callback.
    bluetooth_connected: bool,
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
            bluetooth_connected: false,
            device_name: device_name.to_string(),
        })
    }

    /// Pick the best remaining plane after an IP path drops.
    /// Local-first: WiFi multicast > Bluetooth > None (relay handled by caller
    /// before this runs, or via a separate cellular-up event).
    fn promote_after_ip_loss(&mut self) -> ActiveTransport {
        if self.wifi.get_state() == WifiState::Active {
            info!("TransportManager: IP loss — WiFi multicast still active, promoting to Wifi");
            ActiveTransport::Wifi
        } else if self.bluetooth_connected {
            info!("TransportManager: IP loss — promoting Bluetooth (RFCOMM still up)");
            ActiveTransport::Bluetooth
        } else {
            info!("TransportManager: IP loss — no local path left");
            ActiveTransport::None
        }
    }

    /// Initialize WiFi multicast transport (call after permissions granted)
    pub fn init_wifi(&mut self) -> Result<(), String> {
        self.wifi.init()
    }

    /// Install the active encryption session (one channel active at a time).
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

        if matches!(
            self.active,
            ActiveTransport::WifiDirect | ActiveTransport::Wifi
        ) {
            // Prefer relay if still up; else Bluetooth; else None.
            self.active = if self.cellular.get_state() == CellularState::Connected {
                info!("TransportManager: WiFi Direct down — relay still up, promoting Cellular");
                ActiveTransport::Cellular
            } else {
                self.promote_after_ip_loss()
            };
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

    /// Encrypt raw data for a specific channel (for BT path).
    /// Falls back to legacy crypto if no per-channel key.
    pub fn encrypt_raw(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        // Try legacy crypto first (backward compat)
        if let Some(ref mut crypto) = self.crypto {
            crypto.encrypt(data)
        } else {
            Err("No encryption session".to_string())
        }
    }

    /// Decrypt raw data using the active session.
    pub fn decrypt_raw(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if let Some(ref crypto) = self.crypto {
            crypto.decrypt(data)
        } else {
            Err("No encryption session".to_string())
        }
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

        // Send on primary transport
        let primary_result = match self.active {
            ActiveTransport::WifiDirect | ActiveTransport::Wifi => {
                self.wifi.send_audio(&payload)
            }
            ActiveTransport::Cellular => {
                self.cellular.send_audio(&payload)
            }
            ActiveTransport::Bluetooth => {
                Ok(payload.len())
            }
            ActiveTransport::None => {
                Err("No active transport".to_string())
            }
        };

        // Also send on cellular relay if it's active and primary is WiFi
        // (dual-path: local peers get multicast, remote peers get relay)
        if matches!(self.active, ActiveTransport::Wifi | ActiveTransport::WifiDirect) {
            if self.cellular.get_state() == CellularState::Connected {
                if self.cellular.is_outbound_congested() {
                    warn!("Dual-path: relay outbound queue congested, skipping relay send");
                } else if let Err(e) = self.cellular.send_audio(&payload) {
                    warn!("Dual-path: relay send failed: {}", e);
                }
            }
        }

        primary_result
    }

    /// Decrypt a raw transport payload into `buffer`. Returns 0 on failure/drop.
    fn decrypt_into(&self, raw_data: &[u8], buffer: &mut [u8]) -> Result<usize, String> {
        if let Some(ref crypto) = self.crypto {
            match crypto.decrypt(raw_data) {
                Ok(plaintext) => {
                    let copy_len = plaintext.len().min(buffer.len());
                    buffer[..copy_len].copy_from_slice(&plaintext[..copy_len]);
                    Ok(copy_len)
                }
                Err(e) => {
                    error!("Decryption failed (dropping packet): {}", e);
                    Ok(0)
                }
            }
        } else {
            warn!("RX: No encryption session, dropping {} bytes", raw_data.len());
            Ok(0)
        }
    }

    /// Non-blocking WiFi multicast poll. Returns decrypted bytes written to
    /// `buffer`, or 0 if nothing waiting. Safe to call regardless of which
    /// transport is marked `active` — needed for dual-path RX when both WiFi
    /// and the cellular relay are live simultaneously.
    pub fn poll_wifi_into(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        use crate::wifi_transport::WifiState;
        if self.wifi.get_state() != WifiState::Active {
            return Ok(0);
        }
        // Reuse a thread-local scratch to avoid allocating every idle poll.
        thread_local! {
            static WIFI_SCRATCH: std::cell::RefCell<Vec<u8>> =
                std::cell::RefCell::new(Vec::with_capacity(2048));
        }
        WIFI_SCRATCH.with(|cell| {
            let mut wifi_buf = cell.borrow_mut();
            let need = buffer.len() + 128;
            if wifi_buf.len() < need {
                wifi_buf.resize(need, 0);
            }
            match self.wifi.receive_audio(&mut wifi_buf[..need]) {
                Ok(n) if n > 0 => self.decrypt_into(&wifi_buf[..n], buffer),
                Ok(_) => Ok(0),
                Err(e) => {
                    if !e.contains("would block") && !e.contains("timed out") {
                        warn!("WiFi receive failed: {}", e);
                    }
                    Ok(0)
                }
            }
        })
    }

    /// Non-blocking cellular relay poll. Returns decrypted bytes written to
    /// `buffer`, or 0 if nothing waiting.
    pub fn poll_cellular_into(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        if self.cellular.get_state() != CellularState::Connected {
            return Ok(0);
        }
        thread_local! {
            static CELL_SCRATCH: std::cell::RefCell<Vec<u8>> =
                std::cell::RefCell::new(Vec::with_capacity(2048));
        }
        CELL_SCRATCH.with(|cell| {
            let mut cell_buf = cell.borrow_mut();
            let need = buffer.len() + 128;
            if cell_buf.len() < need {
                cell_buf.resize(need, 0);
            }
            match self.cellular.receive_audio(&mut cell_buf[..need]) {
                Ok(n) if n > 0 => self.decrypt_into(&cell_buf[..n], buffer),
                Ok(_) => Ok(0),
                Err(e) => {
                    warn!("Cellular receive failed: {}", e);
                    Ok(0)
                }
            }
        })
    }

    /// Receive data from active transport with decryption
    pub fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        // Legacy single-source receive — prefer poll_wifi_into +
        // poll_cellular_into from the RX thread so both paths are drained.
        let raw_data = match self.active {
            ActiveTransport::Wifi | ActiveTransport::WifiDirect => {
                match self.poll_wifi_into(buffer)? {
                    0 if self.cellular.get_state() == CellularState::Connected => {
                        self.poll_cellular_into(buffer)?
                    }
                    n => n,
                }
            }
            ActiveTransport::Cellular => {
                match self.poll_cellular_into(buffer)? {
                    0 => self.poll_wifi_into(buffer)?,
                    n => n,
                }
            }
            ActiveTransport::Bluetooth => {
                return Ok(0);
            }
            ActiveTransport::None => {
                return Ok(0);
            }
        };
        Ok(raw_data)
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

    /// Called by Kotlin when WebSocket connects successfully.
    ///
    /// Like Bluetooth (see `on_bluetooth_connected`), the relay must NOT steal
    /// the active slot from an established WiFi path: `send()` routes the
    /// primary TX by `active` and only mirrors WiFi→relay (there is no
    /// relay→WiFi mirror), so promoting to Cellular here silenced LAN
    /// multicast TX for the rest of the session whenever both paths were up.
    /// With `active` left on Wifi, the dual-path block in `send()` still
    /// carries every frame to the relay too.
    pub fn on_cellular_connected(&mut self) -> Result<(), String> {
        info!("TransportManager: cellular WebSocket connected");
        self.cellular.on_connected();
        match self.active {
            ActiveTransport::Wifi | ActiveTransport::WifiDirect => {
                info!("TransportManager: relay up alongside {:?} — active unchanged (dual-path)", self.active);
            }
            _ => {
                self.active = ActiveTransport::Cellular;
                info!("TransportManager: active transport = Cellular");
            }
        }
        Ok(())
    }

    /// Called by Kotlin when WebSocket disconnects
    pub fn on_cellular_disconnected(&mut self, reason: &str) {
        info!("TransportManager: cellular disconnected: {}", reason);
        self.cellular.on_disconnected(reason);

        if self.active == ActiveTransport::Cellular {
            // Fall back to a still-live local path instead of going dark:
            // leaving `active = None` hard-blocked PTT even with WiFi or
            // RFCOMM fully up. Prefer WiFi, then re-promote Bluetooth.
            self.active = self.promote_after_ip_loss();
        }
    }

    /// True while an IP transport (WiFi multicast or the relay) is live —
    /// i.e. the shared audio TX/RX threads still have a path to serve.
    pub fn has_live_ip_transport(&self) -> bool {
        self.wifi.get_state() == WifiState::Active
            || self.cellular.get_state() == CellularState::Connected
    }

    /// True while any plane can carry audio (IP or Bluetooth RFCOMM).
    pub fn has_live_audio_path(&self) -> bool {
        self.has_live_ip_transport() || self.bluetooth_connected
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

    // ── Bluetooth operations (Kotlin-managed RFCOMM, Rust handles codec) ──

    /// Called by Kotlin when BT RFCOMM connects.
    ///
    /// Bluetooth is the FALLBACK plane: it only takes the active audio slot when
    /// no IP transport (WiFi multicast / relay) is carrying audio. Otherwise a BT
    /// peer wandering into range while we're on WiFi would silently steal `active`
    /// and stop WiFi TX, since `send()` routes by `active`. BT's own audio path
    /// (Kotlin `btEncodeFrame`/`btDecodeFrame` pump) runs in parallel regardless
    /// of this flag, so promoting it is unnecessary while an IP path is healthy —
    /// and harmful, because it would knock IP peers off the air.
    pub fn on_bluetooth_connected(&mut self) {
        self.bluetooth_connected = true;
        if self.active == ActiveTransport::None {
            self.active = ActiveTransport::Bluetooth;
            info!("TransportManager: Bluetooth RFCOMM connected — now the active path (no IP transport up)");
        } else {
            info!("TransportManager: Bluetooth RFCOMM connected — IP path still active, BT runs in parallel");
        }
    }

    /// Called by Kotlin when BT RFCOMM disconnects
    pub fn on_bluetooth_disconnected(&mut self) {
        info!("TransportManager: Bluetooth disconnected");
        self.bluetooth_connected = false;
        if self.active == ActiveTransport::Bluetooth {
            self.active = if self.wifi.get_state() == WifiState::Active {
                ActiveTransport::Wifi
            } else if self.cellular.get_state() == CellularState::Connected {
                ActiveTransport::Cellular
            } else {
                ActiveTransport::None
            };
        }
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
        self.bluetooth_connected = false;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_promoted_when_cellular_dies_and_rfcomm_up() {
        let mut tm = TransportManager::new("test").unwrap();
        // Simulate cellular as active primary with BT connected in parallel.
        tm.bluetooth_connected = true;
        tm.active = ActiveTransport::Cellular;
        // cellular state starts Disconnected; mark active Cellular then disconnect
        // via the public path which clears cellular and re-promotes.
        tm.on_cellular_disconnected("test flap");
        assert_eq!(tm.active_transport(), ActiveTransport::Bluetooth);
    }

    #[test]
    fn wifi_preferred_over_bluetooth_on_connect() {
        let mut tm = TransportManager::new("test").unwrap();
        tm.bluetooth_connected = true;
        tm.active = ActiveTransport::Wifi;
        tm.on_bluetooth_connected();
        assert_eq!(tm.active_transport(), ActiveTransport::Wifi);
        assert!(tm.bluetooth_connected);
    }

    #[test]
    fn bluetooth_takes_slot_when_no_ip() {
        let mut tm = TransportManager::new("test").unwrap();
        assert_eq!(tm.active_transport(), ActiveTransport::None);
        tm.on_bluetooth_connected();
        assert_eq!(tm.active_transport(), ActiveTransport::Bluetooth);
    }

    #[test]
    fn has_live_audio_path_includes_bluetooth() {
        let mut tm = TransportManager::new("test").unwrap();
        assert!(!tm.has_live_audio_path());
        tm.on_bluetooth_connected();
        assert!(tm.has_live_audio_path());
        assert!(!tm.has_live_ip_transport());
    }
}
