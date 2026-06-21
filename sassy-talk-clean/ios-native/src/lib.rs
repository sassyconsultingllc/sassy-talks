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
use std::sync::{Mutex, OnceLock};
use log::info;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Global app state. `OnceLock<Mutex<Option<_>>>` (not `static mut`) so the
/// Swift audio/render thread and the UI/timer thread share it without UB; the
/// inner `Option` lets `sassytalkie_shutdown` clear it. Mirrors BT_MANAGER.
static APP_STATE: OnceLock<Mutex<Option<StateMachine>>> = OnceLock::new();

fn app_state() -> &'static Mutex<Option<StateMachine>> {
    APP_STATE.get_or_init(|| Mutex::new(None))
}

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
            if let Ok(mut g) = app_state().lock() {
                *g = Some(state);
            }
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
    if let Ok(mut g) = app_state().lock() {
        if let Some(mut s) = g.take() {
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
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
            s.set_channel(channel);
            return true;
        }
    }
    false
}

/// Get current channel
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_get_channel() -> u8 {
    if let Ok(g) = app_state().lock() {
        if let Some(s) = g.as_ref() {
            return s.get_channel();
        }
    }
    1 // Default channel
}

/// Start PTT transmission
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_ptt_press() -> bool {
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
            return s.on_ptt_press().is_ok();
        }
    }
    false
}

/// Stop PTT transmission
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_ptt_release() -> bool {
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
            return s.on_ptt_release().is_ok();
        }
    }
    false
}

/// Connect to a peer device
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_connect(device_id: u32) -> bool {
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
            return s.connect_to_device(device_id).is_ok();
        }
    }
    false
}

/// Disconnect from peer
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_disconnect() -> bool {
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
            return s.disconnect().is_ok();
        }
    }
    false
}

/// Start listening for incoming connections
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_start_listening() -> bool {
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
            return s.start_listening().is_ok();
        }
    }
    false
}

/// Get current state (0=Idle, 1=Connecting, 2=Connected, 3=Transmitting, 4=Receiving)
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_get_state() -> u8 {
    if let Ok(g) = app_state().lock() {
        if let Some(s) = g.as_ref() {
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
    
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
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
    
    if let Ok(mut g) = app_state().lock() {
        if let Some(s) = g.as_mut() {
            return s.get_audio_output(out_buffer).unwrap_or(0);
        }
    }
    0
}

// ───────────────────────── Bluetooth peer-finding (CoreBluetooth bridge) ─────────────────────────
//
// On iOS, Bluetooth is the PEER-DISCOVERY plane only — audio rides the IP transport
// (iOS does not expose Bluetooth Classic / RFCOMM to third-party apps). The radio
// work (BLE advertise + scan) lives in Swift via CoreBluetooth (BluetoothManager.swift).
// Swift reports discovered / lost peers across this FFI; Rust keeps the canonical
// roster and hands it back to the UI as JSON.
//
// The advertised + scanned GATT service UUID MUST match the Android app so iOS↔Android
// peers discover each other.

use crate::bluetooth::{BluetoothManager, BluetoothDevice};

/// SassyTalkie BLE service UUID — identical to Android `BleSignalingService.SERVICE_UUID`.
pub const SASSYTALKIE_BLE_SERVICE_UUID: &str = "b1a2e5d4-d5ab-7890-bede-fa12345678f0";

/// Process-wide BLE peer roster. `OnceLock<Mutex<_>>` (not `static mut`) so the
/// Swift radio thread and the Rust UI-query thread share it without UB.
static BT_MANAGER: OnceLock<Mutex<BluetoothManager>> = OnceLock::new();

fn bt_manager() -> &'static Mutex<BluetoothManager> {
    BT_MANAGER.get_or_init(|| Mutex::new(BluetoothManager::new()))
}

/// Return the BLE service UUID that Swift should advertise + scan for.
///
/// # Safety
/// Caller must free the returned string with `sassytalkie_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_bt_service_uuid() -> *const c_char {
    CString::new(SASSYTALKIE_BLE_SERVICE_UUID).unwrap().into_raw()
}

/// Register a peer discovered by the Swift CoreBluetooth central.
///
/// # Safety
/// `id` and `name` must be valid NUL-terminated UTF-8 C strings (or null).
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_bt_device_found(
    id: *const c_char,
    name: *const c_char,
    rssi: i32,
) -> bool {
    let id = match ffi::helpers::c_string_to_rust(id) {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };
    let name = ffi::helpers::c_string_to_rust(name).unwrap_or_else(|| "SassyTalkie".to_string());
    if let Ok(mut mgr) = bt_manager().lock() {
        mgr.add_device(BluetoothDevice { id, name, rssi });
        info!("BLE peer discovered (rssi={})", rssi);
        true
    } else {
        false
    }
}

/// Remove a peer the Swift central reported as lost / disconnected.
///
/// # Safety
/// `id` must be a valid NUL-terminated UTF-8 C string (or null).
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_bt_device_lost(id: *const c_char) -> bool {
    let id = match ffi::helpers::c_string_to_rust(id) {
        Some(s) => s,
        None => return false,
    };
    if let Ok(mut mgr) = bt_manager().lock() {
        mgr.remove_device(&id);
        true
    } else {
        false
    }
}

/// Number of currently-discovered BLE peers.
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_bt_peer_count() -> usize {
    bt_manager().lock().map(|m| m.devices().len()).unwrap_or(0)
}

/// JSON array of discovered peers: `[{"id":..,"name":..,"rssi":..}, ...]`.
///
/// # Safety
/// Caller must free the returned string with `sassytalkie_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_bt_get_peers_json() -> *const c_char {
    let json = bt_manager()
        .lock()
        .ok()
        .map(|m| serde_json::to_string(&m.devices()).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_else(|| "[]".to_string());
    CString::new(json).unwrap_or_default().into_raw()
}

/// Mark a peer as the actively-connected device.
///
/// # Safety
/// `id` must be a valid NUL-terminated UTF-8 C string (or null).
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_bt_set_connected(id: *const c_char) -> bool {
    let id = match ffi::helpers::c_string_to_rust(id) {
        Some(s) => s,
        None => return false,
    };
    if let Ok(mut mgr) = bt_manager().lock() {
        mgr.set_connected(id);
        true
    } else {
        false
    }
}

/// Clear the active BLE connection.
#[no_mangle]
pub unsafe extern "C" fn sassytalkie_bt_clear_connected() {
    if let Ok(mut mgr) = bt_manager().lock() {
        mgr.clear_connected();
    }
}
