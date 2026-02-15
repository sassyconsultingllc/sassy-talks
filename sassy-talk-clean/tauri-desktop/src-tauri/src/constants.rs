/// Shared Constants - Common Configuration for SassyTalkie
/// 
/// Centralized constants used across all platforms
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.

/// Protocol version for wire compatibility
pub const PROTOCOL_VERSION: u8 = 1;

/// Application version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default multicast address for peer discovery
pub const DEFAULT_MULTICAST_ADDR: &str = "239.255.42.42";

/// Default multicast port (used if random port disabled)
pub const DEFAULT_MULTICAST_PORT: u16 = 5555;

/// Port range for random port selection
pub const PORT_RANGE_START: u16 = 49152;  // Start of dynamic/private ports
pub const PORT_RANGE_END: u16 = 65535;    // End of port range

/// Maximum UDP packet size
pub const MAX_PACKET_SIZE: usize = 1500;

/// Peer timeout in seconds
pub const PEER_TIMEOUT_SECS: u64 = 30;

/// Discovery beacon interval in seconds
pub const BEACON_INTERVAL_SECS: u64 = 5;

/// Keepalive interval in seconds (between discovery beacons)
pub const KEEPALIVE_INTERVAL_SECS: u64 = 10;

/// Audio configuration
pub const SAMPLE_RATE: u32 = 48000;       // 48kHz for Opus
pub const FRAME_SIZE: usize = 960;        // 20ms at 48kHz
pub const FRAME_DURATION_MS: u32 = 20;
pub const OPUS_BITRATE: i32 = 32000;      // 32kbps VBR

/// Channel configuration
pub const MIN_CHANNEL: u8 = 1;
pub const MAX_CHANNEL: u8 = 16;           // Standardized to 1-16

/// Encryption settings
pub const KEY_ROTATION_SECS: u64 = 60;    // Rotate keys every 60 seconds
pub const NONCE_SIZE: usize = 12;         // 96-bit nonce for AES-GCM
pub const AUTH_TAG_SIZE: usize = 16;      // 128-bit auth tag

/// Service UUID for Bluetooth RFCOMM (Android)
pub const BLUETOOTH_SERVICE_UUID: &str = "8ce255c0-223a-11e0-ac64-0803450c9a66";
