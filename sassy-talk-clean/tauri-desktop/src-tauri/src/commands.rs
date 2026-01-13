// Tauri Commands - IPC interface between frontend and Rust backend
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use std::sync::Arc;
use tauri::State;
use tracing::{info, warn, error, debug};
use serde::{Serialize, Deserialize};

use crate::AppState;
use crate::transport::PeerInfo;
use crate::audio::AudioDeviceInfo;

// ============================================================================
// Response Types
// ============================================================================

#[derive(Serialize)]
pub struct StatusResponse {
    pub connected: bool,
    pub transmitting: bool,
    pub receiving: bool,
    pub channel: u8,
    pub peer_count: usize,
    pub signal_strength: i32, // -100 to 0 dBm (simulated)
}

#[derive(Serialize)]
pub struct DeviceInfoResponse {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct VolumeResponse {
    pub input: u32,
    pub output: u32,
}

// ============================================================================
// Discovery & Connection Commands
// ============================================================================

#[tauri::command]
pub async fn start_discovery(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    info!("Starting device discovery...");
    
    let mut transport = state.transport.write().await;
    transport.start_discovery().await.map_err(|e| e.to_string())?;
    
    Ok(())
}

#[tauri::command]
pub async fn stop_discovery(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    info!("Stopping device discovery");
    
    let mut transport = state.transport.write().await;
    transport.stop_discovery().await;
    
    Ok(())
}

#[tauri::command]
pub async fn get_nearby_devices(state: State<'_, Arc<AppState>>) -> Result<Vec<PeerInfo>, String> {
    let transport = state.transport.read().await;
    Ok(transport.get_nearby_peers())
}

#[tauri::command]
pub async fn connect_to_peer(
    state: State<'_, Arc<AppState>>,
    peer_id: u32,
) -> Result<(), String> {
    info!("Connecting to peer: {:08X}", peer_id);
    
    let mut transport = state.transport.write().await;
    transport.connect_to_peer(peer_id).await.map_err(|e| e.to_string())?;
    
    // Add to connected peers
    if let Some(peer) = transport.get_peer_info(peer_id) {
        let mut peers = state.connected_peers.write().await;
        if !peers.iter().any(|p| p.device_id == peer_id) {
            peers.push(peer);
        }
    }
    
    Ok(())
}

#[tauri::command]
pub async fn disconnect(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    info!("Disconnecting from all peers");
    
    let mut transport = state.transport.write().await;
    transport.disconnect_all().await;
    
    let mut peers = state.connected_peers.write().await;
    peers.clear();
    
    Ok(())
}

#[tauri::command]
pub async fn get_connection_status(state: State<'_, Arc<AppState>>) -> Result<bool, String> {
    let peers = state.connected_peers.read().await;
    Ok(!peers.is_empty())
}

// ============================================================================
// Audio Commands
// ============================================================================

#[tauri::command]
pub async fn start_transmit(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut transmitting = state.transmitting.write().await;
    if *transmitting {
        return Ok(()); // Already transmitting
    }
    
    info!("PTT: Starting transmission");
    *transmitting = true;
    
    // Start audio capture
    let mut audio = state.audio_engine.write().await;
    audio.start_capture().map_err(|e| e.to_string())?;
    
    Ok(())
}

#[tauri::command]
pub async fn stop_transmit(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut transmitting = state.transmitting.write().await;
    if !*transmitting {
        return Ok(());
    }
    
    info!("PTT: Stopping transmission");
    *transmitting = false;
    
    // Stop audio capture
    let mut audio = state.audio_engine.write().await;
    audio.stop_capture();
    
    Ok(())
}

#[tauri::command]
pub async fn start_receiving(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    info!("Starting audio reception");
    
    let mut audio = state.audio_engine.write().await;
    audio.start_playback().map_err(|e| e.to_string())?;
    
    Ok(())
}

#[tauri::command]
pub async fn stop_receiving(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    info!("Stopping audio reception");
    
    let mut audio = state.audio_engine.write().await;
    audio.stop_playback();
    
    Ok(())
}

#[tauri::command]
pub async fn get_audio_devices(state: State<'_, Arc<AppState>>) -> Result<Vec<AudioDeviceInfo>, String> {
    let audio = state.audio_engine.read().await;
    Ok(audio.list_devices())
}

#[tauri::command]
pub async fn set_input_device(
    state: State<'_, Arc<AppState>>,
    device_name: String,
) -> Result<(), String> {
    info!("Setting input device: {}", device_name);
    
    let mut audio = state.audio_engine.write().await;
    audio.select_input_device(&device_name).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_output_device(
    state: State<'_, Arc<AppState>>,
    device_name: String,
) -> Result<(), String> {
    info!("Setting output device: {}", device_name);
    
    let mut audio = state.audio_engine.write().await;
    audio.select_output_device(&device_name).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_volume(state: State<'_, Arc<AppState>>) -> Result<VolumeResponse, String> {
    let audio = state.audio_engine.read().await;
    Ok(VolumeResponse {
        input: audio.get_input_volume(),
        output: audio.get_output_volume(),
    })
}

#[tauri::command]
pub async fn set_volume(
    state: State<'_, Arc<AppState>>,
    input: Option<u32>,
    output: Option<u32>,
) -> Result<(), String> {
    let audio = state.audio_engine.read().await;
    
    if let Some(vol) = input {
        audio.set_input_volume(vol);
    }
    if let Some(vol) = output {
        audio.set_output_volume(vol);
    }
    
    Ok(())
}

// ============================================================================
// Channel Commands
// ============================================================================

#[tauri::command]
pub async fn get_channel(state: State<'_, Arc<AppState>>) -> Result<u8, String> {
    let channel = state.channel.read().await;
    Ok(*channel)
}

#[tauri::command]
pub async fn set_channel(
    state: State<'_, Arc<AppState>>,
    channel: u8,
) -> Result<(), String> {
    if channel < 1 || channel > crate::NUM_CHANNELS {
        return Err(format!("Invalid channel: {} (must be 1-{})", channel, crate::NUM_CHANNELS));
    }
    
    info!("Switching to channel {}", channel);
    
    let mut current = state.channel.write().await;
    *current = channel;
    
    // Update transport to filter for this channel
    let mut transport = state.transport.write().await;
    transport.set_channel(channel);
    
    Ok(())
}

// ============================================================================
// Status Commands
// ============================================================================

#[tauri::command]
pub async fn get_status(state: State<'_, Arc<AppState>>) -> Result<StatusResponse, String> {
    let transmitting = state.transmitting.read().await;
    let channel = state.channel.read().await;
    let peers = state.connected_peers.read().await;
    let audio = state.audio_engine.read().await;
    
    Ok(StatusResponse {
        connected: !peers.is_empty(),
        transmitting: *transmitting,
        receiving: audio.is_playing(),
        channel: *channel,
        peer_count: peers.len(),
        signal_strength: -50, // Simulated for now
    })
}

#[tauri::command]
pub async fn get_device_info(state: State<'_, Arc<AppState>>) -> Result<DeviceInfoResponse, String> {
    let platform = if cfg!(target_os = "android") {
        "Android"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        "Unknown"
    };
    
    Ok(DeviceInfoResponse {
        device_id: format!("{:08X}", state.device_id),
        device_name: state.device_name.clone(),
        platform: platform.to_string(),
        version: crate::VERSION.to_string(),
    })
}

// ============================================================================
// Settings Commands
// ============================================================================

#[tauri::command]
pub async fn set_roger_beep(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    info!("Roger beep: {}", if enabled { "enabled" } else { "disabled" });
    // TODO: Store in settings
    Ok(())
}

#[tauri::command]
pub async fn set_vox_enabled(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    info!("VOX: {}", if enabled { "enabled" } else { "disabled" });
    // TODO: Implement voice-activated transmission
    Ok(())
}

#[tauri::command]
pub async fn set_vox_threshold(
    state: State<'_, Arc<AppState>>,
    threshold: f32,
) -> Result<(), String> {
    if threshold < 0.0 || threshold > 1.0 {
        return Err("Threshold must be between 0.0 and 1.0".to_string());
    }
    info!("VOX threshold: {}", threshold);
    // TODO: Store threshold
    Ok(())
}
