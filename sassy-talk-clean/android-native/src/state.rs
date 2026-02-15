/// State Machine - Coordinates Transport, Audio, and UI
///
/// Manages the lifecycle of connections, PTT events, and audio streams.
/// Uses TransportManager for unified Bluetooth + WiFi support with encryption.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::Duration;
use log::{error, info, warn};

use crate::transport::{TransportManager, ActiveTransport};
use crate::bluetooth::{BluetoothDevice, ConnectionState};
use crate::audio::{AudioEngine, AudioFrame, FRAME_SIZE};
use crate::crypto;

/// Application state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Initializing,
    Ready,
    Connecting,
    Connected,
    Transmitting,
    Receiving,
    Disconnecting,
    Error,
}

/// State machine for managing app lifecycle
pub struct StateMachine {
    state: Arc<Mutex<AppState>>,
    transport: Arc<Mutex<Option<TransportManager>>>,
    audio: Arc<Mutex<Option<AudioEngine>>>,

    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,

    tx_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    rx_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    discovery_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,

    running: Arc<AtomicBool>,
}

impl StateMachine {
    pub fn new(
        ptt_pressed: Arc<AtomicBool>,
        current_channel: Arc<AtomicU8>,
    ) -> Self {
        info!("Creating state machine");

        Self {
            state: Arc::new(Mutex::new(AppState::Initializing)),
            transport: Arc::new(Mutex::new(None)),
            audio: Arc::new(Mutex::new(None)),
            ptt_pressed,
            current_channel,
            tx_thread: Arc::new(Mutex::new(None)),
            rx_thread: Arc::new(Mutex::new(None)),
            discovery_thread: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Initialize transport and audio subsystems
    pub fn initialize(&self) -> Result<(), String> {
        info!("Initializing state machine");

        *self.state.lock().unwrap() = AppState::Initializing;

        // Initialize transport manager
        let mut tm = TransportManager::new("SassyTalkie")?;

        // Generate and set a session PSK for encryption
        // In production, this would come from key exchange during connection
        let psk = crypto::generate_psk();
        tm.set_psk(&psk);

        *self.transport.lock().unwrap() = Some(tm);

        // Initialize audio engine
        let audio = AudioEngine::new()?;
        *self.audio.lock().unwrap() = Some(audio);

        *self.state.lock().unwrap() = AppState::Ready;
        info!("State machine initialized (BT default, WiFi upgrade enabled)");

        Ok(())
    }

    /// Start listening for incoming Bluetooth connections
    pub fn start_listening(&self) -> Result<(), String> {
        info!("Starting server mode");

        *self.state.lock().unwrap() = AppState::Connecting;

        let mut tm = self.transport.lock().unwrap();
        if let Some(transport) = tm.as_mut() {
            transport.listen_bluetooth()?;
            drop(tm);

            // Start WiFi discovery in background
            self.start_discovery_thread();

            info!("Listening for connections (BT + WiFi discovery)");
            Ok(())
        } else {
            Err("Transport not initialized".to_string())
        }
    }

    /// Connect to a specific Bluetooth device
    pub fn connect_to_device(&self, device_address: &str) -> Result<(), String> {
        info!("Connecting to device: {}", device_address);

        *self.state.lock().unwrap() = AppState::Connecting;

        let mut tm = self.transport.lock().unwrap();
        if let Some(transport) = tm.as_mut() {
            transport.connect_bluetooth(device_address)?;
            drop(tm);

            *self.state.lock().unwrap() = AppState::Connected;

            // Start RX thread + WiFi discovery
            self.start_rx_thread();
            self.start_discovery_thread();

            info!("Connected to {} (WiFi upgrade scanning)", device_address);
            Ok(())
        } else {
            Err("Transport not initialized".to_string())
        }
    }

    /// Disconnect current connection
    pub fn disconnect(&self) -> Result<(), String> {
        info!("Disconnecting");

        *self.state.lock().unwrap() = AppState::Disconnecting;

        // Stop threads
        self.running.store(false, Ordering::Relaxed);

        if let Some(handle) = self.tx_thread.lock().unwrap().take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.rx_thread.lock().unwrap().take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.discovery_thread.lock().unwrap().take() {
            let _ = handle.join();
        }

        // Stop audio
        if let Some(audio) = self.audio.lock().unwrap().as_ref() {
            let _ = audio.stop_recording();
            let _ = audio.stop_playing();
        }

        // Disconnect transport
        let mut tm = self.transport.lock().unwrap();
        if let Some(transport) = tm.as_mut() {
            transport.disconnect()?;
        }

        *self.state.lock().unwrap() = AppState::Ready;
        info!("Disconnected");

        Ok(())
    }

    /// Handle PTT press event
    pub fn on_ptt_press(&self) -> Result<(), String> {
        let channel = self.current_channel.load(Ordering::Relaxed);
        info!("PTT pressed - Channel {}", channel);

        // Check if connected via any transport
        let tm = self.transport.lock().unwrap();
        if let Some(transport) = tm.as_ref() {
            if transport.active_transport() == ActiveTransport::None {
                return Err("Not connected".to_string());
            }
        } else {
            return Err("Transport not initialized".to_string());
        }
        drop(tm);

        *self.state.lock().unwrap() = AppState::Transmitting;

        // Initialize audio if needed
        let audio = self.audio.lock().unwrap();
        if let Some(audio_engine) = audio.as_ref() {
            if audio_engine.get_state() != crate::audio::AudioState::Recording {
                audio_engine.init_recorder()?;
            }
        }
        drop(audio);

        self.start_recording()?;
        self.start_tx_thread();

        info!("Transmitting via {:?}", self.get_active_transport());
        Ok(())
    }

    /// Handle PTT release event
    pub fn on_ptt_release(&self) -> Result<(), String> {
        info!("PTT released");

        *self.state.lock().unwrap() = AppState::Connected;
        self.stop_recording()?;

        info!("Transmission stopped");
        Ok(())
    }

    fn start_recording(&self) -> Result<(), String> {
        let audio = self.audio.lock().unwrap();
        if let Some(audio_engine) = audio.as_ref() {
            audio_engine.start_recording()?;
            Ok(())
        } else {
            Err("Audio not initialized".to_string())
        }
    }

    fn stop_recording(&self) -> Result<(), String> {
        let audio = self.audio.lock().unwrap();
        if let Some(audio_engine) = audio.as_ref() {
            audio_engine.stop_recording()?;
        }
        Ok(())
    }

    /// TX thread: reads mic audio, encrypts, sends via active transport
    fn start_tx_thread(&self) {
        info!("Starting TX thread");

        self.running.store(true, Ordering::Relaxed);

        let running = Arc::clone(&self.running);
        let audio = Arc::clone(&self.audio);
        let transport = Arc::clone(&self.transport);
        let current_channel = Arc::clone(&self.current_channel);

        let handle = thread::spawn(move || {
            let mut frame = AudioFrame::new(FRAME_SIZE);

            while running.load(Ordering::Relaxed) {
                let is_recording = audio.lock().unwrap()
                    .as_ref()
                    .map(|a| a.is_recording())
                    .unwrap_or(false);

                if !is_recording {
                    break;
                }

                // Read audio
                let bytes_read = match audio.lock().unwrap().as_ref() {
                    Some(a) => match a.read_audio(&mut frame.samples) {
                        Ok(n) => n,
                        Err(e) => {
                            if !e.contains("would block") {
                                error!("TX: Failed to read audio: {}", e);
                            }
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                    },
                    None => break,
                };

                if bytes_read == 0 {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }

                // Build packet: [channel:1][audio_data:N]
                let channel = current_channel.load(Ordering::Relaxed);
                let audio_bytes = frame.to_bytes();
                let mut packet = Vec::with_capacity(1 + audio_bytes.len());
                packet.push(channel);
                packet.extend_from_slice(&audio_bytes);

                // Send via transport (handles encryption internally)
                let mut tm = transport.lock().unwrap();
                if let Some(t) = tm.as_mut() {
                    if let Err(e) = t.send(&packet) {
                        error!("TX: Failed to send: {}", e);
                    }
                } else {
                    break;
                }
            }

            info!("TX thread stopped");
        });

        *self.tx_thread.lock().unwrap() = Some(handle);
    }

    /// RX thread: receives from transport, decrypts, plays audio
    fn start_rx_thread(&self) {
        info!("Starting RX thread");

        self.running.store(true, Ordering::Relaxed);

        let running = Arc::clone(&self.running);
        let audio = Arc::clone(&self.audio);
        let transport = Arc::clone(&self.transport);
        let state = Arc::clone(&self.state);

        let handle = thread::spawn(move || {
            // Buffer sized for encrypted packet + overhead
            let mut buffer = vec![0u8; (FRAME_SIZE * 2) + 128];

            // Initialize audio player
            if let Some(a) = audio.lock().unwrap().as_ref() {
                if let Err(e) = a.init_player() {
                    error!("RX: Failed to init player: {}", e);
                    return;
                }
            }

            while running.load(Ordering::Relaxed) {
                // Receive via transport (handles decryption internally)
                let bytes_received = {
                    let mut tm = transport.lock().unwrap();
                    match tm.as_mut() {
                        Some(t) => match t.receive(&mut buffer) {
                            Ok(n) => n,
                            Err(e) => {
                                if !e.contains("would block") {
                                    error!("RX: receive error: {}", e);
                                }
                                thread::sleep(Duration::from_millis(5));
                                continue;
                            }
                        },
                        None => break,
                    }
                };

                if bytes_received < 2 {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }

                // Parse: [channel:1][audio_data:N]
                let _channel = buffer[0];
                let audio_data = &buffer[1..bytes_received];

                let frame = match AudioFrame::from_bytes(audio_data) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("RX: Failed to parse audio frame: {}", e);
                        continue;
                    }
                };

                *state.lock().unwrap() = AppState::Receiving;

                // Start playback if not already playing
                let should_start = audio.lock().unwrap()
                    .as_ref()
                    .map(|a| !a.is_playing())
                    .unwrap_or(false);

                if should_start {
                    if let Some(a) = audio.lock().unwrap().as_ref() {
                        if let Err(e) = a.start_playing() {
                            error!("RX: Failed to start playback: {}", e);
                        }
                    }
                }

                // Play audio
                if let Some(a) = audio.lock().unwrap().as_ref() {
                    if let Err(e) = a.write_audio(&frame.samples) {
                        error!("RX: Failed to write audio: {}", e);
                    }
                }
            }

            // Stop playback
            if let Some(a) = audio.lock().unwrap().as_ref() {
                let _ = a.stop_playing();
            }

            info!("RX thread stopped");
        });

        *self.rx_thread.lock().unwrap() = Some(handle);
    }

    /// Discovery thread: periodic WiFi announcements + checks for upgrade
    fn start_discovery_thread(&self) {
        info!("Starting WiFi discovery thread");

        let running = Arc::clone(&self.running);
        let transport = Arc::clone(&self.transport);
        let current_channel = Arc::clone(&self.current_channel);

        // Set running if not already
        self.running.store(true, Ordering::Relaxed);

        let handle = thread::spawn(move || {
            let mut announce_counter = 0u32;

            while running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(2));

                let channel = current_channel.load(Ordering::Relaxed);

                let mut tm = transport.lock().unwrap();
                if let Some(t) = tm.as_mut() {
                    // Send announcement every ~10 seconds
                    announce_counter += 1;
                    if announce_counter % 5 == 0 {
                        t.announce_wifi(channel);
                    }

                    // Check for WiFi upgrade
                    t.check_wifi_upgrade();
                }
            }

            info!("Discovery thread stopped");
        });

        *self.discovery_thread.lock().unwrap() = Some(handle);
    }

    // ── Getters ──

    pub fn get_state(&self) -> AppState {
        *self.state.lock().unwrap()
    }

    pub fn get_connected_device(&self) -> Option<BluetoothDevice> {
        self.transport.lock().unwrap()
            .as_ref()
            .and_then(|t| t.get_connected_device())
    }

    pub fn get_paired_devices(&self) -> Result<Vec<BluetoothDevice>, String> {
        let mut tm = self.transport.lock().unwrap();
        if let Some(transport) = tm.as_mut() {
            transport.get_paired_devices()
        } else {
            Err("Transport not initialized".to_string())
        }
    }

    pub fn is_bluetooth_enabled(&self) -> bool {
        self.transport.lock().unwrap()
            .as_ref()
            .map(|t| t.is_bluetooth_enabled())
            .unwrap_or(false)
    }

    pub fn enable_bluetooth(&self) -> Result<(), String> {
        let tm = self.transport.lock().unwrap();
        if let Some(transport) = tm.as_ref() {
            transport.enable_bluetooth()
        } else {
            Err("Transport not initialized".to_string())
        }
    }

    pub fn get_active_transport(&self) -> ActiveTransport {
        self.transport.lock().unwrap()
            .as_ref()
            .map(|t| t.active_transport())
            .unwrap_or(ActiveTransport::None)
    }

    /// Shutdown state machine
    pub fn shutdown(&self) -> Result<(), String> {
        info!("Shutting down state machine");

        let current = self.get_state();
        if current == AppState::Connected
            || current == AppState::Transmitting
            || current == AppState::Receiving
        {
            self.disconnect()?;
        }

        if let Some(audio) = self.audio.lock().unwrap().as_ref() {
            audio.release()?;
        }

        *self.state.lock().unwrap() = AppState::Ready;
        info!("State machine shutdown complete");

        Ok(())
    }
}

impl Drop for StateMachine {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU8};

    #[test]
    fn test_state_machine_creation() {
        let ptt = Arc::new(AtomicBool::new(false));
        let channel = Arc::new(AtomicU8::new(1));
        let _sm = StateMachine::new(ptt, channel);
    }
}
