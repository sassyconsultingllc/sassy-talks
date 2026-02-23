// SassyTalkie - Standalone egui UI for development/testing
// This module is only compiled with `--features standalone-ui`.
// The production app uses Kotlin/Compose + JNI (jni_bridge.rs).

use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use log::{error, info};

use crate::state::{StateMachine, AppState};
use crate::transport::ActiveTransport;
use crate::permissions::PermissionManager;
use crate::VERSION;

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
    Connect,
    Permissions,
}

struct SassyTalkApp {
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    state_machine: Arc<Mutex<Option<StateMachine>>>,
    permission_manager: Arc<Mutex<PermissionManager>>,
    current_screen: Screen,
    status_message: Arc<Mutex<String>>,
    last_error: Arc<Mutex<Option<String>>>,
    show_error: bool,
}

impl Default for SassyTalkApp {
    fn default() -> Self {
        let ptt_pressed = Arc::new(AtomicBool::new(false));
        let current_channel = Arc::new(AtomicU8::new(1));

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
            last_error: Arc::new(Mutex::new(None)),
            show_error: false,
        }
    }
}

impl SassyTalkApp {
    fn initialize(&mut self) {
        info!("Initializing SassyTalkie");

        let has_permissions = self.permission_manager.lock().unwrap()
            .check_permissions();

        if !has_permissions {
            self.current_screen = Screen::Permissions;
            *self.status_message.lock().unwrap() = "Permissions required".to_string();
            return;
        }

        let init_result = {
            if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
                sm.initialize()
            } else {
                Err("State machine not available".to_string())
            }
        };

        match init_result {
            Ok(_) => {
                info!("App initialized");
                *self.status_message.lock().unwrap() = "Ready".to_string();
                self.current_screen = Screen::Connect;
            }
            Err(e) => {
                error!("Failed to initialize: {}", e);
                *self.last_error.lock().unwrap() = Some(e);
                self.show_error = true;
            }
        }
    }

    fn request_permissions(&mut self) {
        info!("Requesting permissions");

        let missing = self.permission_manager.lock().unwrap()
            .request_permissions();

        if missing.is_empty() {
            self.current_screen = Screen::Connect;
            self.initialize();
        } else {
            *self.status_message.lock().unwrap() =
                format!("Please grant {} permissions", missing.len());
        }
    }

    fn connect_wifi_multicast(&mut self) {
        info!("Connecting via WiFi multicast");

        *self.status_message.lock().unwrap() = "Joining WiFi...".to_string();

        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            match sm.connect_wifi_multicast() {
                Ok(_) => {
                    info!("WiFi multicast connected");
                    *self.status_message.lock().unwrap() = "Connected (WiFi)".to_string();
                    self.current_screen = Screen::Main;
                }
                Err(e) => {
                    error!("WiFi multicast failed: {}", e);
                    *self.last_error.lock().unwrap() = Some(format!("WiFi failed: {}", e));
                    self.show_error = true;
                    *self.status_message.lock().unwrap() = "WiFi failed".to_string();
                }
            }
        }
    }

    fn disconnect(&mut self) {
        info!("Disconnecting");

        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            match sm.disconnect() {
                Ok(_) => {
                    *self.status_message.lock().unwrap() = "Disconnected".to_string();
                    self.current_screen = Screen::Connect;
                }
                Err(e) => {
                    error!("Disconnect failed: {}", e);
                    *self.last_error.lock().unwrap() = Some(e);
                    self.show_error = true;
                }
            }
        }
    }

    fn handle_ptt_press(&self) {
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            if let Err(e) = sm.on_ptt_press() {
                error!("PTT press failed: {}", e);
            }
        }
    }

    fn handle_ptt_release(&self) {
        if let Some(sm) = self.state_machine.lock().unwrap().as_ref() {
            if let Err(e) = sm.on_ptt_release() {
                error!("PTT release failed: {}", e);
            }
        }
    }

    fn get_app_state(&self) -> AppState {
        self.state_machine.lock().unwrap()
            .as_ref()
            .map(|sm| sm.get_state())
            .unwrap_or(AppState::Initializing)
    }

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
        self.update_status();

        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = DARK_BG;
        style.visuals.window_fill = DARK_BG;
        ctx.set_style(style);

        match self.current_screen {
            Screen::Permissions => self.render_permissions_screen(ctx),
            Screen::Connect => self.render_connect_screen(ctx),
            Screen::Main => self.render_main_screen(ctx),
        }

        if self.show_error {
            self.render_error_dialog(ctx);
        }

        ctx.request_repaint();
    }
}

impl SassyTalkApp {
    fn render_permissions_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(DARK_BG))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new("Permissions Required")
                        .color(ORANGE).size(28.0));
                    ui.add_space(30.0);

                    egui::Frame::none()
                        .fill(CARD_BG)
                        .rounding(12.0)
                        .inner_margin(20.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Sassy-Talk needs the following permissions:")
                                .color(TEXT_GRAY).size(14.0));
                            ui.add_space(15.0);
                            ui.label(egui::RichText::new("Microphone")
                                .color(CYAN).size(16.0));
                            ui.label(egui::RichText::new("Record your voice for transmission")
                                .color(TEXT_GRAY).size(12.0));
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("WiFi")
                                .color(CYAN).size(16.0));
                            ui.label(egui::RichText::new("Connect to other devices on the network")
                                .color(TEXT_GRAY).size(12.0));
                        });

                    ui.add_space(40.0);

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

    fn render_connect_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(DARK_BG))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.heading(egui::RichText::new("Connect")
                        .color(ORANGE).size(28.0));
                    ui.add_space(10.0);

                    let status = self.status_message.lock().unwrap().clone();
                    ui.label(egui::RichText::new(status).color(TEXT_GRAY));
                    ui.add_space(20.0);

                    egui::Frame::none()
                        .fill(CARD_BG)
                        .rounding(12.0)
                        .inner_margin(20.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("WiFi (Recommended)")
                                .color(CYAN).size(18.0).strong());
                            ui.add_space(5.0);
                            ui.label(egui::RichText::new("All devices on the same WiFi network can talk.\nWorks with Android, iOS, Windows, Mac.")
                                .color(TEXT_GRAY).size(13.0));
                            ui.add_space(12.0);
                            if ui.add_sized([220.0, 50.0], egui::Button::new(
                                egui::RichText::new("Join WiFi Channel").size(16.0).color(egui::Color32::WHITE)
                            ).fill(CYAN).rounding(25.0)).clicked() {
                                self.connect_wifi_multicast();
                            }
                        });
                });
            });
    }

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

                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        if ui.button("Back").clicked() {
                            self.disconnect();
                        }
                        ui.add_space(20.0);
                        ui.heading(egui::RichText::new("Sassy-Talk").color(ORANGE).size(28.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(20.0);
                            let status_color = if is_connected { GREEN } else { RED };
                            ui.label(egui::RichText::new("*").color(status_color).size(16.0));
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
                                if ui.add_sized([60.0, 60.0], egui::Button::new(
                                    egui::RichText::new("-").size(32.0).color(CYAN)
                                ).fill(DARK_BG).rounding(8.0)).clicked() {
                                    if channel > 1 {
                                        self.current_channel.store(channel - 1, Ordering::Relaxed);
                                    }
                                }
                                ui.add_space(20.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(egui::RichText::new("CHANNEL").size(12.0).color(TEXT_GRAY));
                                    ui.label(egui::RichText::new(format!("{:02}", channel)).size(48.0).color(CYAN).strong());
                                });
                                ui.add_space(20.0);
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
                                    AppState::Transmitting => ("TX", ORANGE, format!("TRANSMITTING ON CH {}", channel)),
                                    AppState::Receiving => ("RX", CYAN, "RECEIVING AUDIO".to_string()),
                                    AppState::Connected => ("--", TEXT_GRAY, "READY - HOLD TO TALK".to_string()),
                                    _ => ("--", RED, "NOT CONNECTED".to_string()),
                                };

                                ui.label(egui::RichText::new(icon).color(color).size(18.0));
                                ui.label(egui::RichText::new(text).color(color).size(14.0));
                            });
                        });

                    ui.add_space(20.0);

                    let transport_label = match self.state_machine.lock().unwrap().as_ref() {
                        Some(sm) => match sm.get_active_transport() {
                            ActiveTransport::Wifi => "WiFi",
                            ActiveTransport::WifiDirect => "P2P",
                            ActiveTransport::Cellular => "Cell",
                            ActiveTransport::None => "---",
                        },
                        None => "---",
                    };
                    ui.label(egui::RichText::new(
                        format!("v{} | AES-256-GCM | {}", VERSION, transport_label)
                    ).size(11.0).color(TEXT_GRAY));
                });
            });
    }

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

    crate::jni_bridge::init_jvm(vm);
    info!("JVM initialized");

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
