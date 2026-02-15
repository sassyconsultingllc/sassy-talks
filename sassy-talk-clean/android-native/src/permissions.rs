/// Permissions Module - Runtime Permission Requests for Android
/// 
/// Handles Android 6.0+ runtime permission requests via JNI
/// Required for: Bluetooth, Microphone access
/// 
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use log::{error, info, warn};
use crate::jni_bridge::get_jvm;
use jni::objects::{JObject, JString, JValue};

/// Permission state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PermissionState {
    Granted,
    Denied,
    Unknown,
}

/// Required permissions for the app
pub struct AppPermissions {
    pub bluetooth_connect: PermissionState,
    pub bluetooth_scan: PermissionState,
    pub bluetooth_advertise: PermissionState,
    pub record_audio: PermissionState,
}

impl AppPermissions {
    /// Create new permissions tracker
    pub fn new() -> Self {
        Self {
            bluetooth_connect: PermissionState::Unknown,
            bluetooth_scan: PermissionState::Unknown,
            bluetooth_advertise: PermissionState::Unknown,
            record_audio: PermissionState::Unknown,
        }
    }

    /// Check if all critical permissions are granted
    pub fn all_granted(&self) -> bool {
        self.bluetooth_connect == PermissionState::Granted &&
        self.bluetooth_scan == PermissionState::Granted &&
        self.record_audio == PermissionState::Granted
    }

    /// Get list of permissions that need to be requested
    pub fn get_missing_permissions(&self) -> Vec<String> {
        let mut missing = Vec::new();
        
        if self.bluetooth_connect != PermissionState::Granted {
            missing.push("android.permission.BLUETOOTH_CONNECT".to_string());
        }
        if self.bluetooth_scan != PermissionState::Granted {
            missing.push("android.permission.BLUETOOTH_SCAN".to_string());
        }
        if self.bluetooth_advertise != PermissionState::Granted {
            missing.push("android.permission.BLUETOOTH_ADVERTISE".to_string());
        }
        if self.record_audio != PermissionState::Granted {
            missing.push("android.permission.RECORD_AUDIO".to_string());
        }
        
        missing
    }
}

/// Permission constants from Android SDK
const PERMISSION_GRANTED: i32 = 0;  // PackageManager.PERMISSION_GRANTED
const PERMISSION_DENIED: i32 = -1;  // PackageManager.PERMISSION_DENIED

/// Permission manager with real JNI implementation
pub struct PermissionManager {
    permissions: AppPermissions,
}

impl PermissionManager {
    /// Create new permission manager
    pub fn new() -> Self {
        info!("Creating permission manager");
        
        Self {
            permissions: AppPermissions::new(),
        }
    }

    /// Check if a specific permission is granted via JNI
    fn check_permission_jni(&self, permission: &str) -> PermissionState {
        let vm = match get_jvm() {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to get JVM: {}", e);
                return PermissionState::Unknown;
            }
        };
        
        let mut env = match vm.attach_current_thread() {
            Ok(e) => e,
            Err(e) => {
                error!("Failed to attach thread: {}", e);
                return PermissionState::Unknown;
            }
        };
        
        // Get application context via ActivityThread
        // ActivityThread.currentApplication().getApplicationContext()
        let activity_thread_class = match env.find_class("android/app/ActivityThread") {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to find ActivityThread: {}", e);
                return PermissionState::Unknown;
            }
        };
        
        let current_app = match env.call_static_method(
            activity_thread_class,
            "currentApplication",
            "()Landroid/app/Application;",
            &[]
        ) {
            Ok(r) => match r.l() {
                Ok(obj) => obj,
                Err(e) => {
                    error!("Failed to get Application object: {}", e);
                    return PermissionState::Unknown;
                }
            },
            Err(e) => {
                error!("Failed to call currentApplication: {}", e);
                return PermissionState::Unknown;
            }
        };
        
        // Get context
        let context = match env.call_method(
            &current_app,
            "getApplicationContext",
            "()Landroid/content/Context;",
            &[]
        ) {
            Ok(r) => match r.l() {
                Ok(obj) => obj,
                Err(e) => {
                    error!("Failed to get Context object: {}", e);
                    return PermissionState::Unknown;
                }
            },
            Err(e) => {
                error!("Failed to call getApplicationContext: {}", e);
                return PermissionState::Unknown;
            }
        };
        
        // Create permission string
        let permission_jstr = match env.new_string(permission) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to create permission string: {}", e);
                return PermissionState::Unknown;
            }
        };
        
        // Call ContextCompat.checkSelfPermission(context, permission)
        let compat_class = match env.find_class("androidx/core/content/ContextCompat") {
            Ok(c) => c,
            Err(_) => {
                // Fallback to Context.checkSelfPermission for older apps
                return self.check_permission_legacy(&mut env, &context, &permission_jstr);
            }
        };
        
        let result = match env.call_static_method(
            compat_class,
            "checkSelfPermission",
            "(Landroid/content/Context;Ljava/lang/String;)I",
            &[JValue::Object(&context), JValue::Object(&permission_jstr.into())]
        ) {
            Ok(r) => match r.i() {
                Ok(i) => i,
                Err(e) => {
                    error!("Failed to get permission result: {}", e);
                    return PermissionState::Unknown;
                }
            },
            Err(e) => {
                error!("Failed to call checkSelfPermission: {}", e);
                return PermissionState::Unknown;
            }
        };
        
        if result == PERMISSION_GRANTED {
            PermissionState::Granted
        } else if result == PERMISSION_DENIED {
            PermissionState::Denied
        } else {
            warn!("Unexpected permission result code: {}", result);
            PermissionState::Unknown
        }
    }

    /// Fallback permission check using Context directly (API 23+)
    fn check_permission_legacy<'a>(
        &self,
        env: &mut jni::JNIEnv<'a>,
        context: &JObject<'a>,
        permission: &JString<'a>
    ) -> PermissionState {
        let permission_obj: JObject<'a> = unsafe { JObject::from_raw(permission.as_raw()) };
        let result = match env.call_method(
            context,
            "checkSelfPermission",
            "(Ljava/lang/String;)I",
            &[JValue::Object(&permission_obj)]
        ) {
            Ok(r) => match r.i() {
                Ok(i) => i,
                Err(e) => {
                    error!("Legacy permission check failed: {}", e);
                    return PermissionState::Unknown;
                }
            },
            Err(e) => {
                error!("Failed to call checkSelfPermission (legacy): {}", e);
                return PermissionState::Unknown;
            }
        };

        if result == PERMISSION_GRANTED {
            PermissionState::Granted
        } else if result == PERMISSION_DENIED {
            PermissionState::Denied
        } else {
            warn!("Unexpected legacy permission result: {}", result);
            PermissionState::Unknown
        }
    }

    /// Check all permissions via JNI
    pub fn check_permissions(&mut self) -> bool {
        info!("Checking permissions via JNI");
        
        self.permissions.bluetooth_connect = 
            self.check_permission_jni("android.permission.BLUETOOTH_CONNECT");
        info!("BLUETOOTH_CONNECT: {:?}", self.permissions.bluetooth_connect);
        
        self.permissions.bluetooth_scan = 
            self.check_permission_jni("android.permission.BLUETOOTH_SCAN");
        info!("BLUETOOTH_SCAN: {:?}", self.permissions.bluetooth_scan);
        
        self.permissions.bluetooth_advertise = 
            self.check_permission_jni("android.permission.BLUETOOTH_ADVERTISE");
        info!("BLUETOOTH_ADVERTISE: {:?}", self.permissions.bluetooth_advertise);
        
        self.permissions.record_audio = 
            self.check_permission_jni("android.permission.RECORD_AUDIO");
        info!("RECORD_AUDIO: {:?}", self.permissions.record_audio);
        
        let all_granted = self.permissions.all_granted();
        info!("All permissions granted: {}", all_granted);
        
        all_granted
    }

    /// Request permissions - returns list of missing permissions
    /// Note: Actual permission request dialog must be triggered from Activity
    pub fn request_permissions(&self) -> Vec<String> {
        info!("Requesting permissions");
        
        let missing = self.permissions.get_missing_permissions();
        
        if missing.is_empty() {
            info!("All permissions already granted");
            return Vec::new();
        }
        
        info!("Permissions to request: {:?}", missing);
        missing
    }

    /// Handle permission request result
    /// This is called from Java/Kotlin onRequestPermissionsResult() via JNI
    pub fn on_permission_result(&mut self, permission: &str, granted: bool) {
        info!("Permission result: {} = {}", permission, granted);
        
        let state = if granted {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        };
        
        match permission {
            "android.permission.BLUETOOTH_CONNECT" => {
                self.permissions.bluetooth_connect = state;
            }
            "android.permission.BLUETOOTH_SCAN" => {
                self.permissions.bluetooth_scan = state;
            }
            "android.permission.BLUETOOTH_ADVERTISE" => {
                self.permissions.bluetooth_advertise = state;
            }
            "android.permission.RECORD_AUDIO" => {
                self.permissions.record_audio = state;
            }
            _ => {
                warn!("Unknown permission: {}", permission);
            }
        }
    }

    /// Get current permission states
    pub fn get_permissions(&self) -> &AppPermissions {
        &self.permissions
    }

    /// Check if critical permissions are granted
    pub fn has_critical_permissions(&self) -> bool {
        self.permissions.all_granted()
    }

    /// Get user-friendly permission explanation
    pub fn get_permission_explanation(&self, permission: &str) -> String {
        match permission {
            "android.permission.BLUETOOTH_CONNECT" => {
                "Bluetooth Connect permission is required to establish peer-to-peer \
                 connections with other Sassy-Talk users. Without this, you cannot \
                 communicate with other devices.".to_string()
            }
            "android.permission.BLUETOOTH_SCAN" => {
                "Bluetooth Scan permission is required to discover and pair with \
                 nearby devices running Sassy-Talk.".to_string()
            }
            "android.permission.BLUETOOTH_ADVERTISE" => {
                "Bluetooth Advertise permission is required to make your device \
                 discoverable to other Sassy-Talk users.".to_string()
            }
            "android.permission.RECORD_AUDIO" => {
                "Microphone permission is required to record your voice when you \
                 press the Push-to-Talk button. Without this, you cannot transmit \
                 audio to other users.".to_string()
            }
            _ => format!("Permission {} is required for app functionality.", permission)
        }
    }
}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to show permission rationale dialog
pub fn show_permission_rationale(permission: &str) -> String {
    let manager = PermissionManager::new();
    manager.get_permission_explanation(permission)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissions_creation() {
        let perms = AppPermissions::new();
        assert_eq!(perms.bluetooth_connect, PermissionState::Unknown);
        assert!(!perms.all_granted());
    }

    #[test]
    fn test_permission_manager() {
        let mut manager = PermissionManager::new();
        assert!(!manager.has_critical_permissions());
        
        manager.on_permission_result("android.permission.BLUETOOTH_CONNECT", true);
        manager.on_permission_result("android.permission.BLUETOOTH_SCAN", true);
        manager.on_permission_result("android.permission.RECORD_AUDIO", true);
        
        assert!(manager.has_critical_permissions());
    }

    #[test]
    fn test_missing_permissions() {
        let perms = AppPermissions::new();
        let missing = perms.get_missing_permissions();
        assert_eq!(missing.len(), 4); // Now includes BLUETOOTH_ADVERTISE
    }

    #[test]
    fn test_permission_explanation() {
        let manager = PermissionManager::new();
        let explanation = manager.get_permission_explanation("android.permission.RECORD_AUDIO");
        assert!(explanation.contains("Microphone"));
    }
}
