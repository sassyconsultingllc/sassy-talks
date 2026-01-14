/// Tauri Commands - Frontend API
/// 
/// These commands are called from the React frontend via invoke()

use crate::{AppState, AudioDeviceInfo, ConnectionStatus, PeerInfo};
use std::sync::Arc;
use tauri::State;

/// Start discovery and listening for peers
#[tauri::command]
pub async fn start_discovery(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.start_discovery()
        .await
        .map_err(|e| e.to_string())
}

/// Stop discovery
#[tauri::command]
pub async fn stop_discovery(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.stop_discovery()
        .await
        .map_err(|e| e.to_string())
}

/// Get list of nearby devices
#[tauri::command]
pub async fn get_nearby_devices(state: State<'_, Arc<AppState>>) -> Result<Vec<PeerInfo>, String> {
    Ok(state.get_nearby_devices().await)
}

/// Connect to a peer (not used in multicast mode, but kept for API compatibility)
#[tauri::command]
pub async fn connect_to_peer(
    _state: State<'_, Arc<AppState>>,
    _peer_id: u32,
) -> Result<(), String> {
    // In multicast mode, connection is automatic
    Ok(())
}

/// Disconnect from all peers
#[tauri::command]
pub async fn disconnect(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.stop_discovery()
        .await
        .map_err(|e| e.to_string())
}

/// Get connection status
#[tauri::command]
pub async fn get_connection_status(state: State<'_, Arc<AppState>>) -> Result<ConnectionStatus, String> {
    Ok(state.get_connection_status().await)
}

/// Start transmitting audio (PTT press)
#[tauri::command]
pub async fn start_transmit(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.start_transmit()
        .await
        .map_err(|e| e.to_string())
}

/// Stop transmitting audio (PTT release)
#[tauri::command]
pub async fn stop_transmit(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.stop_transmit()
        .await
        .map_err(|e| e.to_string())
}

/// Start receiving audio (already handled by discovery)
#[tauri::command]
pub async fn start_receiving(_state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Receiving is automatic in multicast mode
    Ok(())
}

/// Stop receiving audio
#[tauri::command]
pub async fn stop_receiving(_state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Receiving is automatic in multicast mode
    Ok(())
}

/// Audio devices response
#[derive(serde::Serialize)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDeviceInfo>,
    pub outputs: Vec<AudioDeviceInfo>,
}

/// Get available audio devices
#[tauri::command]
pub async fn get_audio_devices(state: State<'_, Arc<AppState>>) -> Result<AudioDevices, String> {
    let (inputs, outputs) = state.get_audio_devices().await;
    Ok(AudioDevices { inputs, outputs })
}

/// Set input audio device
#[tauri::command]
pub async fn set_input_device(
    state: State<'_, Arc<AppState>>,
    device_name: String,
) -> Result<(), String> {
    state.set_input_device(&device_name)
        .await
        .map_err(|e| e.to_string())
}

/// Set output audio device
#[tauri::command]
pub async fn set_output_device(
    state: State<'_, Arc<AppState>>,
    device_name: String,
) -> Result<(), String> {
    state.set_output_device(&device_name)
        .await
        .map_err(|e| e.to_string())
}

/// Volume levels
#[derive(serde::Serialize)]
pub struct Volume {
    pub input: f32,
    pub output: f32,
}

/// Get volume levels
#[tauri::command]
pub async fn get_volume(state: State<'_, Arc<AppState>>) -> Result<Volume, String> {
    let (input, output) = state.get_volume().await;
    Ok(Volume { input, output })
}

/// Set volume levels
#[tauri::command]
pub async fn set_volume(
    state: State<'_, Arc<AppState>>,
    input: f32,
    output: f32,
) -> Result<(), String> {
    state.set_volume(input, output).await;
    Ok(())
}

/// Get current channel
#[tauri::command]
pub async fn get_channel(state: State<'_, Arc<AppState>>) -> Result<u8, String> {
    Ok(state.get_channel())
}

/// Set channel
#[tauri::command]
pub async fn set_channel(
    state: State<'_, Arc<AppState>>,
    channel: u8,
) -> Result<(), String> {
    state.set_channel(channel).await;
    Ok(())
}

/// Application status
#[derive(serde::Serialize)]
pub struct AppStatus {
    pub connection_status: ConnectionStatus,
    pub channel: u8,
    pub peer_count: usize,
    pub is_transmitting: bool,
}

/// Get application status
#[tauri::command]
pub async fn get_status(state: State<'_, Arc<AppState>>) -> Result<AppStatus, String> {
    let connection_status = state.get_connection_status().await;
    let channel = state.get_channel();
    let peers = state.get_nearby_devices().await;
    let peer_count = peers.len();
    
    Ok(AppStatus {
        connection_status,
        channel,
        peer_count,
        is_transmitting: matches!(connection_status, ConnectionStatus::Transmitting),
    })
}

/// Device information
#[derive(serde::Serialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_name: String,
    pub version: String,
}

/// Get device information
#[tauri::command]
pub async fn get_device_info(state: State<'_, Arc<AppState>>) -> Result<DeviceInfo, String> {
    let (device_id, device_name) = state.get_device_info();
    
    Ok(DeviceInfo {
        device_id: format!("{:08X}", device_id),
        device_name,
        version: crate::VERSION.to_string(),
    })
}

/// Set roger beep enabled
#[tauri::command]
pub async fn set_roger_beep(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    state.set_roger_beep(enabled);
    Ok(())
}

/// Set VOX enabled
#[tauri::command]
pub async fn set_vox_enabled(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    state.set_vox_enabled(enabled);
    Ok(())
}

/// Set VOX threshold
#[tauri::command]
pub async fn set_vox_threshold(
    state: State<'_, Arc<AppState>>,
    threshold: f32,
) -> Result<(), String> {
    state.set_vox_threshold(threshold).await;
    Ok(())
}
