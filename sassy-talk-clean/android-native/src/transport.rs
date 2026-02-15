/// Transport Module - Unified abstraction over Bluetooth and WiFi
///
/// Implements smart transport selection:
/// - Default: Bluetooth RFCOMM
/// - Preferred: WiFi multicast when both peers report WiFi connectivity
/// - Automatic fallback: if WiFi fails, falls back to Bluetooth

use std::sync::{Arc, Mutex};
use log::{error, info, warn};

use crate::bluetooth::{BluetoothManager, BluetoothDevice, ConnectionState};
use crate::wifi_transport::{WifiTransport, WifiState, WifiPeer};
use crate::crypto::CryptoSession;

/// Which transport is currently active for data
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveTransport {
    None,
    Bluetooth,
    Wifi,
}

/// Unified transport manager
pub struct TransportManager {
    bluetooth: BluetoothManager,
    wifi: WifiTransport,
    crypto: Option<CryptoSession>,
    active: ActiveTransport,
    device_name: String,
}

impl TransportManager {
    pub fn new(device_name: &str) -> Result<Self, String> {
        info!("TransportManager: initializing");

        let bluetooth = BluetoothManager::new()?;
        let wifi = WifiTransport::new(device_name);

        Ok(Self {
            bluetooth,
            wifi,
            crypto: None,
            active: ActiveTransport::None,
            device_name: device_name.to_string(),
        })
    }

    /// Initialize WiFi transport (call after permissions granted)
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

    // ── Bluetooth operations ──

    pub fn is_bluetooth_enabled(&self) -> bool {
        self.bluetooth.is_enabled()
    }

    pub fn enable_bluetooth(&self) -> Result<(), String> {
        self.bluetooth.enable()
    }

    pub fn get_paired_devices(&mut self) -> Result<Vec<BluetoothDevice>, String> {
        self.bluetooth.get_paired_devices()
    }

    pub fn connect_bluetooth(&mut self, address: &str) -> Result<(), String> {
        self.bluetooth.connect(address)?;
        self.active = ActiveTransport::Bluetooth;
        info!("TransportManager: connected via Bluetooth to {}", address);

        // Try to also init WiFi for potential upgrade
        if let Err(e) = self.wifi.init() {
            warn!("TransportManager: WiFi init failed (BT-only mode): {}", e);
        }

        Ok(())
    }

    pub fn listen_bluetooth(&mut self) -> Result<(), String> {
        self.bluetooth.listen()?;
        self.active = ActiveTransport::Bluetooth;
        info!("TransportManager: listening via Bluetooth");

        // Try WiFi too
        if let Err(e) = self.wifi.init() {
            warn!("TransportManager: WiFi init failed (BT-only mode): {}", e);
        }

        Ok(())
    }

    pub fn bt_state(&self) -> ConnectionState {
        self.bluetooth.get_state()
    }

    pub fn get_connected_device(&self) -> Option<BluetoothDevice> {
        self.bluetooth.get_connected_device()
    }

    // ── WiFi operations ──

    /// Start WiFi peer discovery (sends periodic announcements)
    pub fn announce_wifi(&self, channel: u8) {
        if let Err(e) = self.wifi.announce(channel) {
            // Non-fatal: WiFi may not be available
            warn!("WiFi announce failed: {}", e);
        }
    }

    /// Check for WiFi peers and potentially upgrade transport
    pub fn check_wifi_upgrade(&mut self) -> bool {
        let new_peers = self.wifi.check_discovery();

        if !new_peers.is_empty() && self.active == ActiveTransport::Bluetooth {
            // Found WiFi peers while on Bluetooth - upgrade
            self.wifi.activate();
            self.active = ActiveTransport::Wifi;
            info!("TransportManager: upgraded to WiFi transport");
            return true;
        }

        false
    }

    pub fn wifi_state(&self) -> WifiState {
        self.wifi.get_state()
    }

    pub fn get_wifi_peers(&self) -> &[WifiPeer] {
        self.wifi.get_peers()
    }

    // ── Unified send/receive ──

    /// Send data through the active transport with encryption
    pub fn send(&mut self, data: &[u8]) -> Result<usize, String> {
        // Encrypt if crypto session is active
        let payload = if let Some(ref mut crypto) = self.crypto {
            crypto.encrypt(data)?
        } else {
            data.to_vec()
        };

        match self.active {
            ActiveTransport::Wifi => {
                match self.wifi.send_audio(&payload) {
                    Ok(n) => Ok(n),
                    Err(e) => {
                        // Fallback to Bluetooth
                        warn!("WiFi send failed, falling back to BT: {}", e);
                        self.active = ActiveTransport::Bluetooth;
                        self.bluetooth.send_audio(&payload)
                    }
                }
            }
            ActiveTransport::Bluetooth => {
                self.bluetooth.send_audio(&payload)
            }
            ActiveTransport::None => {
                Err("No active transport".to_string())
            }
        }
    }

    /// Receive data from active transport with decryption
    pub fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        // Try WiFi first if active (lower latency)
        let raw_data = if self.active == ActiveTransport::Wifi {
            let mut wifi_buf = vec![0u8; buffer.len() + 128]; // extra for crypto overhead
            match self.wifi.receive_audio(&mut wifi_buf) {
                Ok(n) if n > 0 => Some(wifi_buf[..n].to_vec()),
                Ok(_) => None, // No data on WiFi, try BT
                Err(e) => {
                    warn!("WiFi receive failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Fall back to Bluetooth if no WiFi data
        let raw_data = if let Some(data) = raw_data {
            data
        } else if self.bt_state() == ConnectionState::Connected {
            let mut bt_buf = vec![0u8; buffer.len() + 128];
            match self.bluetooth.receive_audio(&mut bt_buf) {
                Ok(n) if n > 0 => bt_buf[..n].to_vec(),
                Ok(_) => return Ok(0),
                Err(e) => {
                    if !e.contains("would block") {
                        error!("BT receive failed: {}", e);
                    }
                    return Ok(0);
                }
            }
        } else {
            return Ok(0);
        };

        // Decrypt if crypto session is active
        if let Some(ref crypto) = self.crypto {
            match crypto.decrypt(&raw_data) {
                Ok(plaintext) => {
                    let copy_len = plaintext.len().min(buffer.len());
                    buffer[..copy_len].copy_from_slice(&plaintext[..copy_len]);
                    Ok(copy_len)
                }
                Err(e) => {
                    error!("Decryption failed: {}", e);
                    Err(e)
                }
            }
        } else {
            let copy_len = raw_data.len().min(buffer.len());
            buffer[..copy_len].copy_from_slice(&raw_data[..copy_len]);
            Ok(copy_len)
        }
    }

    /// Get which transport is currently active
    pub fn active_transport(&self) -> ActiveTransport {
        self.active
    }

    /// Disconnect all transports
    pub fn disconnect(&mut self) -> Result<(), String> {
        info!("TransportManager: disconnecting all");

        self.wifi.shutdown();
        self.bluetooth.disconnect()?;
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
