use log::{error, info, warn};
use crate::bluetooth::{BluetoothManager, BluetoothDevice, ConnectionState};
use crate::wifi_transport::{WifiTransport, WifiState, WifiPeer};
use crate::crypto::CryptoSession;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveTransport { None, Bluetooth, Wifi }

pub struct TransportManager {
    bluetooth: BluetoothManager, wifi: WifiTransport, crypto: Option<CryptoSession>,
    active: ActiveTransport, device_name: String,
}

#[allow(dead_code)]
impl TransportManager {
    pub fn new(device_name: &str) -> Result<Self, String> {
        let bluetooth = BluetoothManager::new()?;
        let wifi = WifiTransport::new(device_name);
        Ok(Self { bluetooth, wifi, crypto: None, active: ActiveTransport::None, device_name: device_name.to_string() })
    }
    pub fn init_wifi(&mut self) -> Result<(), String> { self.wifi.init() }
    pub fn set_crypto(&mut self, session: CryptoSession) { self.crypto = Some(session); info!("TransportManager: encryption enabled"); }
    pub fn set_psk(&mut self, key: &[u8; 32]) { self.crypto = Some(CryptoSession::from_psk(key)); }
    pub fn is_bluetooth_enabled(&self) -> bool { self.bluetooth.is_enabled() }
    pub fn enable_bluetooth(&self) -> Result<(), String> { self.bluetooth.enable() }
    pub fn get_paired_devices(&mut self) -> Result<Vec<BluetoothDevice>, String> { self.bluetooth.get_paired_devices() }
    pub fn connect_bluetooth(&mut self, address: &str) -> Result<(), String> {
        self.bluetooth.connect(address)?; self.active = ActiveTransport::Bluetooth;
        if let Err(e) = self.wifi.init() { warn!("WiFi init failed: {}", e); }
        Ok(())
    }
    pub fn listen_bluetooth(&mut self) -> Result<(), String> {
        self.bluetooth.listen()?; self.active = ActiveTransport::Bluetooth;
        if let Err(e) = self.wifi.init() { warn!("WiFi init failed: {}", e); }
        Ok(())
    }
    pub fn bt_state(&self) -> ConnectionState { self.bluetooth.get_state() }
    pub fn get_connected_device(&self) -> Option<BluetoothDevice> { self.bluetooth.get_connected_device() }
    pub fn announce_wifi(&self, channel: u8) { if let Err(e) = self.wifi.announce(channel) { warn!("WiFi announce failed: {}", e); } }
    pub fn check_wifi_upgrade(&mut self) -> bool {
        let new_peers = self.wifi.check_discovery();
        if !new_peers.is_empty() && self.active == ActiveTransport::Bluetooth {
            self.wifi.activate(); self.active = ActiveTransport::Wifi; info!("TransportManager: upgraded to WiFi"); return true;
        }
        false
    }
    pub fn wifi_state(&self) -> WifiState { self.wifi.get_state() }
    pub fn get_wifi_peers(&self) -> &[WifiPeer] { self.wifi.get_peers() }
    pub fn has_wifi_peers(&self) -> bool { self.wifi.has_peers() }
    pub fn is_encrypted(&self) -> bool { self.crypto.is_some() }
    pub fn send(&mut self, data: &[u8]) -> Result<usize, String> {
        let payload = if let Some(ref mut crypto) = self.crypto { crypto.encrypt(data)? }
        else { return Err("Encryption required: authenticate via QR code first".into()); };
        match self.active {
            ActiveTransport::Wifi => match self.wifi.send_audio(&payload) {
                Ok(n) => Ok(n),
                Err(e) => { warn!("WiFi send failed, fallback to BT: {}", e); self.active = ActiveTransport::Bluetooth; self.bluetooth.send_audio(&payload) }
            },
            ActiveTransport::Bluetooth => self.bluetooth.send_audio(&payload),
            ActiveTransport::None => Err("No active transport".into()),
        }
    }
    pub fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        let raw_data = if self.active == ActiveTransport::Wifi {
            let mut wb = vec![0u8; buffer.len()+128];
            match self.wifi.receive_audio(&mut wb) { Ok(n) if n > 0 => Some(wb[..n].to_vec()), Ok(_) => None, Err(_) => None }
        } else { None };
        let raw_data = if let Some(data) = raw_data { data }
        else if self.bt_state() == ConnectionState::Connected {
            let mut bb = vec![0u8; buffer.len()+128];
            match self.bluetooth.receive_audio(&mut bb) { Ok(n) if n > 0 => bb[..n].to_vec(), Ok(_) => return Ok(0), Err(_) => return Ok(0) }
        } else { return Ok(0); };
        if let Some(ref crypto) = self.crypto {
            match crypto.decrypt(&raw_data) {
                Ok(plain) => { let cl = plain.len().min(buffer.len()); buffer[..cl].copy_from_slice(&plain[..cl]); Ok(cl) }
                Err(e) => { error!("Decryption failed: {}", e); Ok(0) }
            }
        } else { warn!("No encryption session, dropping {} bytes", raw_data.len()); Ok(0) }
    }
    pub fn active_transport(&self) -> ActiveTransport { self.active }
    pub fn device_name(&self) -> &str { &self.device_name }
    pub fn disconnect(&mut self) -> Result<(), String> {
        self.wifi.shutdown(); self.bluetooth.disconnect()?; self.crypto = None; self.active = ActiveTransport::None; Ok(())
    }
    pub fn shutdown(&mut self) -> Result<(), String> { self.disconnect() }
}
impl Drop for TransportManager { fn drop(&mut self) { let _ = self.shutdown(); } }
