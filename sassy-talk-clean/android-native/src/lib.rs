// SassyTalkie - Complete Production-Ready Android App
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use log::{error, info, warn};

mod bluetooth;
mod jni_bridge;
mod audio;
mod state;
mod permissions;

use bluetooth::BluetoothDevice;
use state::{StateMachine, AppState};
use permissions::PermissionManager;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Colors
const DARK_BG: egui::Color32 = egui::Color32::from_rgb(26, 26, 46);
const CARD_BG: egui::Color32 = egui::Color32::from_rgb(37, 37, 64);
const ORANGE: egui::Color32 = egui::Color32::from_rgb(255, 140, 0);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0, 230, 200);
const GREEN: egui::Color32 = egui::Color32::from_rgb(76, 217, 100);
const RED: egui::Color32 = egui::Color32::from_rgb(239, 83, 80);
const TEXT_GRAY: egui::Color32 = egui::Color32::from_rgb(150, 150, 160);

/// UI Screen
#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Main,
    DeviceList,
    Permissions,
}

struct SassyTalkApp {
    // Atomics for PTT and channel
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    
    // State machine (coordinates everything)
    state_machine: Arc<Mutex<Option<StateMachine>>>,
    
    // Permission manager
    permission_manager: Arc<Mutex<PermissionManager>>,
    
    // UI state
    current_screen: Screen,
    status_message: Arc<Mutex<String>>,
    paired_devices: Arc<Mutex<Vec<BluetoothDevice>>>,
    selected_device: Option<usize>,
    
    // Error handling
    last_error: Arc<Mutex<Option<String>>>,
    show_error: bool,
}

impl Default for SassyTalkApp {
    fn default() -> Self {
        let ptt_pressed = Arc::new(AtomicBool::new(false));
        let current_channel = Arc::new(AtomicU8::new(1));
        
        // Create state machine
        let state_machine = StateMachine::new(
            Arc::clone(&ptt_pressed),
            Arc::clone(&current_channel),
        );
        
        Self {
            ptt_pressed,
            current_channel,
            state_machine: Arc::new(Mutex::new(Some(state_machine))),
            permission_manager: Arc::new(Mutex::new(PermissionManager::new())),
            current_screen: Screen::Permissions,
            status_message: Arc::new(Mutex::new("Initializing...".to_string())),
            paired_devices: Arc::new(Mutex::new(Vec::new())),
            selected_device: None,
            last_error: Arc::new(Mutex::new(None)),
            show_error: false,
        }
    }
}

impl SassyTalkApp {
    /// Initialize the app
    fn initialize(&mut self) {
        info!("Initializing SassyTalkie");
        
        // Check permissions first
        let has_permissions = self.permission_manager.lock().unwrap()
            .check_permissions();
        
        if !has_permissions {
            self.current_screen = Screen::Permissions;
            *self.status_message.lock().unwrap() = "Permissions required".to_string();
            return;
        }
        
        // Initialize state machine
        let init_result = {
            if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
                sm.initialize()
            } else {
                Err("State machine not available".to_string())
            }
        };
        
        match init_result {
            Ok(_) => {
                info!("✓ App initialized");
                *self.status_message.lock().unwrap() = "Ready".to_string();
                self.current_screen = Screen::DeviceList;
                self.refresh_device_list();
            }
            Err(e) => {
                error!("Failed to initialize: {}", e);
                *self.last_error.lock().unwrap() = Some(e);
                self.show_error = true;
            }
        }
    }

    /// Request permissions
    fn request_permissions(&mut self) {
        info!("Requesting permissions");
        
        let missing = self.permission_manager.lock().unwrap()
            .request_permissions();
        
        if missing.is_empty() {
            // All permissions granted
            self.current_screen = Screen::DeviceList;
            self.initialize();
        } else {
            // Show rationale for each permission
            *self.status_message.lock().unwrap() = 
                format!("Please grant {} permissions", missing.len());
        }
    }

    /// Refresh paired devices list
    fn refresh_device_list(&mut self) {
        info!("Refreshing device list");
        
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            match sm.get_paired_devices() {
                Ok(devices) => {
                    info!("Found {} paired device(s)", devices.len());
                    *self.paired_devices.lock().unwrap() = devices;
                }
                Err(e) => {
                    error!("Failed to get devices: {}", e);
                    *self.last_error.lock().unwrap() = Some(e);
                    self.show_error = true;
                }
            }
        }
    }

    /// Connect to selected device
    fn connect_to_device(&mut self, device_address: String) {
        info!("Connecting to: {}", device_address);
        
        *self.status_message.lock().unwrap() = "Connecting...".to_string();
        
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            match sm.connect_to_device(&device_address) {
                Ok(_) => {
                    info!("✓ Connected");
                    *self.status_message.lock().unwrap() = "Connected".to_string();
                    self.current_screen = Screen::Main;
                }
                Err(e) => {
                    error!("Connection failed: {}", e);
                    *self.last_error.lock().unwrap() = Some(format!("Connection failed: {}", e));
                    self.show_error = true;
                    *self.status_message.lock().unwrap() = "Connection failed".to_string();
                }
            }
        }
    }

    /// Start listening mode (server)
    fn start_listening(&mut self) {
        info!("Starting listen mode");
        
        *self.status_message.lock().unwrap() = "Listening...".to_string();
        
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            match sm.start_listening() {
                Ok(_) => {
                    info!("✓ Listening for connections");
                    self.current_screen = Screen::Main;
                }
                Err(e) => {
                    error!("Failed to listen: {}", e);
                    *self.last_error.lock().unwrap() = Some(e);
                    self.show_error = true;
                }
            }
        }
    }

    /// Disconnect
    fn disconnect(&mut self) {
        info!("Disconnecting");
        
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            match sm.disconnect() {
                Ok(_) => {
                    *self.status_message.lock().unwrap() = "Disconnected".to_string();
                    self.current_screen = Screen::DeviceList;
                }
                Err(e) => {
                    error!("Disconnect failed: {}", e);
                    *self.last_error.lock().unwrap() = Some(e);
                    self.show_error = true;
                }
            }
        }
    }

    /// Handle PTT press
    fn handle_ptt_press(&self) {
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            if let Err(e) = sm.on_ptt_press() {
                error!("PTT press failed: {}", e);
            }
        }
    }

    /// Handle PTT release
    fn handle_ptt_release(&self) {
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            if let Err(e) = sm.on_ptt_release() {
                error!("PTT release failed: {}", e);
            }
        }
    }

    /// Get current app state
    fn get_app_state(&self) -> AppState {
        self.state_machine.lock().unwrap()
            .as_ref()
            .map(|sm| sm.get_state())
            .unwrap_or(AppState::Initializing)
    }

    /// Update status message
    fn update_status(&self) {
        let state = self.get_app_state();
        
        let status = match state {
            AppState::Initializing => "Initializing...",
            AppState::Ready => "Ready",
            AppState::Connecting => "Connecting...",
            AppState::Connected => "Connected",
            AppState::Transmitting => "Transmitting",
            AppState::Receiving => "Receiving",
            AppState::Disconnecting => "Disconnecting...",
            AppState::Error => "Error",
        };
        
        *self.status_message.lock().unwrap() = status.to_string();
    }
}

impl eframe::App for SassyTalkApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Update status
        self.update_status();
        
        // Dark theme
        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = DARK_BG;
        style.visuals.window_fill = DARK_BG;
        ctx.set_style(style);
        
        match self.current_screen {
            Screen::Permissions => self.render_permissions_screen(ctx),
            Screen::DeviceList => self.render_device_list_screen(ctx),
            Screen::Main => self.render_main_screen(ctx),
        }
        
        // Show error dialog if needed
        if self.show_error {
            self.render_error_dialog(ctx);
        }
        
        ctx.request_repaint();
    }
}

impl SassyTalkApp {
    /// Render permissions screen
    fn render_permissions_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(DARK_BG))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    
                    // Icon
                    ui.label(egui::RichText::new("🔒").size(80.0));
                    ui.add_space(20.0);
                    
                    // Title
                    ui.heading(egui::RichText::new("Permissions Required")
                        .color(ORANGE).size(28.0));
                    ui.add_space(30.0);
                    
                    // Explanation
                    egui::Frame::none()
                        .fill(CARD_BG)
                        .rounding(12.0)
                        .inner_margin(20.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Sassy-Talk needs the following permissions:")
                                .color(TEXT_GRAY).size(14.0));
                            ui.add_space(15.0);
                            
                            ui.label(egui::RichText::new("🎤 Microphone")
                                .color(CYAN).size(16.0));
                            ui.label(egui::RichText::new("Record your voice for transmission")
                                .color(TEXT_GRAY).size(12.0));
                            ui.add_space(10.0);
                            
                            ui.label(egui::RichText::new("📡 Bluetooth")
                                .color(CYAN).size(16.0));
                            ui.label(egui::RichText::new("Connect to other devices")
                                .color(TEXT_GRAY).size(12.0));
                        });
                    
                    ui.add_space(40.0);
                    
                    // Grant button
                    if ui.add_sized([220.0, 60.0], egui::Button::new(
                        egui::RichText::new("Grant Permissions").size(18.0).color(egui::Color32::WHITE)
                    ).fill(ORANGE).rounding(30.0)).clicked() {
                        self.request_permissions();
                    }
                    
                    ui.add_space(20.0);
                    ui.label(egui::RichText::new("Privacy: We don't collect any data")
                        .color(TEXT_GRAY).size(11.0));
                });
            });
    }

    /// Render device list screen
    fn render_device_list_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(DARK_BG))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    
                    // Header
                    ui.heading(egui::RichText::new("Select Device")
                        .color(ORANGE).size(28.0));
                    ui.add_space(10.0);
                    
                    let status = self.status_message.lock().unwrap().clone();
                    ui.label(egui::RichText::new(status).color(TEXT_GRAY));
                    
                    ui.add_space(30.0);
                    
                    // Refresh button
                    if ui.button("🔄 Refresh Devices").clicked() {
                        self.refresh_device_list();
                    }
                    
                    ui.add_space(20.0);
                    
                    // Device list
                    let devices = self.paired_devices.lock().unwrap().clone();
                    
                    if devices.is_empty() {
                        egui::Frame::none()
                            .fill(CARD_BG)
                            .rounding(12.0)
                            .inner_margin(20.0)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("No paired devices found")
                                    .color(TEXT_GRAY).size(16.0));
                                ui.add_space(10.0);
                                ui.label(egui::RichText::new("Pair devices in Android Bluetooth settings")
                                    .color(TEXT_GRAY).size(12.0));
                            });
                    } else {
                        for (idx, device) in devices.iter().enumerate() {
                            let device_name = device.name.clone();
                            let device_addr = device.address.clone();
                            
                            egui::Frame::none()
                                .fill(CARD_BG)
                                .rounding(12.0)
                                .inner_margin(15.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("📱").size(24.0));
                                        ui.add_space(10.0);
                                        ui.vertical(|ui| {
                                            ui.label(egui::RichText::new(&device_name)
                                                .color(CYAN).size(16.0));
                                            ui.label(egui::RichText::new(&device_addr)
                                                .color(TEXT_GRAY).size(12.0));
                                        });
                                    });
                                    
                                    ui.add_space(10.0);
                                    
                                    if ui.add_sized([200.0, 40.0], egui::Button::new(
                                        egui::RichText::new("Connect").size(14.0)
                                    ).fill(GREEN).rounding(8.0)).clicked() {
                                        self.connect_to_device(device_addr);
                                    }
                                });
                            
                            ui.add_space(10.0);
                        }
                    }
                    
                    ui.add_space(30.0);
                    
                    // Listen mode button
                    if ui.add_sized([220.0, 50.0], egui::Button::new(
                        egui::RichText::new("Listen for Connection").size(16.0).color(egui::Color32::WHITE)
                    ).fill(CYAN).rounding(25.0)).clicked() {
                        self.start_listening();
                    }
                });
            });
    }

    /// Render main PTT screen
    fn render_main_screen(&mut self, ctx: &egui::Context) {
        let ptt_active = self.ptt_pressed.load(Ordering::Relaxed);
        let channel = self.current_channel.load(Ordering::Relaxed);
        let app_state = self.get_app_state();
        
        let is_connected = app_state == AppState::Connected || 
                          app_state == AppState::Transmitting ||
                          app_state == AppState::Receiving;
        
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(DARK_BG))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    
                    // Header
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        
                        // Back button
                        if ui.button("← Devices").clicked() {
                            self.disconnect();
                        }
                        
                        ui.add_space(20.0);
                        ui.heading(egui::RichText::new("Sassy-Talk").color(ORANGE).size(28.0));
                        
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(20.0);
                            let status_color = if is_connected { GREEN } else { RED };
                            ui.label(egui::RichText::new("●").color(status_color).size(16.0));
                            
                            let status = self.status_message.lock().unwrap().clone();
                            ui.label(egui::RichText::new(status).color(TEXT_GRAY));
                        });
                    });
                    
                    ui.add_space(30.0);
                    
                    // Channel selector
                    egui::Frame::none()
                        .fill(CARD_BG)
                        .rounding(12.0)
                        .inner_margin(16.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Channel down
                                if ui.add_sized([60.0, 60.0], egui::Button::new(
                                    egui::RichText::new("−").size(32.0).color(CYAN)
                                ).fill(DARK_BG).rounding(8.0)).clicked() {
                                    if channel > 1 {
                                        self.current_channel.store(channel - 1, Ordering::Relaxed);
                                    }
                                }
                                
                                ui.add_space(20.0);
                                
                                // Channel display
                                ui.vertical_centered(|ui| {
                                    ui.label(egui::RichText::new("CHANNEL").size(12.0).color(TEXT_GRAY));
                                    ui.label(egui::RichText::new(format!("{:02}", channel)).size(48.0).color(CYAN).strong());
                                });
                                
                                ui.add_space(20.0);
                                
                                // Channel up
                                if ui.add_sized([60.0, 60.0], egui::Button::new(
                                    egui::RichText::new("+").size(32.0).color(CYAN)
                                ).fill(DARK_BG).rounding(8.0)).clicked() {
                                    if channel < 99 {
                                        self.current_channel.store(channel + 1, Ordering::Relaxed);
                                    }
                                }
                            });
                        });
                    
                    ui.add_space(40.0);
                    
                    // PTT Button
                    let button_size = 220.0;
                    let button_color = if ptt_active { ORANGE } else { CARD_BG };
                    let ring_color = if ptt_active { ORANGE } else { if is_connected { CYAN } else { TEXT_GRAY } };
                    
                    let response = ui.add_sized(
                        [button_size, button_size],
                        egui::Button::new(
                            egui::RichText::new(if ptt_active { "TX" } else { "PTT" })
                                .size(36.0)
                                .color(egui::Color32::WHITE)
                                .strong()
                        )
                        .fill(button_color)
                        .stroke(egui::Stroke::new(6.0, ring_color))
                        .rounding(button_size / 2.0)
                    );
                    
                    // Handle PTT press/release - WIRED TO REAL AUDIO!
                    if is_connected {
                        if response.is_pointer_button_down_on() {
                            if !ptt_active {
                                self.ptt_pressed.store(true, Ordering::Relaxed);
                                self.handle_ptt_press();
                            }
                        } else if ptt_active {
                            self.ptt_pressed.store(false, Ordering::Relaxed);
                            self.handle_ptt_release();
                        }
                    }
                    
                    ui.add_space(40.0);
                    
                    // Status bar
                    let status_bg = match app_state {
                        AppState::Transmitting => egui::Color32::from_rgba_unmultiplied(255, 140, 0, 50),
                        AppState::Receiving => egui::Color32::from_rgba_unmultiplied(0, 230, 200, 50),
                        _ => CARD_BG,
                    };
                    
                    egui::Frame::none()
                        .fill(status_bg)
                        .rounding(8.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (icon, color, text) = match app_state {
                                    AppState::Transmitting => ("◉", ORANGE, format!("TRANSMITTING ON CH {}", channel)),
                                    AppState::Receiving => ("◉", CYAN, "RECEIVING AUDIO".to_string()),
                                    AppState::Connected => ("○", TEXT_GRAY, "READY - HOLD TO TALK".to_string()),
                                    _ => ("○", RED, "NOT CONNECTED".to_string()),
                                };
                                
                                ui.label(egui::RichText::new(icon).color(color).size(18.0));
                                ui.label(egui::RichText::new(text).color(color).size(14.0));
                            });
                        });
                    
                    ui.add_space(20.0);
                    
                    // Version
                    ui.label(egui::RichText::new(format!("v{} • AES-256 Encrypted", VERSION))
                        .size(11.0).color(TEXT_GRAY));
                });
            });
    }

    /// Render error dialog
    fn render_error_dialog(&mut self, ctx: &egui::Context) {
        egui::Window::new("Error")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if let Some(error) = self.last_error.lock().unwrap().as_ref() {
                    ui.label(egui::RichText::new(error).color(RED));
                }
                
                ui.add_space(20.0);
                
                if ui.button("OK").clicked() {
                    self.show_error = false;
                    *self.last_error.lock().unwrap() = None;
                }
            });
    }
}

#[cfg(target_os = "android")]
use eframe::Renderer;

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    use eframe::NativeOptions;
    
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("SassyTalk"),
    );
    
    info!("=== Sassy-Talk v{} Starting ===", VERSION);
    
    // CRITICAL: Initialize JVM for JNI
    info!("Initializing JVM for JNI...");
    let vm = unsafe {
        match jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to get JavaVM: {}", e);
                return;
            }
        }
    };
    
    jni_bridge::init_jvm(vm);
    info!("✓ JVM initialized");
    
    // Create app
    let mut sassy_app = SassyTalkApp::default();
    sassy_app.initialize();
    
    let options = NativeOptions {
        renderer: Renderer::Glow,
        ..Default::default()
    };
    
    eframe::run_native(
        "Sassy-Talk",
        options,
        Box::new(|_cc| Box::new(sassy_app)),
    ).expect("Failed to start eframe");
}
