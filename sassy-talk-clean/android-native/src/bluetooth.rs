use std::sync::{Arc, Mutex};
use log::{warn, info, error};
use crate::jni_bridge::{AndroidBluetoothAdapter, AndroidBluetoothSocket, AndroidBluetoothServerSocket, AndroidInputStream, AndroidOutputStream};

pub const SERVICE_UUID: &str = "8ce255c0-223a-11e0-ac64-0803450c9a66";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionState { Disconnected, Connecting, Connected, Listening }

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BluetoothDevice { pub address: String, pub name: String, pub paired: bool }

pub struct BluetoothConnection {
    socket: Option<AndroidBluetoothSocket>, input_stream: Option<AndroidInputStream>,
    output_stream: Option<AndroidOutputStream>, server_socket: Option<AndroidBluetoothServerSocket>,
    state: ConnectionState, device: Option<BluetoothDevice>,
}

impl BluetoothConnection {
    pub fn new() -> Self { Self { socket: None, input_stream: None, output_stream: None, server_socket: None, state: ConnectionState::Disconnected, device: None } }
    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool { self.state == ConnectionState::Connected }
    pub fn send(&mut self, data: &[u8]) -> Result<usize, String> {
        match &self.output_stream { Some(s) => { s.write(data)?; s.flush()?; Ok(data.len()) } None => Err("Output stream not available".into()) }
    }
    pub fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        match &self.input_stream { Some(s) => s.read(buffer), None => Err("Input stream not available".into()) }
    }
    pub fn close(&mut self) -> Result<(), String> {
        if let Some(s) = &self.socket { s.close()?; }
        if let Some(s) = &self.server_socket { s.close()?; }
        self.socket = None; self.input_stream = None; self.output_stream = None;
        self.server_socket = None; self.state = ConnectionState::Disconnected; self.device = None;
        Ok(())
    }
}

pub struct BluetoothManager {
    adapter: Option<AndroidBluetoothAdapter>, connection: Arc<Mutex<BluetoothConnection>>, paired_devices: Vec<BluetoothDevice>,
}

impl BluetoothManager {
    pub fn new() -> Result<Self, String> {
        let adapter = match AndroidBluetoothAdapter::get_default() { Ok(a) => Some(a), Err(e) => { warn!("BT adapter not available: {}", e); None } };
        Ok(Self { adapter, connection: Arc::new(Mutex::new(BluetoothConnection::new())), paired_devices: Vec::new() })
    }
    pub fn is_enabled(&self) -> bool { self.adapter.as_ref().and_then(|a| a.is_enabled().ok()).unwrap_or(false) }
    pub fn enable(&self) -> Result<(), String> { self.adapter.as_ref().ok_or("No adapter")?.enable()?; Ok(()) }
    pub fn get_paired_devices(&mut self) -> Result<Vec<BluetoothDevice>, String> {
        let adapter = self.adapter.as_ref().ok_or("No adapter")?;
        let devices = adapter.get_bonded_devices()?;
        self.paired_devices = devices.iter().map(|d| BluetoothDevice {
            name: d.get_name().unwrap_or_else(|_| "Unknown".into()),
            address: d.get_address().unwrap_or_else(|_| "Unknown".into()), paired: true,
        }).collect();
        Ok(self.paired_devices.clone())
    }
    pub fn connect(&mut self, device_address: &str) -> Result<(), String> {
        let adapter = self.adapter.as_ref().ok_or("No adapter")?;
        let devices = adapter.get_bonded_devices()?;
        let target = devices.iter().find(|d| d.get_address().ok().map(|a| a == device_address).unwrap_or(false))
            .ok_or_else(|| format!("Device {} not found", device_address))?;
        let name = target.get_name().unwrap_or_else(|_| "Unknown".into());
        let socket = match target.create_rfcomm_socket(SERVICE_UUID) {
            Ok(s) => s,
            Err(e) => {
                warn!("Standard RFCOMM socket failed ({}), trying fallback", e);
                target.create_rfcomm_socket_fallback()?
            }
        };
        { self.connection.lock().unwrap().state = ConnectionState::Connecting; }
        let connection = Arc::clone(&self.connection);
        let addr = device_address.to_string();
        let fallback_socket = target.create_rfcomm_socket_fallback().ok();
        std::thread::spawn(move || {
            let try_connect = |sock: &AndroidBluetoothSocket| -> Result<(), String> {
                sock.connect()?;
                Ok(())
            };
            let (final_socket, connected) = match try_connect(&socket) {
                Ok(()) => (Some(socket), true),
                Err(e) => {
                    warn!("Primary RFCOMM connect failed: {}", e);
                    let _ = socket.close();
                    if let Some(fb) = fallback_socket {
                        match try_connect(&fb) {
                            Ok(()) => { info!("Fallback RFCOMM connect succeeded"); (Some(fb), true) }
                            Err(e2) => { error!("Fallback RFCOMM also failed: {}", e2); let _ = fb.close(); (None, false) }
                        }
                    } else {
                        error!("No fallback socket available"); (None, false)
                    }
                }
            };
            if connected {
                if let Some(sock) = final_socket {
                    match (sock.get_input_stream(), sock.get_output_stream()) {
                        (Ok(input_stream), Ok(output_stream)) => {
                            let mut conn = connection.lock().unwrap();
                            conn.input_stream = Some(input_stream);
                            conn.output_stream = Some(output_stream);
                            conn.socket = Some(sock);
                            conn.state = ConnectionState::Connected;
                            conn.device = Some(BluetoothDevice { address: addr, name, paired: true });
                            info!("Bluetooth connected");
                        }
                        _ => {
                            let mut conn = connection.lock().unwrap();
                            conn.state = ConnectionState::Disconnected;
                            error!("Failed to get streams after connect");
                        }
                    }
                }
            } else {
                let mut conn = connection.lock().unwrap();
                conn.state = ConnectionState::Disconnected;
            }
        });
        Ok(())
    }
    pub fn listen(&mut self) -> Result<(), String> {
        let adapter = self.adapter.as_ref().ok_or("No adapter")?;
        { self.connection.lock().unwrap().state = ConnectionState::Listening; }
        let server_socket = adapter.create_rfcomm_server("SassyTalkie", SERVICE_UUID)?;
        { self.connection.lock().unwrap().server_socket = Some(server_socket); }
        self.spawn_accept_thread();
        Ok(())
    }
    fn spawn_accept_thread(&self) {
        let connection = Arc::clone(&self.connection);
        std::thread::spawn(move || {
            loop {
                let should_accept = { let c = connection.lock().unwrap(); c.state == ConnectionState::Listening && c.server_socket.is_some() };
                if !should_accept { break; }
                let accept_result = { let c = connection.lock().unwrap(); c.server_socket.as_ref().map(|s| s.accept()) };
                match accept_result {
                    Some(Ok(socket)) => {
                        match (socket.get_input_stream(), socket.get_output_stream()) {
                            (Ok(input), Ok(output)) => {
                                let mut c = connection.lock().unwrap();
                                c.socket = Some(socket); c.input_stream = Some(input); c.output_stream = Some(output);
                                c.state = ConnectionState::Connected;
                                c.device = Some(BluetoothDevice { address: "Remote".into(), name: "Incoming".into(), paired: false });
                                break;
                            }
                            _ => continue,
                        }
                    }
                    Some(Err(_)) => { std::thread::sleep(std::time::Duration::from_secs(1)); continue; }
                    None => break,
                }
            }
        });
    }
    pub fn disconnect(&mut self) -> Result<(), String> { self.connection.lock().unwrap().close() }
    pub fn send_audio(&self, data: &[u8]) -> Result<usize, String> { self.connection.lock().unwrap().send(data) }
    pub fn receive_audio(&self, buffer: &mut [u8]) -> Result<usize, String> { self.connection.lock().unwrap().receive(buffer) }
    pub fn get_state(&self) -> ConnectionState { self.connection.lock().unwrap().state }
    pub fn get_connected_device(&self) -> Option<BluetoothDevice> { self.connection.lock().unwrap().device.clone() }
}
