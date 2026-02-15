use std::sync::{Arc, Mutex};
use log::{error, info, warn};
use crate::jni_bridge::{
    AndroidBluetoothAdapter,
    AndroidBluetoothDevice,
    AndroidBluetoothSocket,
    AndroidBluetoothServerSocket,
    AndroidInputStream,
    AndroidOutputStream,
};

/// UUID for SassyTalkie RFCOMM service
pub const SERVICE_UUID: &str = "8ce255c0-223a-11e0-ac64-0803450c9a66";

/// Bluetooth connection state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Listening,
}

/// Bluetooth device information
#[derive(Debug, Clone)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    pub paired: bool,
}

/// Bluetooth connection wrapper
pub struct BluetoothConnection {
    socket: Option<AndroidBluetoothSocket>,
    input_stream: Option<AndroidInputStream>,
    output_stream: Option<AndroidOutputStream>,
    server_socket: Option<AndroidBluetoothServerSocket>,
    state: ConnectionState,
    device: Option<BluetoothDevice>,
}

impl BluetoothConnection {
    pub fn new() -> Self {
        Self {
            socket: None,
            input_stream: None,
            output_stream: None,
            server_socket: None,
            state: ConnectionState::Disconnected,
            device: None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected
    }

    pub fn send(&mut self, data: &[u8]) -> Result<usize, String> {
        match &self.output_stream {
            Some(stream) => {
                stream.write(data)?;
                stream.flush()?;
                Ok(data.len())
            }
            None => Err("Output stream not available".to_string()),
        }
    }

    pub fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        match &self.input_stream {
            Some(stream) => stream.read(buffer),
            None => Err("Input stream not available".to_string()),
        }
    }

    pub fn close(&mut self) -> Result<(), String> {
        // Close streams and socket
        if let Some(socket) = &self.socket {
            socket.close()?;
        }
        if let Some(server) = &self.server_socket {
            server.close()?;
        }

        self.socket = None;
        self.input_stream = None;
        self.output_stream = None;
        self.server_socket = None;
        self.state = ConnectionState::Disconnected;
        self.device = None;

        Ok(())
    }
}

/// Bluetooth manager for handling connections
pub struct BluetoothManager {
    adapter: Option<AndroidBluetoothAdapter>,
    connection: Arc<Mutex<BluetoothConnection>>,
    paired_devices: Vec<BluetoothDevice>,
}

impl BluetoothManager {
    pub fn new() -> Result<Self, String> {
        info!("Initializing Bluetooth manager");
        
        // Get default Bluetooth adapter
        let adapter = match AndroidBluetoothAdapter::get_default() {
            Ok(a) => {
                info!("✓ Bluetooth adapter obtained");
                Some(a)
            }
            Err(e) => {
                warn!("Bluetooth adapter not available: {}", e);
                None
            }
        };

        Ok(Self {
            adapter,
            connection: Arc::new(Mutex::new(BluetoothConnection::new())),
            paired_devices: Vec::new(),
        })
    }

    /// Check if Bluetooth is enabled
    pub fn is_enabled(&self) -> bool {
        match &self.adapter {
            Some(adapter) => {
                match adapter.is_enabled() {
                    Ok(enabled) => enabled,
                    Err(e) => {
                        error!("Failed to check Bluetooth state: {}", e);
                        false
                    }
                }
            }
            None => {
                warn!("Bluetooth adapter not available");
                false
            }
        }
    }

    /// Enable Bluetooth
    pub fn enable(&self) -> Result<(), String> {
        match &self.adapter {
            Some(adapter) => {
                info!("Enabling Bluetooth...");
                adapter.enable()?;
                info!("✓ Bluetooth enable request sent");
                Ok(())
            }
            None => Err("Bluetooth adapter not available".to_string()),
        }
    }

    /// Scan for paired devices
    pub fn get_paired_devices(&mut self) -> Result<Vec<BluetoothDevice>, String> {
        match &self.adapter {
            Some(adapter) => {
                info!("Getting paired devices...");
                
                let devices = adapter.get_bonded_devices()?;
                
                self.paired_devices = devices.iter().map(|device| {
                    let name = device.get_name().unwrap_or_else(|_| "Unknown".to_string());
                    let address = device.get_address().unwrap_or_else(|_| "Unknown".to_string());
                    
                    BluetoothDevice {
                        address,
                        name,
                        paired: true,
                    }
                }).collect();

                info!("✓ Found {} paired device(s)", self.paired_devices.len());
                Ok(self.paired_devices.clone())
            }
            None => Err("Bluetooth adapter not available".to_string()),
        }
    }

    /// Connect to a specific device by address
    pub fn connect(&mut self, device_address: &str) -> Result<(), String> {
        info!("Connecting to device: {}", device_address);

        let adapter = self.adapter.as_ref()
            .ok_or("Bluetooth adapter not available")?;

        // Get bonded devices and find target
        let devices = adapter.get_bonded_devices()?;
        
        let target_device = devices.iter()
            .find(|d| {
                d.get_address().ok()
                    .map(|addr| addr == device_address)
                    .unwrap_or(false)
            })
            .ok_or_else(|| format!("Device {} not found in paired devices", device_address))?;

        let mut conn = self.connection.lock().unwrap();
        conn.state = ConnectionState::Connecting;
        drop(conn);

        // Create RFCOMM socket
        info!("Creating RFCOMM socket with UUID: {}", SERVICE_UUID);
        let socket = target_device.create_rfcomm_socket(SERVICE_UUID)?;

        // Connect to remote device
        info!("Connecting to remote device...");
        socket.connect()?;
        info!("✓ Socket connected");

        // Get I/O streams
        let input_stream = socket.get_input_stream()?;
        let output_stream = socket.get_output_stream()?;
        info!("✓ I/O streams obtained");

        // Update connection state
        let mut conn = self.connection.lock().unwrap();
        conn.socket = Some(socket);
        conn.input_stream = Some(input_stream);
        conn.output_stream = Some(output_stream);
        conn.state = ConnectionState::Connected;
        
        // Store device info
        let name = target_device.get_name().unwrap_or_else(|_| "Unknown".to_string());
        conn.device = Some(BluetoothDevice {
            address: device_address.to_string(),
            name,
            paired: true,
        });

        info!("✓ Connected successfully to {}", device_address);
        Ok(())
    }

    /// Start listening for incoming connections
    pub fn listen(&mut self) -> Result<(), String> {
        info!("Starting Bluetooth server");

        let adapter = self.adapter.as_ref()
            .ok_or("Bluetooth adapter not available")?;

        let mut conn = self.connection.lock().unwrap();
        conn.state = ConnectionState::Listening;
        drop(conn);

        // Create server socket
        info!("Creating RFCOMM server socket with UUID: {}", SERVICE_UUID);
        let server_socket = adapter.create_rfcomm_server("SassyTalkie", SERVICE_UUID)?;
        info!("✓ Server socket created");

        // Store server socket
        let mut conn = self.connection.lock().unwrap();
        conn.server_socket = Some(server_socket);
        drop(conn);

        info!("✓ Listening for incoming connections");
        
        // Spawn accept thread
        self.spawn_accept_thread();

        Ok(())
    }

    /// Spawn thread to accept incoming connections
    fn spawn_accept_thread(&self) {
        let connection = Arc::clone(&self.connection);
        
        std::thread::spawn(move || {
            info!("Accept thread started");
            
            loop {
                // Check state and get server socket under lock
                let should_accept = {
                    let conn = connection.lock().unwrap();
                    
                    // Check if still listening
                    if conn.state != ConnectionState::Listening {
                        info!("Accept thread exiting (not listening)");
                        false
                    } else {
                        conn.server_socket.is_some()
                    }
                };
                
                if !should_accept {
                    break;
                }
                
                // Accept incoming connection (blocking) - need to hold lock for accept
                info!("Waiting for incoming connection...");
                let accept_result = {
                    let conn = connection.lock().unwrap();
                    if let Some(ref server_socket) = conn.server_socket {
                        Some(server_socket.accept())
                    } else {
                        None
                    }
                };
                
                match accept_result {
                    Some(Ok(socket)) => {
                        info!("✓ Incoming connection accepted");
                        
                        // Get I/O streams
                        match (socket.get_input_stream(), socket.get_output_stream()) {
                            (Ok(input), Ok(output)) => {
                                // Update connection
                                let mut conn = connection.lock().unwrap();
                                conn.socket = Some(socket);
                                conn.input_stream = Some(input);
                                conn.output_stream = Some(output);
                                conn.state = ConnectionState::Connected;
                                conn.device = Some(BluetoothDevice {
                                    address: "Remote".to_string(),
                                    name: "Incoming Connection".to_string(),
                                    paired: false,
                                });
                                
                                info!("✓ Connection established with remote device");
                                break; // Exit accept loop
                            }
                            (Err(e), _) | (_, Err(e)) => {
                                error!("Failed to get I/O streams: {}", e);
                                continue;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!("Failed to accept connection: {}", e);
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                    None => {
                        error!("Server socket not available");
                        break;
                    }
                }
            }
            
            info!("Accept thread terminated");
        });
    }

    /// Disconnect current connection
    pub fn disconnect(&mut self) -> Result<(), String> {
        info!("Disconnecting");

        let mut conn = self.connection.lock().unwrap();
        conn.close()?;

        info!("✓ Disconnected");
        Ok(())
    }

    /// Send audio data over Bluetooth
    pub fn send_audio(&self, data: &[u8]) -> Result<usize, String> {
        let mut conn = self.connection.lock().unwrap();
        
        if !conn.is_connected() {
            return Err("Not connected".to_string());
        }

        match conn.send(data) {
            Ok(size) => Ok(size),
            Err(e) => {
                error!("Failed to send audio: {}", e);
                Err(e)
            }
        }
    }

    /// Receive audio data from Bluetooth
    pub fn receive_audio(&self, buffer: &mut [u8]) -> Result<usize, String> {
        let mut conn = self.connection.lock().unwrap();
        
        if !conn.is_connected() {
            return Err("Not connected".to_string());
        }

        match conn.receive(buffer) {
            Ok(size) => Ok(size),
            Err(e) => {
                // Don't log "would block" errors (normal for non-blocking I/O)
                if !e.contains("would block") {
                    error!("Failed to receive audio: {}", e);
                }
                Err(e)
            }
        }
    }

    /// Get current connection state
    pub fn get_state(&self) -> ConnectionState {
        self.connection.lock().unwrap().state
    }

    /// Get connected device info
    pub fn get_connected_device(&self) -> Option<BluetoothDevice> {
        self.connection.lock().unwrap().device.clone()
    }
}
