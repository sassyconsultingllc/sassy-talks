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
pub mod state;
pub mod permissions;
pub mod crypto;
pub mod wifi_transport;
pub mod wifi_direct;
pub mod transport;
pub mod session;
pub mod users;
pub mod audio_cache;
pub mod codec;
pub mod audio_pipeline;
pub mod cellular_transport;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Standalone egui UI (development mode only) ──
// Compiled only with `cargo build --features standalone-ui`.
// The production Kotlin/Compose app uses JNI exports from jni_bridge.rs instead.
#[cfg(feature = "standalone-ui")]
mod standalone_ui;
