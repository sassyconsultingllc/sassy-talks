/// Permissions Module - Runtime Permission Requests for Android
/// 
/// Handles Android 6.0+ runtime permission requests
/// Required for: Bluetooth, Microphone access

use log::{error, info, warn};

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
    pub record_audio: PermissionState,
}

impl AppPermissions {
    /// Create new permissions tracker
    pub fn new() -> Self {
        Self {
            bluetooth_connect: PermissionState::Unknown,
            bluetooth_scan: PermissionState::Unknown,
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
        if self.record_audio != PermissionState::Granted {
            missing.push("android.permission.RECORD_AUDIO".to_string());
        }
        
        missing
    }
}

/// Permission manager
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

    /// Check if permissions are granted (mock for now)
    /// 
    /// NOTE: In a real implementation, this would use JNI to call:
    /// ActivityCompat.checkSelfPermission(context, permission)
    /// 
    /// For now, we'll assume permissions need to be requested
    pub fn check_permissions(&mut self) -> bool {
        info!("Checking permissions");
        
        // In real implementation, check each permission via JNI
        // For now, default to needing permissions
        
        self.permissions.bluetooth_connect = PermissionState::Unknown;
        self.permissions.bluetooth_scan = PermissionState::Unknown;
        self.permissions.record_audio = PermissionState::Unknown;
        
        false
    }

    /// Request permissions (mock for now)
    /// 
    /// NOTE: In a real implementation, this would use JNI to call:
    /// ActivityCompat.requestPermissions(activity, permissions, requestCode)
    /// 
    /// The results would come back through onRequestPermissionsResult()
    /// which would need to be bridged back to Rust
    pub fn request_permissions(&self) -> Vec<String> {
        info!("Requesting permissions");
        
        let missing = self.permissions.get_missing_permissions();
        
        if missing.is_empty() {
            info!("All permissions already granted");
            return Vec::new();
        }
        
        info!("Permissions to request: {:?}", missing);
        
        // In real implementation:
        // 1. Get Activity context via JNI
        // 2. Call ActivityCompat.requestPermissions()
        // 3. Wait for callback in onRequestPermissionsResult()
        // 4. Update permission states based on results
        
        missing
    }

    /// Handle permission request result
    /// 
    /// This would be called from Java/Kotlin onRequestPermissionsResult()
    /// bridged through JNI
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
/// 
/// NOTE: In a real implementation, this would show a native Android dialog
/// explaining why the permission is needed before requesting it
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
        assert_eq!(missing.len(), 3);
    }

    #[test]
    fn test_permission_explanation() {
        let manager = PermissionManager::new();
        let explanation = manager.get_permission_explanation("android.permission.RECORD_AUDIO");
        assert!(explanation.contains("Microphone"));
    }
}
