/// UI module for SassyTalkie
/// 
/// Since we're building a pure Rust Android app, UI can be:
/// 1. Native Rust UI (via egui or similar)
/// 2. Minimal JNI bridge to Android Views
/// 3. WebView with Rust backend
/// 
/// For now, we'll define the UI interface that can be implemented
/// using any of the above approaches.

use log::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiEvent {
    PttPressed,
    PttReleased,
    ConnectClicked,
    ListenClicked,
    DisconnectClicked,
}

#[derive(Debug, Clone)]
pub enum UiState {
    Disconnected,
    Connecting,
    Connected { device_name: String },
    Listening,
    Transmitting,
}

/// UI manager interface
pub trait UiManager {
    fn update_status(&mut self, message: &str);
    fn update_state(&mut self, state: UiState);
    fn show_error(&mut self, error: &str);
    fn show_toast(&mut self, message: &str);
    fn enable_ptt(&mut self, enabled: bool);
}

/// Android UI implementation via JNI
#[cfg(target_os = "android")]
pub mod android {
    use super::*;
    use jni::{JNIEnv, objects::{JObject, JString}, sys::jstring};

    pub struct AndroidUi {
        // References to Java UI objects
        activity: Option<JObject<'static>>,
    }

    impl AndroidUi {
        pub fn new(env: &JNIEnv, activity: JObject) -> Self {
            Self {
                activity: Some(activity),
            }
        }

        fn run_on_ui_thread(&self, env: &JNIEnv, runnable: JObject) {
            // TODO: Call activity.runOnUiThread(runnable) via JNI
        }
    }

    impl UiManager for AndroidUi {
        fn update_status(&mut self, message: &str) {
            info!("UI Status: {}", message);
            // TODO: Update TextView via JNI
        }

        fn update_state(&mut self, state: UiState) {
            info!("UI State: {:?}", state);
            // TODO: Update UI elements via JNI based on state
        }

        fn show_error(&mut self, error: &str) {
            warn!("UI Error: {}", error);
            // TODO: Show Toast or AlertDialog via JNI
        }

        fn show_toast(&mut self, message: &str) {
            info!("UI Toast: {}", message);
            // TODO: Call Toast.makeText() via JNI
        }

        fn enable_ptt(&mut self, enabled: bool) {
            info!("PTT enabled: {}", enabled);
            // TODO: Enable/disable PTT button via JNI
        }
    }
}

/// Mock UI for testing without Android
pub struct MockUi {
    status: String,
    state: UiState,
}

impl MockUi {
    pub fn new() -> Self {
        Self {
            status: String::new(),
            state: UiState::Disconnected,
        }
    }
}

impl UiManager for MockUi {
    fn update_status(&mut self, message: &str) {
        self.status = message.to_string();
        println!("[UI] Status: {}", message);
    }

    fn update_state(&mut self, state: UiState) {
        self.state = state;
        println!("[UI] State: {:?}", self.state);
    }

    fn show_error(&mut self, error: &str) {
        eprintln!("[UI] Error: {}", error);
    }

    fn show_toast(&mut self, message: &str) {
        println!("[UI] Toast: {}", message);
    }

    fn enable_ptt(&mut self, enabled: bool) {
        println!("[UI] PTT enabled: {}", enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_ui() {
        let mut ui = MockUi::new();
        ui.update_status("Test message");
        ui.update_state(UiState::Connected { 
            device_name: "Test Device".to_string() 
        });
        ui.show_toast("Test toast");
    }
}
