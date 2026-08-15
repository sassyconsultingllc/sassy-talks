// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-ZIXFDMTBKCCQ
/// Transport Module - Unified abstraction over WiFi Direct, WiFi Multicast,
/// and the Cloudflare relay.
///
/// Relay-first, fan-out:
/// 1. Cloudflare relay is the preferred common meeting point (any internet).
/// 2. WiFi multicast / WiFi Direct run *alongside* relay for nearby peers.
/// 3. Bluetooth RFCOMM is last-resort when no IP path is up.
///
/// Audio is fanned out to every live IP plane so mixed-protocol groups
/// (one peer on relay, one on WiFi, one on BT+relay) can still hear each
/// other. `active` is the preferred UI/PTT slot, not an XOR send gate.
use log::{error, info, warn};

use crate::cellular_transport::{CellularState, CellularTransport, PacketQueue};
use crate::crypto::CryptoSession;
use crate::wifi_direct::{GroupRole, WifiDirectManager, WifiDirectPeer, WifiDirectState};
use crate::wifi_transport::{WifiPeer, WifiState, WifiTransport};
use std::time::Duration;

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
    /// Staged hybrid session: decrypt-only until the peer is live on the new
    /// key. Prevents a lost CONFIRM_ACK from splitting TX keys.
    pending_rx: Option<CryptoSession>,
    /// Sticky user/UI preference. It survives bearer flaps and is never used
    /// as proof that a bearer can transmit right now.
    preferred: ActiveTransport,
    /// True while at least one RFCOMM peer is connected (Kotlin-managed).
    /// Used to re-promote Bluetooth when IP paths die without a fresh
    /// `on_bluetooth_connected` callback.
    bluetooth_connected: bool,
    bluetooth_tx_allowed: bool,
    bluetooth_outbound: PacketQueue,
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
            pending_rx: None,
            preferred: ActiveTransport::None,
            bluetooth_connected: false,
            bluetooth_tx_allowed: false,
            bluetooth_outbound: PacketQueue::new(32, Duration::from_millis(500)),
            device_name: device_name.to_string(),
        })
    }

    /// WiFi is an extra plane, not a replacement for a live relay hub.
    fn adopt_wifi_as_active(&mut self) {
        if self.cellular.get_state() == CellularState::Connected {
            info!("TransportManager: WiFi up alongside relay — keeping Cellular as preferred active (fan-out)");
        } else {
            self.preferred = ActiveTransport::Wifi;
        }
    }

    /// Initialize WiFi multicast transport (call after permissions granted)
    pub fn init_wifi(&mut self) -> Result<(), String> {
        self.wifi.init()
    }

    /// Install the active encryption session (one channel active at a time).
    pub fn set_crypto(&mut self, session: CryptoSession) {
        self.crypto = Some(session);
        self.pending_rx = None;
        info!("TransportManager: encryption enabled");
    }

    /// Arm a staged hybrid session for RX only. TX stays on the live (old) key
    /// until [promote_pending_rx] or a successful decrypt of peer ciphertext.
    pub fn arm_pending_rx(&mut self, session: CryptoSession) {
        self.pending_rx = Some(session);
        info!("TransportManager: staged hybrid RX armed (TX still on live key)");
    }

    pub fn discard_pending_rx(&mut self) {
        if self.pending_rx.take().is_some() {
            info!("TransportManager: staged hybrid RX discarded");
        }
    }

    /// Set encryption from pre-shared key
    pub fn set_psk(&mut self, key: &[u8; 32]) {
        self.crypto = Some(CryptoSession::from_psk(key));
        self.pending_rx = None;
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
        if self.cellular.get_state() == CellularState::Connected {
            info!("TransportManager: WiFi Direct up alongside relay — keeping Cellular as preferred active (fan-out)");
        } else {
            self.preferred = ActiveTransport::WifiDirect;
            info!("TransportManager: active transport = WifiDirect (multicast on P2P network)");
        }
        Ok(())
    }

    /// Called when WiFi Direct group is dissolved
    pub fn on_wifi_direct_disconnected(&mut self) {
        info!("TransportManager: WiFi Direct group dissolved");
        self.wifi.shutdown();

        if matches!(
            self.preferred,
            ActiveTransport::WifiDirect | ActiveTransport::Wifi
        ) {
            info!("TransportManager: WiFi path down — retaining sticky preference");
        }
    }

    // ── WiFi Multicast operations (cross-platform, shared WiFi network) ──

    /// Start multicast transport directly (for cross-platform use on shared WiFi)
    pub fn connect_wifi_multicast(&mut self) -> Result<(), String> {
        info!("TransportManager: starting WiFi multicast (cross-platform mode)");
        self.wifi.init()?;
        self.wifi.activate();
        self.adopt_wifi_as_active();
        if self.preferred == ActiveTransport::Wifi {
            info!("TransportManager: active transport = WiFi multicast");
        }
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

    /// Relay transport stats as JSON — room, wire packet counts, queue depths
    /// and drop counters. Pairs with `diag::snapshot_json` (which reports what
    /// happened to those packets after they arrived) to give the full relay
    /// picture in one read.
    pub fn cellular_stats(&self) -> String {
        self.cellular.get_stats()
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

    /// Decrypt raw data using the live session, then the staged hybrid RX key.
    /// A hit on the staged key promotes it to live TX+RX (peer completed ACK).
    pub fn decrypt_raw(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        self.decrypt_live_or_pending(data)
    }

    fn decrypt_live_or_pending(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        if let Some(ref crypto) = self.crypto {
            if let Ok(pt) = crypto.decrypt(data) {
                return Ok(pt);
            }
        }
        if let Some(ref pending) = self.pending_rx {
            if let Ok(pt) = pending.decrypt(data) {
                if let Some(session) = self.pending_rx.take() {
                    self.crypto = Some(session);
                    info!("TransportManager: staged hybrid RX promoted to live TX");
                }
                return Ok(pt);
            }
        }
        if self.crypto.is_none() && self.pending_rx.is_none() {
            Err("No encryption session".to_string())
        } else {
            Err("Decryption failed".to_string())
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
        crate::diag::note_tx();
        // Debug-only: show the same frame before and after AEAD so the
        // encryption can be verified from outside the process rather than
        // taken on faith. No-op unless the diagnostic tooling armed it.
        crate::diag::trace_crypto(data, &payload);

        // Fan-out to every live IP plane. Gating on `active` was the
        // mixed-protocol silence bug: a relay peer never heard a WiFi-primary
        // sender unless that sender happened to also be marked Wifi (the old
        // dual-path block), and a Cellular-primary sender never hit LAN
        // multicast at all. Bluetooth TX is a separate Kotlin RFCOMM pump;
        // we still tee IP so a BT-labelled device with relay/WiFi up can
        // reach remote peers.
        let wifi_live = self.wifi.get_state() == WifiState::Active;
        let cell_live = self.cellular.can_transmit_realtime();

        let mut sent_any = false;
        let mut last_err: Option<String> = None;

        if wifi_live {
            match self.wifi.send_audio(&payload) {
                Ok(_) => {
                    sent_any = true;
                    crate::diag::note_tx_success();
                }
                Err(e) => last_err = Some(e),
            }
        }
        if cell_live {
            if self.cellular.is_outbound_congested() {
                warn!("Fan-out: relay outbound queue congested, skipping relay send");
            } else {
                match self.cellular.send_audio(&payload) {
                    Ok(_) => sent_any = true,
                    Err(e) => {
                        warn!("Fan-out: relay send failed: {}", e);
                        last_err = Some(e);
                    }
                }
            }
        }

        if self.bluetooth_connected && self.bluetooth_tx_allowed {
            self.bluetooth_outbound.push(payload.clone());
            sent_any = true;
        }

        if sent_any {
            Ok(payload.len())
        } else {
            Err(last_err.unwrap_or_else(|| "No active transport".to_string()))
        }
    }

    /// Decrypt a raw transport payload into `buffer`. Returns 0 on failure/drop.
    fn decrypt_into(&mut self, raw_data: &[u8], buffer: &mut [u8]) -> Result<usize, String> {
        match self.decrypt_live_or_pending(raw_data) {
            Ok(plaintext) => {
                crate::diag::note_decrypt_ok();
                let copy_len = plaintext.len().min(buffer.len());
                buffer[..copy_len].copy_from_slice(&plaintext[..copy_len]);
                Ok(copy_len)
            }
            Err(e) => {
                if self.crypto.is_none() && self.pending_rx.is_none() {
                    crate::diag::note_decrypt_no_session();
                    warn!(
                        "RX: No encryption session, dropping {} bytes",
                        raw_data.len()
                    );
                } else {
                    crate::diag::note_decrypt_fail(&e);
                    error!("Decryption failed (dropping packet): {}", e);
                }
                Ok(0)
            }
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
        let raw_data = match self.live_transport() {
            ActiveTransport::Wifi | ActiveTransport::WifiDirect => {
                match self.poll_wifi_into(buffer)? {
                    0 if self.cellular.get_state() == CellularState::Connected => {
                        self.poll_cellular_into(buffer)?
                    }
                    n => n,
                }
            }
            ActiveTransport::Cellular => match self.poll_cellular_into(buffer)? {
                0 => self.poll_wifi_into(buffer)?,
                n => n,
            },
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
    /// Relay is the preferred hub. `send()` fans out to every live IP plane,
    /// so promoting Cellular here does not silence WiFi multicast (or BT).
    pub fn on_cellular_connected(&mut self) -> Result<(), String> {
        info!("TransportManager: cellular WebSocket connected");
        self.cellular.on_connected();
        let previous = self.preferred;
        self.preferred = ActiveTransport::Cellular;
        if matches!(
            previous,
            ActiveTransport::Wifi | ActiveTransport::WifiDirect
        ) {
            info!("TransportManager: relay up alongside {:?} — Cellular is preferred active (fan-out keeps WiFi TX)", previous);
        } else {
            info!("TransportManager: active transport = Cellular");
        }
        Ok(())
    }

    /// Called by Kotlin when WebSocket disconnects
    pub fn on_cellular_disconnected(&mut self, reason: &str) {
        info!("TransportManager: cellular disconnected: {}", reason);
        self.cellular.on_disconnected(reason);

        if self.preferred == ActiveTransport::Cellular {
            info!("TransportManager: relay down — retaining sticky Cellular preference");
        }
    }

    /// True while an IP transport (WiFi multicast or the relay) is live —
    /// i.e. the shared audio TX/RX threads still have a path to serve.
    pub fn has_live_ip_transport(&self) -> bool {
        self.wifi.get_state() == WifiState::Active || self.cellular.can_transmit_realtime()
    }

    /// True while any plane can carry audio (IP or Bluetooth RFCOMM).
    pub fn has_live_audio_path(&self) -> bool {
        self.has_live_ip_transport() || (self.bluetooth_connected && self.bluetooth_tx_allowed)
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

    pub fn report_cellular_send_result(&mut self, success: bool) {
        self.cellular.report_send_result(success);
    }

    /// Get cellular stats JSON
    pub fn get_cellular_stats(&self) -> String {
        self.cellular.get_stats()
    }

    // ── Bluetooth operations (Kotlin-managed RFCOMM, Rust handles codec) ──

    /// Called by Kotlin when BT RFCOMM connects.
    ///
    /// Bluetooth is the last-resort plane: it only takes the active audio slot
    /// when no IP transport (relay / WiFi) is carrying audio. BT's own Kotlin
    /// RFCOMM pump runs in parallel regardless, and `send()` fans out to any
    /// live IP path so mixed-protocol peers still hear us.
    pub fn on_bluetooth_connected(&mut self) {
        self.bluetooth_connected = true;
        self.bluetooth_tx_allowed = true;
        if self.preferred == ActiveTransport::None {
            self.preferred = ActiveTransport::Bluetooth;
            info!("TransportManager: Bluetooth RFCOMM connected — now the active path (no IP transport up)");
        } else {
            info!("TransportManager: Bluetooth RFCOMM connected — IP path still active, BT runs in parallel");
        }
    }

    /// Called by Kotlin when BT RFCOMM disconnects
    pub fn on_bluetooth_disconnected(&mut self) {
        info!("TransportManager: Bluetooth disconnected");
        self.bluetooth_connected = false;
        self.bluetooth_tx_allowed = false;
        self.bluetooth_outbound.clear();
        if self.preferred == ActiveTransport::Bluetooth {
            info!("TransportManager: Bluetooth down — retaining sticky preference");
        }
    }

    /// Poll a Bluetooth packet produced by the shared capture/encode pass.
    pub fn poll_bluetooth_outbound(&self) -> Option<Vec<u8>> {
        self.bluetooth_outbound.pop()
    }

    pub fn report_bluetooth_send_result(&mut self, success: bool) {
        self.bluetooth_tx_allowed = success;
        if success {
            crate::diag::note_tx_success();
        } else {
            crate::diag::note_tx_failure();
        }
    }

    /// Best bearer that can transmit right now.
    pub fn active_transport(&self) -> ActiveTransport {
        self.live_transport()
    }

    /// Sticky preferred bearer, independent from current liveness.
    pub fn preferred_transport(&self) -> ActiveTransport {
        self.preferred
    }

    fn live_transport(&self) -> ActiveTransport {
        if self.preferred == ActiveTransport::Cellular && self.cellular.can_transmit_realtime() {
            ActiveTransport::Cellular
        } else if matches!(
            self.preferred,
            ActiveTransport::Wifi | ActiveTransport::WifiDirect
        ) && self.wifi.get_state() == WifiState::Active
        {
            self.preferred
        } else if self.preferred == ActiveTransport::Bluetooth
            && self.bluetooth_connected
            && self.bluetooth_tx_allowed
        {
            ActiveTransport::Bluetooth
        } else if self.cellular.can_transmit_realtime() {
            ActiveTransport::Cellular
        } else if self.wifi.get_state() == WifiState::Active {
            ActiveTransport::Wifi
        } else if self.bluetooth_connected && self.bluetooth_tx_allowed {
            ActiveTransport::Bluetooth
        } else {
            ActiveTransport::None
        }
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
        self.bluetooth_tx_allowed = false;
        self.bluetooth_outbound.clear();
        self.preferred = ActiveTransport::None;

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
        tm.bluetooth_tx_allowed = true;
        tm.preferred = ActiveTransport::Cellular;
        // cellular state starts Disconnected; mark active Cellular then disconnect
        // via the public path which clears cellular and re-promotes.
        tm.on_cellular_disconnected("test flap");
        assert_eq!(tm.active_transport(), ActiveTransport::Bluetooth);
    }

    #[test]
    fn wifi_preferred_over_bluetooth_on_connect() {
        let mut tm = TransportManager::new("test").unwrap();
        tm.bluetooth_connected = true;
        tm.preferred = ActiveTransport::Wifi;
        tm.on_bluetooth_connected();
        assert_eq!(tm.preferred_transport(), ActiveTransport::Wifi);
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

    #[test]
    fn relay_takes_preferred_slot_even_when_wifi_was_active() {
        let mut tm = TransportManager::new("test").unwrap();
        tm.preferred = ActiveTransport::Wifi;
        tm.on_cellular_connected().unwrap();
        assert_eq!(tm.active_transport(), ActiveTransport::Cellular);
        assert_eq!(tm.preferred_transport(), ActiveTransport::Cellular);
    }

    #[test]
    fn bluetooth_does_not_steal_relay() {
        let mut tm = TransportManager::new("test").unwrap();
        tm.on_cellular_connected().unwrap();
        tm.on_bluetooth_connected();
        assert_eq!(tm.active_transport(), ActiveTransport::Cellular);
        assert!(tm.bluetooth_connected);
    }

    #[test]
    fn relay_disconnect_keeps_preference_but_not_live_state() {
        let mut tm = TransportManager::new("test").unwrap();
        tm.on_cellular_connected().unwrap();
        tm.on_cellular_disconnected("blip");
        assert_eq!(tm.preferred_transport(), ActiveTransport::Cellular);
        assert_eq!(tm.active_transport(), ActiveTransport::None);
    }

    #[test]
    fn wifi_direct_down_promotes_relay_when_connected() {
        let mut tm = TransportManager::new("test").unwrap();
        tm.on_cellular_connected().unwrap();
        tm.preferred = ActiveTransport::WifiDirect;
        tm.on_wifi_direct_disconnected();
        assert_eq!(tm.active_transport(), ActiveTransport::Cellular);
    }
}
