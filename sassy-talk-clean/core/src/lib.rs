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
// `bitrate` — receiver-side Opus bitrate guard (estimate from frame size,
// reject out-of-band streams). No wire change; see docs/bitrate-guard-design.md.
pub mod bitrate;

// ── 2026–2027 roadmap modules (pure logic, cross-platform) ─────────────────
//   - `pqc`        — hybrid X25519 + ML-KEM-768 post-quantum key agreement,
//                    layered on top of `crypto` without modifying it.
//   - `channels`   — channel scan / priority-channel preemption state machine.
//   - `emergency`  — SOS / man-down / emergency-broadcast signalling.
//
// Gated behind the default-off `roadmap` feature: none of these are wired into
// a production wire path yet, and enabling them (esp. `emergency`, which would
// add new opcodes) requires a coordinated protocol change. Keeping them off by
// default shrinks the default build + audit surface; build/test them with
// `cargo build --features roadmap`.
#[cfg(feature = "roadmap")]
pub mod pqc;
#[cfg(feature = "roadmap")]
pub mod channels;
#[cfg(feature = "roadmap")]
pub mod emergency;
// `sealed` — metadata-resistant blinded room/peer handles so the relay sees
// only rotating opaque tokens it cannot correlate to identity or across epochs.
pub mod sealed;
