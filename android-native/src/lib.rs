// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-UZ73OTR4FDRB
// SassyTalkie - Production-Ready Android Native Library
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
//
// Built as cdylib for Kotlin/Compose app consumption via JNI exports.
// Also supports standalone egui mode for development/testing
// (compile with `--features standalone-ui`).

#[allow(unused_imports)]
use log::{error, info, warn};

pub mod jni_bridge;
pub mod audio;
pub mod audio_effects;
pub mod audio_routing;
pub mod device_quirks;
pub mod state;
pub mod permissions;
pub mod wifi_transport;
pub mod wifi_direct;
pub mod transport;
// session module migrated to sassytalkie-core in v2.8 (pure logic — QR
// generate/import, SessionManager — no JNI deps). Re-export keeps every
// `crate::session::*` consumer in jni_bridge.rs etc. working without source
// changes.
pub use sassytalkie_core::session;
pub mod users;
pub mod opus_ffi;
pub mod codec;
pub mod audio_pipeline;
pub mod cellular_transport;

// ── v2.7.x: shared cross-platform core ────────────────────────────────────
// crypto, audio_cache, cohort_history, and protocol opcodes live in the
// `sassytalkie-core` crate. Re-exported here so existing `crate::crypto::*`
// / `crate::audio_cache::*` import paths in the rest of android-native keep
// working without a sweeping change.
pub use sassytalkie_core::crypto;
pub use sassytalkie_core::pqc;
pub use sassytalkie_core::audio_cache;
pub use sassytalkie_core::cohort_history;
pub use sassytalkie_core::protocol;
// Life-safety signalling (SOS / man-down beacons + stand-down). The wire
// codec and beacon cadence are core-owned so Android/iOS/desktop stay
// byte-identical; this crate supplies only the JNI glue in jni_bridge.rs.
pub use sassytalkie_core::emergency;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Standalone egui UI (development mode only) ──
// Compiled only with `cargo build --features standalone-ui`.
// The production Kotlin/Compose app uses JNI exports from jni_bridge.rs instead.
#[cfg(feature = "standalone-ui")]
mod standalone_ui;
