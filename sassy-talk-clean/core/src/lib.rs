//! sassytalkie-core
//!
//! Cross-platform shared core for the SassyTalkie walkie-talkie family.
//! Anything in here MUST be reachable from every consumer:
//!   - `android-native` (via the Rust JNI bridge in jni_bridge.rs)
//!   - `tauri-desktop/src-tauri` (Windows / macOS / Linux desktop)
//!   - `ios-native` (when revived)
//!
//! The split rule is simple: if a module needs JNI, AudioRecord, AudioTrack,
//! cpal, BLE GATT, ContentProvider, NetworkCallback, or any other OS-level
//! API — it does NOT belong here. It belongs in the consumer crate.
//!
//! Modules:
//!   - `protocol`        — wire-level opcodes (OP_PTT_START_V2, etc.) and
//!                         TLV (de)serialization. THE module that fixes the
//!                         "Android added OP_WAKE but Windows doesn't know"
//!                         drift.
//!   - `audio_cache`     — Live / Queue / Mix / Replay multi-speaker buffer
//!                         + mixer. Pure logic, no audio hardware.
//!   - `cohort_history`  — LRU host/joiner record persistence with JSON
//!                         (de)serialization.
//!   - `crypto`          — AES-256-GCM session encryption + X25519 key
//!                         exchange. Pure Rust, no platform hooks.

pub mod protocol;
pub mod audio_cache;
pub mod cohort_history;
pub mod crypto;
pub mod session;
