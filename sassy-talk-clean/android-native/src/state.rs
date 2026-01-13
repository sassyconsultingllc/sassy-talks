/// State Machine - Coordinates Bluetooth, Audio, and UI
/// 
/// Manages the lifecycle of connections, PTT events, and audio streams

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::Duration;
use log::{error, info, warn};

use crate::bluetooth::{BluetoothManager, BluetoothDevice, ConnectionState};
use crate::audio::{AudioEngine, AudioFrame, FRAME_SIZE};

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
    bluetooth: Arc<Mutex<Option<BluetoothManager>>>,
    audio: Arc<Mutex<Option<AudioEngine>>>,
    
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    
    tx_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    rx_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    
    running: Arc<AtomicBool>,
}

impl StateMachine {
    /// Create new state machine
    pub fn new(
        ptt_pressed: Arc<AtomicBool>,
        current_channel: Arc<AtomicU8>,
    ) -> Self {
        info!("Creating state machine");
        
        Self {
            state: Arc::new(Mutex::new(AppState::Initializing)),
            bluetooth: Arc::new(Mutex::new(None)),
            audio: Arc::new(Mutex::new(None)),
            ptt_pressed,
            current_channel,
            tx_thread: Arc::new(Mutex::new(None)),
            rx_thread: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Initialize Bluetooth and Audio
    pub fn initialize(&self) -> Result<(), String> {
        info!("Initializing state machine");
        
        *self.state.lock().unwrap() = AppState::Initializing;
        
        // Initialize Bluetooth
        let bt = BluetoothManager::new()?;
        *self.bluetooth.lock().unwrap() = Some(bt);
        
        // Initialize Audio
        let audio = AudioEngine::new()?;
        *self.audio.lock().unwrap() = Some(audio);
        
        *self.state.lock().unwrap() = AppState::Ready;
        info!("✓ State machine initialized");
        
        Ok(())
    }

    /// Start listening for incoming connections
    pub fn start_listening(&self) -> Result<(), String> {
        info!("Starting server mode");
        
        *self.state.lock().unwrap() = AppState::Connecting;
        
        let bt = self.bluetooth.lock().unwrap();
        if let Some(bluetooth) = bt.as_ref() {
            drop(bt); // Release lock before calling listen
            
            // Start listening for connections
            let bt_clone = self.bluetooth.clone();
            if let Some(bluetooth) = bt_clone.lock().unwrap().as_mut() {
                bluetooth.listen()?;
            }
            
            info!("✓ Listening for connections");
            Ok(())
        } else {
            Err("Bluetooth not initialized".to_string())
        }
    }

    /// Connect to a specific device
    pub fn connect_to_device(&self, device_address: &str) -> Result<(), String> {
        info!("Connecting to device: {}", device_address);
        
        *self.state.lock().unwrap() = AppState::Connecting;
        
        let mut bt = self.bluetooth.lock().unwrap();
        if let Some(bluetooth) = bt.as_mut() {
            bluetooth.connect(device_address)?;
            drop(bt); // Release lock
            
            *self.state.lock().unwrap() = AppState::Connected;
            
            // Start RX thread
            self.start_rx_thread();
            
            info!("✓ Connected to device");
            Ok(())
        } else {
            Err("Bluetooth not initialized".to_string())
        }
    }

    /// Disconnect current connection
    pub fn disconnect(&self) -> Result<(), String> {
        info!("Disconnecting");
        
        *self.state.lock().unwrap() = AppState::Disconnecting;
        
        // Stop threads
        self.running.store(false, Ordering::Relaxed);
        
        // Wait for threads to finish
        if let Some(handle) = self.tx_thread.lock().unwrap().take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.rx_thread.lock().unwrap().take() {
            let _ = handle.join();
        }
        
        // Stop audio
        if let Some(audio) = self.audio.lock().unwrap().as_ref() {
            let _ = audio.stop_recording();
            let _ = audio.stop_playing();
        }
        
        // Disconnect Bluetooth
        let mut bt = self.bluetooth.lock().unwrap();
        if let Some(bluetooth) = bt.as_mut() {
            bluetooth.disconnect()?;
        }
        
        *self.state.lock().unwrap() = AppState::Ready;
        info!("✓ Disconnected");
        
        Ok(())
    }

    /// Handle PTT press event
    pub fn on_ptt_press(&self) -> Result<(), String> {
        let channel = self.current_channel.load(Ordering::Relaxed);
        info!("PTT pressed - Channel {}", channel);
        
        // Check if connected
        let bt = self.bluetooth.lock().unwrap();
        if let Some(bluetooth) = bt.as_ref() {
            if bluetooth.get_state() != ConnectionState::Connected {
                warn!("Not connected - cannot transmit");
                return Err("Not connected".to_string());
            }
        } else {
            return Err("Bluetooth not initialized".to_string());
        }
        drop(bt);
        
        *self.state.lock().unwrap() = AppState::Transmitting;
        
        // Initialize audio if needed
        let audio = self.audio.lock().unwrap();
        if let Some(audio_engine) = audio.as_ref() {
            if audio_engine.get_state() != crate::audio::AudioState::Recording {
                audio_engine.init_recorder()?;
            }
        }
        drop(audio);
        
        // Start recording and TX thread
        self.start_recording()?;
        self.start_tx_thread();
        
        info!("✓ Transmitting");
        Ok(())
    }

    /// Handle PTT release event
    pub fn on_ptt_release(&self) -> Result<(), String> {
        info!("PTT released");
        
        *self.state.lock().unwrap() = AppState::Connected;
        
        // Stop recording
        self.stop_recording()?;
        
        // TX thread will stop automatically when recording stops
        
        info!("✓ Transmission stopped");
        Ok(())
    }

    /// Start recording audio
    fn start_recording(&self) -> Result<(), String> {
        let audio = self.audio.lock().unwrap();
        if let Some(audio_engine) = audio.as_ref() {
            audio_engine.start_recording()?;
            Ok(())
        } else {
            Err("Audio not initialized".to_string())
        }
    }

    /// Stop recording audio
    fn stop_recording(&self) -> Result<(), String> {
        let audio = self.audio.lock().unwrap();
        if let Some(audio_engine) = audio.as_ref() {
            audio_engine.stop_recording()?;
            Ok(())
        } else {
            Ok(())
        }
    }

    /// Start TX thread (transmit recorded audio)
    fn start_tx_thread(&self) {
        info!("Starting TX thread");
        
        self.running.store(true, Ordering::Relaxed);
        
        let running = Arc::clone(&self.running);
        let audio = Arc::clone(&self.audio);
        let bluetooth = Arc::clone(&self.bluetooth);
        let current_channel = Arc::clone(&self.current_channel);
        
        let handle = thread::spawn(move || {
            let mut frame = AudioFrame::new(FRAME_SIZE);
            
            while running.load(Ordering::Relaxed) {
                // Check if still recording
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
                                error!("Failed to read audio: {}", e);
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
                
                // Prepare packet: [channel_id][audio_data]
                let channel = current_channel.load(Ordering::Relaxed);
                let audio_bytes = frame.to_bytes();
                let mut packet = vec![channel];
                packet.extend_from_slice(&audio_bytes);
                
                // Send via Bluetooth
                match bluetooth.lock().unwrap().as_ref() {
                    Some(bt) => {
                        if let Err(e) = bt.send_audio(&packet) {
                            error!("Failed to send audio: {}", e);
                        }
                    },
                    None => break,
                }
            }
            
            info!("TX thread stopped");
        });
        
        *self.tx_thread.lock().unwrap() = Some(handle);
    }

    /// Start RX thread (receive and play audio)
    fn start_rx_thread(&self) {
        info!("Starting RX thread");
        
        self.running.store(true, Ordering::Relaxed);
        
        let running = Arc::clone(&self.running);
        let audio = Arc::clone(&self.audio);
        let bluetooth = Arc::clone(&self.bluetooth);
        let state = Arc::clone(&self.state);
        
        let handle = thread::spawn(move || {
            let mut buffer = vec![0u8; (FRAME_SIZE * 2) + 1]; // +1 for channel byte
            
            // Initialize audio player
            if let Some(a) = audio.lock().unwrap().as_ref() {
                if let Err(e) = a.init_player() {
                    error!("Failed to init player: {}", e);
                    return;
                }
            }
            
            while running.load(Ordering::Relaxed) {
                // Check if still connected
                let is_connected = bluetooth.lock().unwrap()
                    .as_ref()
                    .map(|bt| bt.get_state() == ConnectionState::Connected)
                    .unwrap_or(false);
                
                if !is_connected {
                    break;
                }
                
                // Receive data
                let bytes_received = match bluetooth.lock().unwrap().as_ref() {
                    Some(bt) => match bt.receive_audio(&mut buffer) {
                        Ok(n) => n,
                        Err(e) => {
                            if !e.contains("would block") {
                                error!("Failed to receive audio: {}", e);
                            }
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                    },
                    None => break,
                };
                
                if bytes_received < 2 {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                
                // Parse packet: [channel_id][audio_data]
                let _channel = buffer[0];
                let audio_data = &buffer[1..bytes_received];
                
                // Convert to audio frame
                let frame = match AudioFrame::from_bytes(audio_data) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("Failed to parse audio frame: {}", e);
                        continue;
                    }
                };
                
                // Update state to receiving
                *state.lock().unwrap() = AppState::Receiving;
                
                // Start playback if not already playing
                let should_start_playback = audio.lock().unwrap()
                    .as_ref()
                    .map(|a| !a.is_playing())
                    .unwrap_or(false);
                
                if should_start_playback {
                    if let Some(a) = audio.lock().unwrap().as_ref() {
                        if let Err(e) = a.start_playing() {
                            error!("Failed to start playback: {}", e);
                        }
                    }
                }
                
                // Play audio
                match audio.lock().unwrap().as_ref() {
                    Some(a) => {
                        if let Err(e) = a.write_audio(&frame.samples) {
                            error!("Failed to write audio: {}", e);
                        }
                    },
                    None => break,
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

    /// Get current app state
    pub fn get_state(&self) -> AppState {
        *self.state.lock().unwrap()
    }

    /// Get connected device info
    pub fn get_connected_device(&self) -> Option<BluetoothDevice> {
        self.bluetooth.lock().unwrap()
            .as_ref()
            .and_then(|bt| bt.get_connected_device())
    }

    /// Get paired devices list
    pub fn get_paired_devices(&self) -> Result<Vec<BluetoothDevice>, String> {
        let mut bt = self.bluetooth.lock().unwrap();
        if let Some(bluetooth) = bt.as_mut() {
            bluetooth.get_paired_devices()
        } else {
            Err("Bluetooth not initialized".to_string())
        }
    }

    /// Check if Bluetooth is enabled
    pub fn is_bluetooth_enabled(&self) -> bool {
        self.bluetooth.lock().unwrap()
            .as_ref()
            .map(|bt| bt.is_enabled())
            .unwrap_or(false)
    }

    /// Enable Bluetooth
    pub fn enable_bluetooth(&self) -> Result<(), String> {
        let bt = self.bluetooth.lock().unwrap();
        if let Some(bluetooth) = bt.as_ref() {
            bluetooth.enable()?;
            Ok(())
        } else {
            Err("Bluetooth not initialized".to_string())
        }
    }

    /// Shutdown state machine
    pub fn shutdown(&self) -> Result<(), String> {
        info!("Shutting down state machine");
        
        // Disconnect if connected
        if self.get_state() == AppState::Connected {
            self.disconnect()?;
        }
        
        // Release audio resources
        if let Some(audio) = self.audio.lock().unwrap().as_ref() {
            audio.release()?;
        }
        
        *self.state.lock().unwrap() = AppState::Ready;
        info!("✓ State machine shutdown");
        
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

    #[test]
    fn test_state_machine_creation() {
        let ptt = Arc::new(AtomicBool::new(false));
        let channel = Arc::new(AtomicU8::new(1));
        let _sm = StateMachine::new(ptt, channel);
    }
}
