/// State Machine for iOS
/// 
/// Coordinates audio, codec, and transport
/// Similar to Android version but adapted for iOS

use crate::audio::{AudioEngine, AudioFrame};
use crate::codec::{OpusEncoder, OpusDecoder};
use crate::protocol::{Packet, PacketType};
use crate::transport::{TransportManager, PeerInfo};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::Duration;
use thiserror::Error;
use log::{error, info, warn};

#[derive(Error, Debug)]
pub enum StateError {
    #[error("Audio error: {0}")]
    AudioError(String),
    
    #[error("Codec error: {0}")]
    CodecError(String),
    
    #[error("Transport error: {0}")]
    TransportError(String),
    
    #[error("Invalid state transition")]
    InvalidStateTransition,
    
    #[error("Not connected")]
    NotConnected,
    
    #[error("Already transmitting")]
    AlreadyTransmitting,
}

/// Application state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Idle,
    Connecting,
    Connected,
    Transmitting,
    Receiving,
    Error,
}

/// State machine
pub struct StateMachine {
    // Current state
    state: Arc<Mutex<AppState>>,
    
    // Device info
    device_id: u32,
    device_name: String,
    
    // Current channel
    current_channel: Arc<AtomicU8>,
    
    // Core components
    audio: Arc<Mutex<AudioEngine>>,
    encoder: Arc<Mutex<OpusEncoder>>,
    decoder: Arc<Mutex<OpusDecoder>>,
    transport: Arc<Mutex<TransportManager>>,
    
    // Control flags
    is_transmitting: Arc<AtomicBool>,
    should_stop_tx: Arc<AtomicBool>,
    should_stop_rx: Arc<AtomicBool>,
}

impl StateMachine {
    /// Create new state machine
    pub fn new() -> Result<Self, StateError> {
        let device_id = rand::random();
        let device_name = format!("iPhone-{}", device_id % 10000);
        
        let audio = AudioEngine::new();
        let encoder = OpusEncoder::new()
            .map_err(|e| StateError::CodecError(e.to_string()))?;
        let decoder = OpusDecoder::new()
            .map_err(|e| StateError::CodecError(e.to_string()))?;
        let transport = TransportManager::new()
            .map_err(|e| StateError::TransportError(e.to_string()))?;
        
        // Start transport
        transport.start()
            .map_err(|e| StateError::TransportError(e.to_string()))?;
        
        Ok(Self {
            state: Arc::new(Mutex::new(AppState::Idle)),
            device_id,
            device_name,
            current_channel: Arc::new(AtomicU8::new(1)),
            audio: Arc::new(Mutex::new(audio)),
            encoder: Arc::new(Mutex::new(encoder)),
            decoder: Arc::new(Mutex::new(decoder)),
            transport: Arc::new(Mutex::new(transport)),
            is_transmitting: Arc::new(AtomicBool::new(false)),
            should_stop_tx: Arc::new(AtomicBool::new(false)),
            should_stop_rx: Arc::new(AtomicBool::new(false)),
        })
    }
    
    /// Set channel
    pub fn set_channel(&self, channel: u8) {
        self.current_channel.store(channel, Ordering::SeqCst);
        info!("Channel set to {}", channel);
    }
    
    /// Get channel
    pub fn get_channel(&self) -> u8 {
        self.current_channel.load(Ordering::SeqCst)
    }
    
    /// Get current state
    pub fn current_state(&self) -> AppState {
        *self.state.lock().unwrap()
    }
    
    /// PTT press - start transmission
    pub fn on_ptt_press(&mut self) -> Result<(), StateError> {
        if self.is_transmitting.load(Ordering::SeqCst) {
            return Err(StateError::AlreadyTransmitting);
        }
        
        info!("PTT pressed - starting transmission");
        
        // Start recording
        self.audio.lock().unwrap().start_recording()
            .map_err(|e| StateError::AudioError(e.to_string()))?;
        
        // Set state
        *self.state.lock().unwrap() = AppState::Transmitting;
        self.is_transmitting.store(true, Ordering::SeqCst);
        self.should_stop_tx.store(false, Ordering::SeqCst);
        
        // Start TX thread
        self.start_tx_thread();
        
        Ok(())
    }
    
    /// PTT release - stop transmission
    pub fn on_ptt_release(&mut self) -> Result<(), StateError> {
        if !self.is_transmitting.load(Ordering::SeqCst) {
            return Ok(());
        }
        
        info!("PTT released - stopping transmission");
        
        // Signal stop
        self.should_stop_tx.store(true, Ordering::SeqCst);
        
        // Stop recording
        self.audio.lock().unwrap().stop_recording()
            .map_err(|e| StateError::AudioError(e.to_string()))?;
        
        // Update state
        *self.state.lock().unwrap() = AppState::Connected;
        self.is_transmitting.store(false, Ordering::SeqCst);
        
        Ok(())
    }
    
    /// Start TX thread
    fn start_tx_thread(&self) {
        let audio = Arc::clone(&self.audio);
        let encoder = Arc::clone(&self.encoder);
        let transport = Arc::clone(&self.transport);
        let should_stop = Arc::clone(&self.should_stop_tx);
        let channel = self.current_channel.load(Ordering::SeqCst);
        let device_id = self.device_id;
        
        thread::spawn(move || {
            info!("TX thread started");
            
            while !should_stop.load(Ordering::SeqCst) {
                // Read audio frame
                let frame = match audio.lock().unwrap().read_input_frame() {
                    Ok(f) => f,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                
                // Encode
                let encoded = match encoder.lock().unwrap().encode(&frame.samples) {
                    Ok(e) => e,
                    Err(e) => {
                        error!("Encode error: {}", e);
                        continue;
                    }
                };
                
                // Create packet
                let packet = Packet::audio(device_id, channel, encoded);
                
                // Send
                if let Ok(bytes) = packet.serialize() {
                    let _ = transport.lock().unwrap().send(&bytes);
                }
            }
            
            info!("TX thread stopped");
        });
    }
    
    /// Start listening for audio
    pub fn start_listening(&mut self) -> Result<(), StateError> {
        info!("Starting RX listener");
        
        self.audio.lock().unwrap().start_playing()
            .map_err(|e| StateError::AudioError(e.to_string()))?;
        
        self.should_stop_rx.store(false, Ordering::SeqCst);
        self.start_rx_thread();
        
        Ok(())
    }
    
    /// Start RX thread
    fn start_rx_thread(&self) {
        let audio = Arc::clone(&self.audio);
        let decoder = Arc::clone(&self.decoder);
        let transport = Arc::clone(&self.transport);
        let should_stop = Arc::clone(&self.should_stop_rx);
        let state = Arc::clone(&self.state);
        let current_channel = Arc::clone(&self.current_channel);
        
        thread::spawn(move || {
            info!("RX thread started");
            let mut buffer = vec![0u8; 2048];
            
            while !should_stop.load(Ordering::SeqCst) {
                // Receive packet
                let (size, _addr) = match transport.lock().unwrap().receive(&mut buffer) {
                    Ok(r) => r,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                
                // Parse packet
                let packet = match Packet::deserialize(&buffer[..size]) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("Failed to parse packet: {}", e);
                        continue;
                    }
                };
                
                // Handle audio packets
                if let PacketType::Audio { channel, data } = packet.packet_type {
                    if channel == current_channel.load(Ordering::SeqCst) {
                        // Decode
                        let samples = match decoder.lock().unwrap().decode(&data) {
                            Ok(s) => s,
                            Err(e) => {
                                error!("Decode error: {}", e);
                                continue;
                            }
                        };
                        
                        // Write to output
                        let frame = AudioFrame::new(samples);
                        let _ = audio.lock().unwrap().write_output_frame(&frame);
                        
                        // Update state
                        *state.lock().unwrap() = AppState::Receiving;
                    }
                }
            }
            
            info!("RX thread stopped");
        });
    }
    
    /// Connect to device
    pub fn connect_to_device(&mut self, _device_id: u32) -> Result<(), StateError> {
        info!("Connecting to device...");
        *self.state.lock().unwrap() = AppState::Connected;
        self.start_listening()?;
        Ok(())
    }
    
    /// Disconnect
    pub fn disconnect(&mut self) -> Result<(), StateError> {
        info!("Disconnecting...");
        self.should_stop_rx.store(true, Ordering::SeqCst);
        self.audio.lock().unwrap().stop_playing()
            .map_err(|e| StateError::AudioError(e.to_string()))?;
        *self.state.lock().unwrap() = AppState::Idle;
        Ok(())
    }
    
    /// Process audio input (called from Swift)
    pub fn process_audio_input(&mut self, samples: &[i16]) -> Result<(), StateError> {
        self.audio.lock().unwrap().write_input(samples)
            .map_err(|e| StateError::AudioError(e.to_string()))
    }
    
    /// Get audio output (called from Swift)
    pub fn get_audio_output(&mut self, buffer: &mut [i16]) -> Result<usize, StateError> {
        self.audio.lock().unwrap().read_output(buffer)
            .map_err(|e| StateError::AudioError(e.to_string()))
    }
    
    /// Shutdown
    pub fn shutdown(&mut self) -> Result<(), StateError> {
        info!("Shutting down state machine");
        self.should_stop_tx.store(true, Ordering::SeqCst);
        self.should_stop_rx.store(true, Ordering::SeqCst);
        self.transport.lock().unwrap().stop();
        Ok(())
    }
}
