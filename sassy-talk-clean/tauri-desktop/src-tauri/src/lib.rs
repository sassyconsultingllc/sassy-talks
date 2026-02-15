// Sassy-Talk Core Library
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

//! # Sassy-Talk
//!
//! Cross-platform PTT walkie-talkie with retro vibes.
//!
//! ## Supported Platforms
//! - Windows (10+)
//! - macOS (11.0+)
//! - Linux (Ubuntu 22.04+)
//!
//! ## Architecture
//! ```text
//! [Mic] → [CPAL] → [Opus] → [AES-GCM] → [UDP Multicast] → [Decrypt] → [Opus] → [Speaker]
//! ```
//!
//! ## Transport Strategy
//! - WiFi UDP Multicast: Primary transport (works everywhere)
//! - Auto-discovery via multicast beacons
//! - No pairing required

pub mod audio;
pub mod codec;
pub mod commands;
pub mod constants;
pub mod protocol;
pub mod security;
pub mod tones;
pub mod transport;

// Re-exports
pub use audio::{AudioEngine, AudioDeviceInfo, AudioState};
pub use codec::{OpusEncoder, OpusDecoder, AudioFrame, SAMPLE_RATE, FRAME_SIZE};
pub use protocol::{Packet, PacketType};
pub use security::CryptoEngine;
pub use tones::{TonePlayer, ToneType, ToneError};
pub use transport::{TransportManager, PeerInfo, TransportConfig};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use thiserror::Error;

// Use centralized version constant
pub use constants::VERSION;

/// Application error types
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Audio error: {0}")]
    AudioError(#[from] audio::AudioError),
    
    #[error("Codec error: {0}")]
    CodecError(#[from] codec::CodecError),
    
    #[error("Transport error: {0}")]
    TransportError(#[from] transport::TransportError),
    
    #[error("Already transmitting")]
    AlreadyTransmitting,
    
    #[error("Not connected")]
    NotConnected,
    
    #[error("Not transmitting")]
    NotTransmitting,
}

/// Connection status
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub enum ConnectionStatus {
    Disconnected,
    Discovering,
    Connected,
    Transmitting,
    Receiving,
}

/// Application state
pub struct AppState {
    // Device info
    device_id: u32,
    device_name: String,
    
    // Core engines
    audio: Arc<Mutex<AudioEngine>>,
    transport: Arc<Mutex<TransportManager>>,
    tone_player: Arc<TonePlayer>,
    
    // Channel
    current_channel: Arc<AtomicU8>,
    
    // Status
    connection_status: Arc<RwLock<ConnectionStatus>>,
    is_transmitting: Arc<AtomicBool>,
    is_receiving: Arc<AtomicBool>,
    
    // PTT threads
    tx_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
    rx_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
    
    // Settings
    roger_beep: Arc<AtomicBool>,
    vox_enabled: Arc<AtomicBool>,
    vox_threshold: Arc<RwLock<f32>>,
}

impl AppState {
    /// Create new application state
    pub fn new(device_id: u32, device_name: String) -> Self {
        info!("Creating AppState");
        info!("Device ID: {:08X}", device_id);
        info!("Device Name: {}", device_name);
        
        let audio = Arc::new(Mutex::new(
            AudioEngine::new().expect("Failed to create audio engine")
        ));
        
        let transport = Arc::new(Mutex::new(
            TransportManager::new(device_id, device_name.clone())
                .expect("Failed to create transport manager")
        ));
        
        Self {
            device_id,
            device_name,
            audio,
            transport,
            tone_player: Arc::new(TonePlayer::new()),
            current_channel: Arc::new(AtomicU8::new(1)),
            connection_status: Arc::new(RwLock::new(ConnectionStatus::Disconnected)),
            is_transmitting: Arc::new(AtomicBool::new(false)),
            is_receiving: Arc::new(AtomicBool::new(false)),
            tx_thread: Arc::new(Mutex::new(None)),
            rx_thread: Arc::new(Mutex::new(None)),
            roger_beep: Arc::new(AtomicBool::new(true)),
            vox_enabled: Arc::new(AtomicBool::new(false)),
            vox_threshold: Arc::new(RwLock::new(0.1)),
        }
    }
    
    /// Start discovery and receiving
    pub async fn start_discovery(&self) -> Result<(), AppError> {
        info!("Starting discovery");
        
        // Start transport
        let transport = self.transport.lock().await;
        transport.start().await?;
        
        *self.connection_status.write().await = ConnectionStatus::Discovering;
        
        // Start RX thread
        self.start_rx_thread().await?;
        
        *self.connection_status.write().await = ConnectionStatus::Connected;
        
        // Play connection success tone (3-tone chime) - use spawn_blocking since Stream is !Send
        let tone_player = Arc::clone(&self.tone_player);
        tokio::task::spawn_blocking(move || {
            if let Err(e) = tone_player.play_sync(ToneType::ConnectionSuccess) {
                warn!("Failed to play connection tone: {}", e);
            }
        });
        
        Ok(())
    }
    
    /// Stop discovery
    pub async fn stop_discovery(&self) -> Result<(), AppError> {
        info!("Stopping discovery");
        
        // Stop transport
        let transport = self.transport.lock().await;
        transport.stop();
        
        // Stop RX thread
        self.stop_rx_thread().await;
        
        *self.connection_status.write().await = ConnectionStatus::Disconnected;
        
        Ok(())
    }
    
    /// Start transmitting (PTT press)
    pub async fn start_transmit(&self) -> Result<(), AppError> {
        if self.is_transmitting.load(Ordering::Relaxed) {
            return Err(AppError::AlreadyTransmitting);
        }
        
        info!("Starting transmission");
        
        self.is_transmitting.store(true, Ordering::Relaxed);
        *self.connection_status.write().await = ConnectionStatus::Transmitting;
        
        // Start audio recording
        let audio = self.audio.lock().await;
        audio.start_recording()?;
        drop(audio);
        
        // Start TX thread
        self.start_tx_thread().await;
        
        Ok(())
    }
    
    /// Stop transmitting (PTT release)
    pub async fn stop_transmit(&self) -> Result<(), AppError> {
        if !self.is_transmitting.load(Ordering::Relaxed) {
            return Err(AppError::NotTransmitting);
        }
        
        info!("Stopping transmission");
        
        self.is_transmitting.store(false, Ordering::Relaxed);
        
        // Stop TX thread
        self.stop_tx_thread().await;
        
        // Stop audio recording
        let audio = self.audio.lock().await;
        audio.stop_recording()?;
        drop(audio);
        
        // Send roger beep if enabled (network + local)
        if self.roger_beep.load(Ordering::Relaxed) {
            // Send over network to peers
            self.send_roger_beep().await;
            
            // Play locally - use spawn_blocking since Stream is !Send
            let tone_player = Arc::clone(&self.tone_player);
            tokio::task::spawn_blocking(move || {
                if let Err(e) = tone_player.play_sync(ToneType::RogerBeep) {
                    warn!("Failed to play local roger beep: {}", e);
                }
            });
        }
        
        *self.connection_status.write().await = ConnectionStatus::Connected;
        
        Ok(())
    }
    
    /// Start TX thread (recording and encoding)
    async fn start_tx_thread(&self) {
        let audio = Arc::clone(&self.audio);
        let transport = Arc::clone(&self.transport);
        let is_transmitting = Arc::clone(&self.is_transmitting);
        let channel = self.current_channel.load(Ordering::Relaxed);
        
        let handle = tokio::spawn(async move {
            let mut encoder = match OpusEncoder::new() {
                Ok(e) => e,
                Err(e) => {
                    error!("Failed to create encoder: {}", e);
                    return;
                }
            };
            
            let mut buffer = vec![0i16; FRAME_SIZE];
            
            while is_transmitting.load(Ordering::Relaxed) {
                // Read audio samples
                let audio_lock = audio.lock().await;
                let samples_read = audio_lock.read_samples(&mut buffer);
                drop(audio_lock);
                
                if samples_read == FRAME_SIZE {
                    // Encode to Opus
                    match encoder.encode(&buffer) {
                        Ok(opus_data) => {
                            // Send via transport
                            let transport_lock = transport.lock().await;
                            if let Err(e) = transport_lock.send_audio(&opus_data) {
                                error!("Failed to send audio: {}", e);
                            }
                            drop(transport_lock);
                        }
                        Err(e) => {
                            error!("Encoding error: {}", e);
                        }
                    }
                } else {
                    // Not enough samples yet, wait briefly
                    tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                }
            }
            
            info!("TX thread stopped");
        });
        
        *self.tx_thread.lock().await = Some(handle);
    }
    
    /// Stop TX thread
    async fn stop_tx_thread(&self) {
        if let Some(handle) = self.tx_thread.lock().await.take() {
            handle.abort();
        }
    }
    
    /// Start RX thread (receiving and decoding)
    async fn start_rx_thread(&self) -> Result<(), AppError> {
        let audio = Arc::clone(&self.audio);
        let transport = Arc::clone(&self.transport);
        let is_receiving = Arc::clone(&self.is_receiving);
        let is_transmitting = Arc::clone(&self.is_transmitting);
        
        // Get audio receiver
        let mut audio_rx = {
            let transport_lock = transport.lock().await;
            transport_lock.take_audio_receiver()
                .ok_or(AppError::NotConnected)?
        };
        
        let handle = tokio::spawn(async move {
            let mut decoder = match OpusDecoder::new() {
                Ok(d) => d,
                Err(e) => {
                    error!("Failed to create decoder: {}", e);
                    return;
                }
            };
            
            // Start audio playback
            {
                let audio_lock = audio.lock().await;
                if let Err(e) = audio_lock.start_playing() {
                    error!("Failed to start playback: {}", e);
                    return;
                }
            }
            
            while let Some(opus_data) = audio_rx.recv().await {
                // Don't play audio while transmitting
                if is_transmitting.load(Ordering::Relaxed) {
                    continue;
                }
                
                is_receiving.store(true, Ordering::Relaxed);
                
                // Decode Opus to PCM
                match decoder.decode(&opus_data) {
                    Ok(pcm_samples) => {
                        // Write to audio output
                        let audio_lock = audio.lock().await;
                        audio_lock.write_samples(&pcm_samples);
                        drop(audio_lock);
                    }
                    Err(e) => {
                        error!("Decoding error: {}", e);
                        // Use packet loss concealment
                        if let Ok(plc_samples) = decoder.decode_plc() {
                            let audio_lock = audio.lock().await;
                            audio_lock.write_samples(&plc_samples);
                            drop(audio_lock);
                        }
                    }
                }
                
                is_receiving.store(false, Ordering::Relaxed);
            }
            
            // Stop audio playback
            {
                let audio_lock = audio.lock().await;
                let _ = audio_lock.stop_playing();
            }
            
            info!("RX thread stopped");
        });
        
        *self.rx_thread.lock().await = Some(handle);
        
        Ok(())
    }
    
    /// Stop RX thread
    async fn stop_rx_thread(&self) {
        if let Some(handle) = self.rx_thread.lock().await.take() {
            handle.abort();
        }
    }
    
    /// Get nearby devices
    pub async fn get_nearby_devices(&self) -> Vec<PeerInfo> {
        let transport = self.transport.lock().await;
        transport.get_peers()
    }
    
    /// Get connection status
    pub async fn get_connection_status(&self) -> ConnectionStatus {
        *self.connection_status.read().await
    }
    
    /// Get current channel
    pub fn get_channel(&self) -> u8 {
        self.current_channel.load(Ordering::Relaxed)
    }
    
    /// Set channel
    pub async fn set_channel(&self, channel: u8) {
        self.current_channel.store(channel, Ordering::Relaxed);
        
        let transport = self.transport.lock().await;
        transport.set_channel(channel);
    }
    
    /// Get audio devices
    pub async fn get_audio_devices(&self) -> (Vec<AudioDeviceInfo>, Vec<AudioDeviceInfo>) {
        let audio = self.audio.lock().await;
        let inputs = audio.get_input_devices();
        let outputs = audio.get_output_devices();
        (inputs, outputs)
    }
    
    /// Set input device
    pub async fn set_input_device(&self, device_name: &str) -> Result<(), AppError> {
        let mut audio = self.audio.lock().await;
        audio.set_input_device(device_name)?;
        Ok(())
    }
    
    /// Set output device
    pub async fn set_output_device(&self, device_name: &str) -> Result<(), AppError> {
        let mut audio = self.audio.lock().await;
        audio.set_output_device(device_name)?;
        Ok(())
    }
    
    /// Get volume
    pub async fn get_volume(&self) -> (f32, f32) {
        let audio = self.audio.lock().await;
        (audio.get_input_volume(), audio.get_output_volume())
    }
    
    /// Set volume
    pub async fn set_volume(&self, input: f32, output: f32) {
        let audio = self.audio.lock().await;
        audio.set_input_volume(input);
        audio.set_output_volume(output);
    }
    
    /// Set roger beep
    pub fn set_roger_beep(&self, enabled: bool) {
        self.roger_beep.store(enabled, Ordering::Relaxed);
    }
    
    /// Set VOX enabled
    pub fn set_vox_enabled(&self, enabled: bool) {
        self.vox_enabled.store(enabled, Ordering::Relaxed);
    }
    
    /// Set VOX threshold
    pub async fn set_vox_threshold(&self, threshold: f32) {
        *self.vox_threshold.write().await = threshold;
    }
    
    /// Send roger beep tone
    async fn send_roger_beep(&self) {
        // Generate a classic two-tone beep (800Hz + 1000Hz, 100ms total)
        let mut encoder = match OpusEncoder::new() {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to create encoder for roger beep: {}", e);
                return;
            }
        };
        
        // Generate 100ms of dual-tone beep (about 5 frames at 20ms each)
        let frames_to_send = 5;
        let mut samples = vec![0i16; FRAME_SIZE];
        
        for frame_idx in 0..frames_to_send {
            // Generate dual-tone samples
            for (i, sample) in samples.iter_mut().enumerate() {
                let t = (frame_idx * FRAME_SIZE + i) as f32 / SAMPLE_RATE as f32;
                // 800Hz + 1000Hz dual tone with envelope
                let envelope = if frame_idx < 2 { 
                    (frame_idx as f32 * FRAME_SIZE as f32 + i as f32) / (2.0 * FRAME_SIZE as f32) 
                } else if frame_idx >= 3 {
                    1.0 - ((frame_idx as f32 - 3.0) * FRAME_SIZE as f32 + i as f32) / (2.0 * FRAME_SIZE as f32)
                } else { 
                    1.0 
                };
                let tone = (f32::sin(2.0 * std::f32::consts::PI * 800.0 * t) * 0.5
                         + f32::sin(2.0 * std::f32::consts::PI * 1000.0 * t) * 0.5)
                         * envelope * 8000.0;
                *sample = tone as i16;
            }
            
            // Encode and send
            if let Ok(opus_data) = encoder.encode(&samples) {
                let transport = self.transport.lock().await;
                if let Err(e) = transport.send_audio(&opus_data) {
                    warn!("Failed to send roger beep frame: {}", e);
                }
            }
            
            // Small delay between frames
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
        
        info!("Roger beep sent");
    }
    
    /// Get device info
    pub fn get_device_info(&self) -> (u32, String) {
        (self.device_id, self.device_name.clone())
    }
    
    /// Get tone player
    pub fn get_tone_player(&self) -> Arc<TonePlayer> {
        Arc::clone(&self.tone_player)
    }
    
    /// Get transport configuration
    pub async fn get_transport_config(&self) -> TransportConfig {
        let transport = self.transport.lock().await;
        transport.get_config()
    }
    
    /// Update transport configuration
    pub async fn set_transport_config(&self, config: TransportConfig) {
        let transport = self.transport.lock().await;
        transport.update_config(config);
    }
    
    /// Get current bound port
    pub async fn get_port(&self) -> u16 {
        let transport = self.transport.lock().await;
        transport.get_port()
    }
    
    /// Check if encryption is active
    pub async fn is_encrypted(&self) -> bool {
        let transport = self.transport.lock().await;
        transport.is_encrypted()
    }
    
    /// Get our public key (hex encoded)
    pub async fn get_public_key(&self) -> Option<String> {
        let transport = self.transport.lock().await;
        transport.get_public_key()
    }
}
