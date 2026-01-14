// SassyTalkie iOS Core Library
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

//! iOS Core Library for SassyTalkie PTT Walkie-Talkie
//! 
//! This library provides the core Rust functionality for iOS,
//! with FFI bindings for Swift to call into.

pub mod audio;
pub mod bluetooth;
pub mod codec;
pub mod protocol;
pub mod state;
pub mod transport;
pub mod ffi;

pub use audio::{AudioEngine, AudioFrame};
pub use codec::{OpusEncoder, OpusDecoder};
pub use protocol::{Packet, PacketType};
pub use state::{StateMachine, AppState};

use std::os::raw::{c_char, c_void};
use std::ffi::{CStr, CString};
use std::sync::{Arc, Mutex};
use log::info;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Global app state (singleton pattern for C FFI)
static mut APP_STATE: Option<Arc<Mutex<StateMachine>>> = None;

/// Initialize the library
/// 
/// # Safety
/// This function must be called before any other library functions
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_init() -> bool {
    // Initialize logger
    env_logger::init();
    info!("SassyTalkie iOS v{} initializing...", VERSION);
    
    // Create state machine
    match StateMachine::new() {
        Ok(state) => {
            APP_STATE = Some(Arc::new(Mutex::new(state)));
            info!("SassyTalkie initialized successfully");
            true
        }
        Err(e) => {
            eprintln!("Failed to initialize SassyTalkie: {}", e);
            false
        }
    }
}

/// Shutdown the library
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_shutdown() {
    info!("SassyTalkie shutting down...");
    if let Some(state) = APP_STATE.take() {
        if let Ok(mut s) = state.lock() {
            let _ = s.shutdown();
        }
    }
}

/// Get version string
/// 
/// # Safety
/// Caller must free the returned string with `sassytalkie_free_string`
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_get_version() -> *const c_char {
    CString::new(VERSION).unwrap().into_raw()
}

/// Free a string allocated by the library
/// 
/// # Safety
/// Pointer must have been returned by a library function
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_free_string(s: *mut c_char) {
    if !s.is_null() {
        let _ = CString::from_raw(s);
    }
}

/// Set current channel
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_set_channel(channel: u8) -> bool {
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            s.set_channel(channel);
            return true;
        }
    }
    false
}

/// Get current channel
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_get_channel() -> u8 {
    if let Some(state) = &APP_STATE {
        if let Ok(s) = state.lock() {
            return s.get_channel();
        }
    }
    1 // Default channel
}

/// Start PTT transmission
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_ptt_press() -> bool {
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            return s.on_ptt_press().is_ok();
        }
    }
    false
}

/// Stop PTT transmission
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_ptt_release() -> bool {
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            return s.on_ptt_release().is_ok();
        }
    }
    false
}

/// Connect to a peer device
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_connect(device_id: u32) -> bool {
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            return s.connect_to_device(device_id).is_ok();
        }
    }
    false
}

/// Disconnect from peer
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_disconnect() -> bool {
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            return s.disconnect().is_ok();
        }
    }
    false
}

/// Start listening for incoming connections
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_start_listening() -> bool {
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            return s.start_listening().is_ok();
        }
    }
    false
}

/// Get current state (0=Idle, 1=Connecting, 2=Connected, 3=Transmitting, 4=Receiving)
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_get_state() -> u8 {
    if let Some(state) = &APP_STATE {
        if let Ok(s) = state.lock() {
            return match s.current_state() {
                AppState::Idle => 0,
                AppState::Connecting => 1,
                AppState::Connected => 2,
                AppState::Transmitting => 3,
                AppState::Receiving => 4,
                AppState::Error => 5,
            };
        }
    }
    0
}

/// Process audio input from Swift (AVAudioEngine)
/// 
/// # Safety
/// `audio_data` must point to valid PCM samples
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_process_audio_input(
    audio_data: *const i16,
    sample_count: usize,
) -> bool {
    if audio_data.is_null() || sample_count == 0 {
        return false;
    }
    
    let samples = std::slice::from_raw_parts(audio_data, sample_count);
    
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            return s.process_audio_input(samples).is_ok();
        }
    }
    false
}

/// Get audio output for Swift (AVAudioEngine)
/// Returns number of samples written
/// 
/// # Safety
/// `buffer` must have space for at least `buffer_size` samples
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_get_audio_output(
    buffer: *mut i16,
    buffer_size: usize,
) -> usize {
    if buffer.is_null() || buffer_size == 0 {
        return 0;
    }
    
    let out_buffer = std::slice::from_raw_parts_mut(buffer, buffer_size);
    
    if let Some(state) = &APP_STATE {
        if let Ok(mut s) = state.lock() {
            return s.get_audio_output(out_buffer).unwrap_or(0);
        }
    }
    0
}
