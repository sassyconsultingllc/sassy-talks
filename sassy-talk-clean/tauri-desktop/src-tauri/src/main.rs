// Sassy-Talk: Cross-Platform PTT Walkie-Talkie
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
// 
// Retro walkie-talkies are legit. This is the next-gen version.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;

// Use the library crate
use sassy_talk_lib::{AppState, commands, VERSION};

fn generate_device_id() -> u32 {
    let uuid = uuid::Uuid::new_v4();
    let bytes = uuid.as_bytes();
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn get_device_name() -> String {
    #[cfg(target_os = "android")]
    {
        return "Android Device".to_string();
    }
    #[cfg(target_os = "ios")]
    {
        return "iOS Device".to_string();
    }
    #[cfg(target_os = "macos")]
    {
        return hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Mac".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        return hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Windows PC".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        return hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Linux PC".to_string());
    }
    #[cfg(not(any(
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )))]
    {
        "Unknown Device".to_string()
    }
}

fn main() {
    // Initialize logging
    let _subscriber = FmtSubscriber::builder()
        .with_max_level(if cfg!(debug_assertions) {
            Level::DEBUG
        } else {
            Level::INFO
        })
        .with_target(false)
        .compact()
        .init();

    info!("╔══════════════════════════════════════════╗");
    info!("║     SASSY-TALK v{}                   ║", VERSION);
    info!("║     Cross-Platform PTT Walkie-Talkie     ║");
    info!("║     © 2025 Sassy Consulting LLC          ║");
    info!("╚══════════════════════════════════════════╝");

    // Mobile-only: Run security checks
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        use sassy_talk_lib::security::run_startup_checks;
        info!("Running mobile security checks...");
        if let Err(e) = run_startup_checks() {
            error!("Security violation: {:?}", e);
            std::process::exit(1);
        }
        info!("✓ Security checks passed");
    }

    // Initialize app state
    let device_id = generate_device_id();
    let device_name = get_device_name();
    let state = AppState::new(device_id, device_name.clone());
    
    info!("Device ID: {:08X}", device_id);
    info!("Device Name: {}", device_name);

    // Run Tauri application
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Arc::new(state))
        .invoke_handler(tauri::generate_handler![
            // Connection
            commands::start_discovery,
            commands::stop_discovery,
            commands::get_nearby_devices,
            commands::connect_to_peer,
            commands::disconnect,
            commands::get_connection_status,
            // Audio
            commands::start_transmit,
            commands::stop_transmit,
            commands::start_receiving,
            commands::stop_receiving,
            commands::get_audio_devices,
            commands::set_input_device,
            commands::set_output_device,
            commands::get_volume,
            commands::set_volume,
            // Channel
            commands::get_channel,
            commands::set_channel,
            // Status
            commands::get_status,
            commands::get_device_info,
            // Settings
            commands::set_roger_beep,
            commands::set_vox_enabled,
            commands::set_vox_threshold,
            // Tones
            commands::play_connection_tone,
            commands::play_delivered_tone,
            commands::play_failed_tone,
            commands::play_roger_tone,
        ])
        .setup(|_app| {
            info!("Sassy-Talk setup complete");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Error running Sassy-Talk");
}
