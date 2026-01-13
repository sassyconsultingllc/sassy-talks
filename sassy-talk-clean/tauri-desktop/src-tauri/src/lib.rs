// Sassy-Talk Core Library
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

//! # Sassy-Talk
//!
//! Cross-platform PTT walkie-talkie with retro vibes.
//!
//! ## Supported Platforms
//! - Android (API 26+)
//! - iOS (14.0+)
//! - macOS (11.0+)
//! - Windows (10+)
//! - Linux (Ubuntu 22.04+)
//!
//! ## Architecture
//! ```text
//! [Mic] → [CPAL] → [Opus] → [AES-GCM] → [UDP Multicast] → [Decrypt] → [Opus] → [Speaker]
//! ```
//!
//! ## Transport Strategy
//! - WiFi UDP Multicast: Primary transport (works everywhere including iOS)
//! - BLE: Discovery only (signaling)
//! - Bluetooth Classic: Android-to-Android optimization (offline capable)

pub mod audio;
pub mod codec;
pub mod commands;
pub mod protocol;
pub mod security;
pub mod transport;

// Re-exports
pub use audio::AudioEngine;
pub use codec::{OpusEncoder, OpusDecoder};
pub use protocol::{Packet, PacketType};
pub use security::CryptoEngine;
pub use transport::{TransportManager, PeerInfo};

use std::sync::Arc;
use tokio::sync::RwLock;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Audio sample rate (Opus native)
pub const SAMPLE_RATE: u32 = 48000;

/// Audio frame duration in milliseconds
pub const FRAME_DURATION_MS: u32 = 20;

/// Samples per frame (48000 * 20 / 1000 = 960)
pub const FRAME_SIZE: usize = 960;

/// Opus bitrate for voice (24kbps provides good quality)
pub const OPUS_BITRATE: i32 = 24000;

/// UDP multicast group for discovery
pub const MULTICAST_ADDR: &str = "224.0.0.251";

/// UDP port for discovery
pub const DISCOVERY_PORT: u16 = 5354;

/// UDP port for audio data
pub const AUDIO_PORT: u16 = 41337;

/// Maximum UDP packet size
pub const MAX_PACKET_SIZE: usize = 1200;

/// Number of virtual channels (like radio frequencies)
pub const NUM_CHANNELS: u8 = 16;

/// Discovery broadcast interval
pub const DISCOVERY_INTERVAL_MS: u64 = 1000;

/// Peer timeout (no heartbeat)
pub const PEER_TIMEOUT_MS: u64 = 5000;

/// Application state shared across all Tauri commands
pub struct AppState {
    pub audio_engine: Arc<RwLock<AudioEngine>>,
    pub transport: Arc<RwLock<TransportManager>>,
    pub crypto: Arc<RwLock<CryptoEngine>>,
    pub device_id: u32,
    pub device_name: String,
    pub channel: Arc<RwLock<u8>>,
    pub transmitting: Arc<RwLock<bool>>,
    pub connected_peers: Arc<RwLock<Vec<PeerInfo>>>,
}

impl AppState {
    pub fn new(device_id: u32, device_name: String) -> Self {
        Self {
            audio_engine: Arc::new(RwLock::new(AudioEngine::new())),
            transport: Arc::new(RwLock::new(TransportManager::new(device_id, device_name.clone()))),
            crypto: Arc::new(RwLock::new(CryptoEngine::new())),
            device_id,
            device_name,
            channel: Arc::new(RwLock::new(1)), // Channel 1 default
            transmitting: Arc::new(RwLock::new(false)),
            connected_peers: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

// FFI exports for mobile platforms

/// Initialize library (FFI)
#[no_mangle]
pub extern "C" fn sassy_talk_init() -> bool {
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("SassyTalk"),
        );
    }
    true
}

/// Get version string (FFI)
#[no_mangle]
pub extern "C" fn sassy_talk_version() -> *const std::ffi::c_char {
    static VERSION_CSTR: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
    VERSION_CSTR.as_ptr() as *const std::ffi::c_char
}
