// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-SCNETVIZP24V
/// State Machine for iOS
/// 
/// Coordinates audio, codec, and transport
/// Similar to Android version but adapted for iOS

use crate::audio::{AudioEngine, AudioFrame};
use crate::codec::{OpusEncoder, OpusDecoder};
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
    // Stable per-install sender id placed in every wire frame (<= 32 bytes).
    // Matches the role of android-native's sender_id; only needs to be a stable
    // UTF-8 string for receiver-side attribution/mixing.
    sender_id: String,
    
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
    // The LIVE AEAD session lives in the TransportManager — the single layer that
    // seals/opens every frame (whole-packet AES-256-GCM, mandatory: no session =>
    // TX refuses, RX drops). StateMachine only holds the handshake bootstrap state
    // below and installs agreed sessions into the transport via `install_session`.
    // The QR pre-shared key, kept so the hybrid PQC handshake can mix it in.
    psk: Arc<Mutex<Option<[u8; 32]>>>,
    // Pending path-(a) hybrid handshake (initiator side), between init & complete.
    pending_hybrid: Arc<Mutex<Option<crate::pqc::PskHybridInitiator>>>,
    // Pending classical X25519 key exchange, between init & complete.
    pending_key_exchange: Arc<Mutex<Option<crate::crypto::KeyExchange>>>,

    // ── Relay (Cloudflare WebSocket) ──
    // The relay room id = the QR session_id, retained on import so the Swift
    // RelayClient can connect (`wss://…/ws?room=<id>…`). Heartbeat identity:
    // epoch fixed per process, seq monotonic — same shape as desktop/Android.
    room_id: Arc<Mutex<Option<String>>>,
    session_epoch: u64,
    heartbeat_seq: Arc<std::sync::atomic::AtomicU32>,
}

impl StateMachine {
    /// Create new state machine
    pub fn new() -> Result<Self, StateError> {
        let device_id = rand::random();
        let device_name = format!("iPhone-{}", device_id % 10000);
        let sender_id = format!("ios-{:08x}", device_id);
        
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
            sender_id,
            current_channel: Arc::new(AtomicU8::new(1)),
            audio: Arc::new(Mutex::new(audio)),
            encoder: Arc::new(Mutex::new(encoder)),
            decoder: Arc::new(Mutex::new(decoder)),
            transport: Arc::new(Mutex::new(transport)),
            is_transmitting: Arc::new(AtomicBool::new(false)),
            should_stop_tx: Arc::new(AtomicBool::new(false)),
            should_stop_rx: Arc::new(AtomicBool::new(false)),
            psk: Arc::new(Mutex::new(None)),
            pending_hybrid: Arc::new(Mutex::new(None)),
            pending_key_exchange: Arc::new(Mutex::new(None)),
            room_id: Arc::new(Mutex::new(None)),
            session_epoch: {
                // Non-zero random epoch for this process (heartbeat identity).
                let v: u64 = rand::random();
                if v == 0 { 1 } else { v }
            },
            heartbeat_seq: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        })
    }

    /// Begin a classical X25519 key exchange. Returns our public key bytes to
    /// send to the peer; finish with `key_exchange_complete`.
    pub fn key_exchange_init(&self) -> Vec<u8> {
        let kx = crate::crypto::KeyExchange::new();
        let pubkey = kx.public_key_bytes().to_vec();
        *self.pending_key_exchange.lock().unwrap() = Some(kx);
        pubkey
    }

    /// Complete the classical key exchange with the peer's public key, installing
    /// the AEAD session. Returns false if there's no pending exchange.
    pub fn key_exchange_complete(&self, remote_pub: &[u8; 32]) -> bool {
        let kx = match self.pending_key_exchange.lock().unwrap().take() {
            Some(k) => k,
            None => return false,
        };
        match kx.complete(remote_pub) {
            Ok(session) => {
                self.install_session(session);
                info!("Crypto: X25519 session installed");
                true
            }
            Err(_) => false,
        }
    }

    // ── Crypto / key agreement (mirrors android-native's JNI crypto seam) ──

    /// Install a pre-shared key (the QR session key). Installs the active AEAD
    /// session into the transport (the single sealing owner) and remembers the
    /// PSK so a later hybrid handshake can mix it in.
    pub fn set_psk(&self, key: &[u8; 32]) {
        *self.psk.lock().unwrap() = Some(*key);
        self.transport.lock().unwrap().set_psk(key);
        info!("Crypto: PSK session installed");
    }

    /// Replace the active AEAD session (e.g. with a key-exchange / hybrid result).
    pub fn set_crypto_session(&self, session: crate::crypto::CryptoSession) {
        self.install_session(session);
    }

    /// Install a freshly-agreed AEAD session into the TransportManager — the one
    /// layer that seals/opens every frame (whole-packet AES-256-GCM, mandatory).
    /// The live session lives ONLY there, so there is exactly one nonce counter +
    /// replay window per channel (a second copy would desync the replay window).
    fn install_session(&self, session: crate::crypto::CryptoSession) {
        self.transport.lock().unwrap().set_crypto(session);
    }

    /// Import a scanned QR session (the JSON an Android/desktop host generates),
    /// switching to its channel and installing its key. Reuses the SHARED core
    /// `SessionManager::import_session`, so iOS accepts/rejects exactly the QRs
    /// Android does (same expiry/length/channel validation) and lands on the same
    /// 32-byte PSK — that PSK is what both ends seal audio with, so the pairing is
    /// genuinely cross-platform. Returns the channel (1-8) on success, None on a
    /// malformed/expired QR.
    pub fn import_session_qr(&self, qr_json: &str) -> Option<u8> {
        // A transient manager runs the canonical validation; we then pull the raw
        // PSK back out so the hybrid-PQC handshake can still mix it in later.
        let mut mgr = crate::session::SessionManager::new(&self.device_name);
        let (channel, _crypto, _cohort) = mgr.import_session(qr_json).ok()?;
        let psk = mgr.get_psk_for_channel(channel)?;
        self.set_channel(channel);
        self.set_psk(&psk); // stores PSK for hybrid + installs the AEAD into the transport
        // Retain the relay room id (= QR session_id) so the Swift RelayClient can
        // join `wss://…/ws?room=<id>`. Same room id Android/desktop derive.
        *self.room_id.lock().unwrap() = mgr.get_session_id(channel);
        info!("Crypto: session imported from QR on channel {}", channel);
        Some(channel)
    }

    // ── Relay (Cloudflare WebSocket) bridge — the socket is owned by Swift
    // (URLSessionWebSocketTask); Rust supplies the room id, seals/opens frames,
    // and builds heartbeats. Mirrors the android-native queue-bridge model. ──

    /// The relay room id (= QR session_id) for the active session, if paired.
    pub fn relay_room_id(&self) -> Option<String> {
        self.room_id.lock().unwrap().clone()
    }

    /// Mark the relay connected/disconnected. While active, TX frames are teed
    /// into the relay outbound queue for the Swift RelayClient to forward.
    pub fn set_relay_active(&self, active: bool) {
        self.transport.lock().unwrap().set_relay_active(active);
    }

    /// Drain one sealed frame for the Swift RelayClient to send over the WS.
    pub fn poll_relay_outbound(&self) -> Option<Vec<u8>> {
        self.transport.lock().unwrap().poll_relay_outbound()
    }

    /// Build the next OP_HEARTBEAT frame for the RelayClient to send every ~2 s
    /// (keeps us off the relay's idle-staleness sweeper + surfaces us in peer
    /// liveness). 23-byte TLV payload, caps=0 (no PQC handshake on iOS yet).
    pub fn relay_heartbeat_frame(&self) -> Vec<u8> {
        let seq = self.heartbeat_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let state = if self.is_transmitting.load(Ordering::SeqCst) {
            crate::control::PRESENCE_SPEAKING
        } else {
            crate::control::PRESENCE_IDLE
        };
        crate::control::encode_heartbeat(self.session_epoch, seq, crate::control::now_ms(), state, 0)
    }

    /// Process a sealed frame received from the relay (Swift hands us the raw WS
    /// binary message): decrypt + unpack + channel-filter + decode + play —
    /// identical to the multicast RX path. Returns true if it was a playable
    /// audio frame for our channel. Control frames (heartbeats from peers) fail
    /// the AEAD decrypt and return false, so audio vs control self-disambiguates.
    pub fn process_relay_frame(&self, sealed: &[u8]) -> bool {
        let plain = match self.transport.lock().unwrap().open_sealed(sealed) {
            Some(p) => p,
            None => return false,
        };
        let (frame_channel, _sub, sender, _name, _ts, compressed) =
            match sassytalkie_core::wire::unpack_wire_frame(&plain) {
                Ok(parts) => parts,
                Err(_) => return false,
            };
        // The relay echoes frames to everyone in the room including us — skip our
        // own loopback (matches desktop cellular `sender == peer_id`).
        if sender == self.sender_id {
            return false;
        }
        if frame_channel != self.current_channel.load(Ordering::SeqCst) {
            return false;
        }
        let samples = match self.decoder.lock().unwrap().decode(&compressed) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let frame = AudioFrame::new(samples);
        let _ = self.audio.lock().unwrap().write_output_frame(&frame);
        *self.state.lock().unwrap() = AppState::Receiving;
        true
    }

    /// Host a channel: mint a fresh session QR (the JSON another device scans) and
    /// install its key locally so the host is paired too. `duration_hours` clamps
    /// to the core's 1..=72 window; `group_name` may be empty ("Channel N"). The
    /// returned JSON is rendered as a QR for an Android/iOS joiner. Same shared
    /// `SessionManager` Android uses, so the QR is cross-platform by construction.
    pub fn generate_session_qr(&self, channel: u8, duration_hours: u32, group_name: &str) -> Option<String> {
        let mut mgr = crate::session::SessionManager::new(&self.device_name);
        let json = mgr.generate_session_qr(channel, duration_hours, group_name).ok()?;
        // Install our own key so the host can also TX/RX on this channel.
        self.import_session_qr(&json);
        Some(json)
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
        self.install_session(session);
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
                self.install_session(session);
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
        let device_name = self.device_name.clone();
        let sender_id = self.sender_id.clone();

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

                // Pack the SHARED cross-platform wire frame (core::wire) — byte
                // identical to android-native's pack_wire_frame. The transport
                // seals the WHOLE frame (header + audio) with the active AEAD
                // session and REFUSES to send when unpaired, so encryption is
                // mandatory and an iOS frame is interchangeable with an Android one.
                let wire = sassytalkie_core::wire::pack_wire_frame(
                    channel,
                    sassytalkie_core::wire::SUBCH_MAIN,
                    &sender_id,
                    &device_name,
                    sassytalkie_core::wire::now_ms(),
                    &encoded,
                );

                match transport.lock().unwrap().send(&wire) {
                    Ok(()) => {}
                    // Not paired yet: drop the frame rather than leak cleartext.
                    Err(crate::transport::TransportError::NotEncrypted) => {}
                    Err(e) => warn!("TX send failed: {}", e),
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
        let transport = Arc::clone(&self.transport);
        let should_stop = Arc::clone(&self.should_stop_rx);
        let state = Arc::clone(&self.state);
        let current_channel = Arc::clone(&self.current_channel);
        let self_sender_id = self.sender_id.clone();

        thread::spawn(move || {
            info!("RX thread started");
            let mut buffer = vec![0u8; 2048];
            // Per-sender Opus decoders. Opus is STATEFUL, so decoding multiple
            // senders through one shared decoder corrupts audio when their
            // frames interleave; key a decoder by wire sender_id instead.
            let mut decoders: std::collections::HashMap<String, OpusDecoder> =
                std::collections::HashMap::new();
            
            while !should_stop.load(Ordering::SeqCst) {
                // Receive packet
                let (size, _addr) = match transport.lock().unwrap().receive(&mut buffer) {
                    Ok(r) => r,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                
                // Unpack the SHARED cross-platform wire frame. The transport has
                // already authenticated + decrypted the whole datagram (mandatory),
                // so unencrypted/tampered/replayed frames never reach here. iOS and
                // Android emit byte-identical frames, so an Android sender decodes
                // here unchanged.
                let (frame_channel, _subch, sender, _name, _ts, compressed) =
                    match sassytalkie_core::wire::unpack_wire_frame(&buffer[..size]) {
                        Ok(parts) => parts,
                        Err(e) => {
                            warn!("Failed to parse wire frame: {}", e);
                            continue;
                        }
                    };

                // Skip our own multicast loopback (mirrors the relay path at
                // `sender == self.sender_id`); the LAN multicast socket echoes
                // our own transmitted frames back to us otherwise.
                if sender == self_sender_id {
                    continue;
                }

                if frame_channel == current_channel.load(Ordering::SeqCst) {
                    let decoder = decoders
                        .entry(sender.clone())
                        .or_insert_with(|| OpusDecoder::new().expect("create Opus decoder"));
                    let samples = match decoder.decode(&compressed) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Decode error from {}: {}", sender, e);
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
