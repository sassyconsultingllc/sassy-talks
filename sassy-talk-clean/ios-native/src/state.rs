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

    // ── Crypto (shared core — parity with Android) ──
    // Active AEAD session: TX encrypts each Opus frame, RX decrypts. One session
    // serves both directions (group-PSK model: every peer derives the same key
    // via from_psk and draws its own random nonce prefix). None = not yet paired
    // (sends/accepts plaintext until a PSK or handshake installs a session).
    crypto: Arc<Mutex<Option<crate::crypto::CryptoSession>>>,
    // The QR pre-shared key, kept so the hybrid PQC handshake can mix it in.
    psk: Arc<Mutex<Option<[u8; 32]>>>,
    // Pending path-(a) hybrid handshake (initiator side), between init & complete.
    pending_hybrid: Arc<Mutex<Option<crate::pqc::PskHybridInitiator>>>,
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
            crypto: Arc::new(Mutex::new(None)),
            psk: Arc::new(Mutex::new(None)),
            pending_hybrid: Arc::new(Mutex::new(None)),
        })
    }

    // ── Crypto / key agreement (mirrors android-native's JNI crypto seam) ──

    /// Install a pre-shared key (the QR session key). Sets the active AEAD
    /// session and remembers the PSK so a later hybrid handshake can mix it in.
    pub fn set_psk(&self, key: &[u8; 32]) {
        *self.psk.lock().unwrap() = Some(*key);
        *self.crypto.lock().unwrap() = Some(crate::crypto::CryptoSession::from_psk(key));
        info!("Crypto: PSK session installed");
    }

    /// Replace the active AEAD session (e.g. with a key-exchange / hybrid result).
    pub fn set_crypto_session(&self, session: crate::crypto::CryptoSession) {
        *self.crypto.lock().unwrap() = Some(session);
    }

    /// This build's capability bitmap (hybrid-PQC support) — same value Android
    /// advertises in its heartbeat.
    pub fn local_capabilities(&self) -> u8 {
        crate::pqc::local_capabilities()
    }

    /// Initiator: begin a path-(a) PSK-authenticated hybrid handshake. Returns the
    /// initiator message bytes to send, or None if no PSK is installed.
    pub fn hybrid_init(&self) -> Option<Vec<u8>> {
        let psk = (*self.psk.lock().unwrap())?;
        let initiator = crate::pqc::PskHybridInitiator::new(&psk);
        let msg = initiator.initiator_message().to_bytes();
        *self.pending_hybrid.lock().unwrap() = Some(initiator);
        Some(msg)
    }

    /// Responder: given the peer's initiator message, install the session and
    /// return the responder message bytes to send back. None on failure.
    pub fn hybrid_respond(&self, init_bytes: &[u8]) -> Option<Vec<u8>> {
        let psk = (*self.psk.lock().unwrap())?;
        let init_msg = crate::pqc::HybridInitiatorMessage::from_bytes(init_bytes).ok()?;
        let (resp, session) = crate::pqc::psk_hybrid_respond(&psk, &init_msg).ok()?;
        *self.crypto.lock().unwrap() = Some(session);
        info!("Crypto: hybrid PQC session installed (responder)");
        Some(resp.to_bytes())
    }

    /// Initiator: complete with the peer's responder message, installing the
    /// session. Must follow a `hybrid_init` on this device.
    pub fn hybrid_complete(&self, resp_bytes: &[u8]) -> bool {
        let initiator = match self.pending_hybrid.lock().unwrap().take() {
            Some(i) => i,
            None => return false,
        };
        let resp_msg = match crate::pqc::HybridResponderMessage::from_bytes(resp_bytes) {
            Ok(m) => m,
            Err(_) => return false,
        };
        match initiator.complete(&resp_msg) {
            Ok(session) => {
                *self.crypto.lock().unwrap() = Some(session);
                info!("Crypto: hybrid PQC session installed (initiator)");
                true
            }
            Err(_) => false,
        }
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
        let crypto = Arc::clone(&self.crypto);

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
                
                // Encrypt the Opus frame with the active AEAD session (parity
                // with Android). If no session is installed yet (not paired),
                // fall back to plaintext so local testing still works.
                let payload = match crypto.lock().unwrap().as_mut() {
                    Some(c) => match c.encrypt(&encoded) {
                        Ok(ct) => ct,
                        Err(e) => { error!("Encrypt error: {}", e); continue; }
                    },
                    None => encoded,
                };

                // Create packet
                let packet = Packet::audio(device_id, channel, payload);

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
        let crypto = Arc::clone(&self.crypto);

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
                        // Decrypt before decode (parity with Android). A replayed
                        // or wrong-key frame is rejected here. Plaintext fallback
                        // only while no session is installed.
                        let plain = match crypto.lock().unwrap().as_ref() {
                            Some(c) => match c.decrypt(&data) {
                                Ok(p) => p,
                                Err(e) => { warn!("Decrypt error: {}", e); continue; }
                            },
                            None => data,
                        };
                        // Decode
                        let samples = match decoder.lock().unwrap().decode(&plain) {
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
