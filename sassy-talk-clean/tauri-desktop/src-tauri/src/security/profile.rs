use std::fs;
use std::path::Path;
use log::{error, warn, info};

/// Multi-user / work profile security violation
#[derive(Debug, Clone, Copy)]
pub enum ProfileViolation {
    WorkProfileDetected,
    SecondaryUserDetected,
    CrossProfileClipboard,
    CrossProfileKeyboard,
    CrossProfileAccessibility,
    ManagedProfileActive,
}

/// User profile types on Android
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProfileType {
    Primary,          // User 0 (main personal profile)
    Secondary,        // User 10, 20, etc. (guest/additional users)
    WorkProfile,      // Managed work profile
    Unknown,
}

/// Work profile and multi-user detector
pub struct ProfileChecker {
    current_user_id: i32,
    profile_type: ProfileType,
}

impl ProfileChecker {
    pub fn new() -> Self {
        let current_user_id = Self::get_current_user_id();
        let profile_type = Self::detect_profile_type(current_user_id);
        
        Self {
            current_user_id,
            profile_type,
        }
    }

    /// Get current Android user ID
    /// Primary user: 0
    /// Secondary users: 10, 11, 12, ...
    /// Work profile: Usually 10+
    fn get_current_user_id() -> i32 {
        // Method 1: Read from environment
        if let Ok(user_id) = std::env::var("USER_ID") {
            if let Ok(id) = user_id.parse::<i32>() {
                return id;
            }
        }

        // Method 2: Parse from /proc/self/cgroup
        if let Ok(cgroup) = fs::read_to_string("/proc/self/cgroup") {
            // Look for lines like: 1:name=systemd:/user.slice/user-10.slice
            for line in cgroup.lines() {
                if line.contains("user-") {
                    if let Some(user_part) = line.split("user-").nth(1) {
                        if let Some(id_str) = user_part.split('.').next() {
                            if let Ok(id) = id_str.parse::<i32>() {
                                return id;
                            }
                        }
                    }
                }
            }
        }

        // Method 3: Check /data/user/<N> directory
        // App data is at /data/user/<user_id>/<package_name>
        if let Ok(cwd) = std::env::current_dir() {
            let path_str = cwd.to_string_lossy();
            if path_str.contains("/data/user/") {
                if let Some(user_part) = path_str.split("/data/user/").nth(1) {
                    if let Some(id_str) = user_part.split('/').next() {
                        if let Ok(id) = id_str.parse::<i32>() {
                            return id;
                        }
                    }
                }
            }
        }

        // Method 4: Check UID range
        // Android user IDs are in ranges:
        // User 0: 10000-19999
        // User 10: 1010000-1019999
        // User 20: 1020000-1029999
        let uid = unsafe { libc::getuid() };
        if uid >= 1000000 {
            // Secondary user: UID format is 1UUUAAA where UUU is user ID
            let user_id = (uid / 100000) % 1000;
            return user_id as i32;
        }

        // Default: assume primary user
        0
    }

    /// Detect profile type based on user ID and system properties
    fn detect_profile_type(user_id: i32) -> ProfileType {
        // User 0 is always primary
        if user_id == 0 {
            return ProfileType::Primary;
        }

        // Check if work profile
        if Self::is_work_profile() {
            return ProfileType::WorkProfile;
        }

        // User 10+ without work profile indicators = secondary user
        if user_id >= 10 {
            return ProfileType::Secondary;
        }

        ProfileType::Unknown
    }

    /// Check if running in work profile (managed profile)
    fn is_work_profile() -> bool {
        // Work profiles are indicated by:
        // 1. /data/misc/profiles/ directory exists
        // 2. Device policy manager is active
        // 3. Specific system properties

        // Check for work profile directory
        if Path::new("/data/misc/profiles").exists() {
            return true;
        }

        // Check for device policy manager database
        if Path::new("/data/system/device_policies.xml").exists() {
            if let Ok(content) = fs::read_to_string("/data/system/device_policies.xml") {
                if content.contains("managedProfile") || content.contains("deviceOwner") {
                    return true;
                }
            }
        }

        // Check for work profile badge resources
        // Work profiles have special launcher icons with badge
        if Path::new("/system/framework/framework-res.apk").exists() {
            // This would require parsing APK, skip for now
        }

        false
    }

    /// Check for cross-profile clipboard access
    /// Work profile can read personal profile clipboard (privacy violation)
    pub fn check_clipboard_access(&self) -> Result<(), ProfileViolation> {
        // Check if clipboard service is from different user
        if let Ok(services) = fs::read_to_string("/proc/self/maps") {
            // Look for clipboard service from different user context
            if services.contains("clipboard") && self.profile_type != ProfileType::Primary {
                warn!("Clipboard service detected in non-primary profile");
                return Err(ProfileViolation::CrossProfileClipboard);
            }
        }

        // Check clipboard content provider
        // Personal clipboard: content://0@clipboard/primary_clip
        // Work profile accessing: content://10@clipboard/primary_clip (accessing user 0)
        
        // This requires JNI to query ContentResolver
        // For now, detect if we're in non-primary profile (potential risk)
        if self.profile_type == ProfileType::WorkProfile {
            warn!("Running in work profile - clipboard may be monitored");
            return Err(ProfileViolation::WorkProfileDetected);
        }

        Ok(())
    }

    /// Check for cross-profile keyboard (IME) access
    /// Work profile keyboard can monitor personal profile typing
    pub fn check_keyboard_access(&self) -> Result<(), ProfileViolation> {
        // Check current input method (keyboard)
        // Location: /data/system/users/<user_id>/settings_secure.xml
        let settings_path = format!("/data/system/users/{}/settings_secure.xml", self.current_user_id);
        
        if let Ok(settings) = fs::read_to_string(&settings_path) {
            // Look for default_input_method setting
            if let Some(ime_line) = settings.lines().find(|l| l.contains("default_input_method")) {
                // Check if IME package belongs to different user
                // IME package format: com.android.inputmethod.latin
                
                // If we're in work profile but using personal profile IME (or vice versa)
                // This is a security risk
                
                if self.profile_type != ProfileType::Primary {
                    warn!("Non-primary profile detected - keyboard may monitor input");
                    return Err(ProfileViolation::CrossProfileKeyboard);
                }
            }
        }

        Ok(())
    }

    /// Check for cross-profile accessibility service access
    /// Accessibility services can read screen content across profiles
    pub fn check_accessibility_access(&self) -> Result<(), ProfileViolation> {
        // Check enabled accessibility services
        let settings_path = format!("/data/system/users/{}/settings_secure.xml", self.current_user_id);
        
        if let Ok(settings) = fs::read_to_string(&settings_path) {
            if let Some(a11y_line) = settings.lines().find(|l| l.contains("enabled_accessibility_services")) {
                // Check if any services are enabled
                if !a11y_line.contains("null") && !a11y_line.is_empty() {
                    warn!("Accessibility services enabled - screen content may be monitored");
                    
                    // If we're in non-primary profile, this is especially concerning
                    if self.profile_type != ProfileType::Primary {
                        return Err(ProfileViolation::CrossProfileAccessibility);
                    }
                }
            }
        }

        // Check for accessibility service process
        if let Ok(maps) = fs::read_to_string("/proc/self/maps") {
            if maps.contains("accessibility") || maps.contains("a11y") {
                warn!("Accessibility service detected in process");
                return Err(ProfileViolation::CrossProfileAccessibility);
            }
        }

        Ok(())
    }

    /// Check if device has active device owner or profile owner (MDM)
    pub fn check_device_management(&self) -> Result<(), ProfileViolation> {
        // Device policy database
        let policy_paths = [
            "/data/system/device_policies.xml",
            "/data/system_de/0/device_policies.xml",
        ];

        for path in &policy_paths {
            if let Ok(content) = fs::read_to_string(path) {
                // Check for device owner (full device control)
                if content.contains("device-owner") || content.contains("deviceOwner") {
                    error!("Device owner detected - device is fully managed");
                    return Err(ProfileViolation::ManagedProfileActive);
                }

                // Check for profile owner (work profile)
                if content.contains("profile-owner") || content.contains("profileOwner") {
                    error!("Profile owner detected - work profile is managed");
                    return Err(ProfileViolation::ManagedProfileActive);
                }
            }
        }

        Ok(())
    }

    /// Check for any other users on device
    pub fn check_multiple_users(&self) -> Result<(), ProfileViolation> {
        // Check /data/system/users/ directory
        if let Ok(entries) = fs::read_dir("/data/system/users") {
            let mut user_count = 0;
            
            for entry in entries.flatten() {
                if entry.file_type().ok().map(|t| t.is_dir()).unwrap_or(false) {
                    user_count += 1;
                }
            }

            if user_count > 1 {
                warn!("Multiple users detected on device: {}", user_count);
                
                // If we're not the primary user, this is a concern
                if self.current_user_id != 0 {
                    return Err(ProfileViolation::SecondaryUserDetected);
                }
            }
        }

        Ok(())
    }

    /// Comprehensive profile security check
    pub fn comprehensive_check(&self) -> Result<(), ProfileViolation> {
        // Only allow primary user (user 0)
        if self.current_user_id != 0 {
            error!("Running in non-primary user profile (user {})", self.current_user_id);
            return Err(ProfileViolation::SecondaryUserDetected);
        }

        // Block work profiles entirely
        if self.profile_type == ProfileType::WorkProfile {
            error!("Running in work profile - BLOCKED");
            return Err(ProfileViolation::WorkProfileDetected);
        }

        // Check for cross-profile threats
        self.check_clipboard_access()?;
        self.check_keyboard_access()?;
        self.check_accessibility_access()?;
        self.check_device_management()?;
        self.check_multiple_users()?;

        Ok(())
    }

    /// Get current user info for logging
    pub fn get_user_info(&self) -> String {
        format!(
            "User ID: {}, Type: {:?}",
            self.current_user_id,
            self.profile_type
        )
    }

    /// Check if running in safe environment
    pub fn is_safe_environment(&self) -> bool {
        self.current_user_id == 0 && self.profile_type == ProfileType::Primary
    }
}

/// Show work profile warning to user
pub fn show_work_profile_warning(violation: ProfileViolation) {
    error!("╔═══════════════════════════════════════════════════╗");
    error!("║        WORK PROFILE / MULTI-USER DETECTED        ║");
    error!("║                                                   ║");
    
    match violation {
        ProfileViolation::WorkProfileDetected => {
            error!("║  This app is running in a work profile.          ║");
            error!("║  Work profiles may monitor clipboard and input.  ║");
        }
        ProfileViolation::SecondaryUserDetected => {
            error!("║  This app is running as a secondary user.        ║");
            error!("║  Secondary users may have restricted access.     ║");
        }
        ProfileViolation::CrossProfileClipboard => {
            error!("║  Cross-profile clipboard access detected.        ║");
            error!("║  Your clipboard may be monitored.                ║");
        }
        ProfileViolation::CrossProfileKeyboard => {
            error!("║  Cross-profile keyboard detected.                ║");
            error!("║  Your typing may be monitored.                   ║");
        }
        ProfileViolation::CrossProfileAccessibility => {
            error!("║  Accessibility services detected.                ║");
            error!("║  Screen content may be monitored.                ║");
        }
        ProfileViolation::ManagedProfileActive => {
            error!("║  Device is managed (MDM/EMM).                    ║");
            error!("║  Administrator has full device control.          ║");
        }
    }
    
    error!("║                                                   ║");
    error!("║  For your security and privacy, this app will    ║");
    error!("║  only run on the primary user profile (user 0).  ║");
    error!("║                                                   ║");
    error!("║  Please install and run from your personal       ║");
    error!("║  profile, not from a work profile.               ║");
    error!("╚═══════════════════════════════════════════════════╝");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_checker_init() {
        let checker = ProfileChecker::new();
        println!("User info: {}", checker.get_user_info());
        
        // On development machine, should be user 0
        // On Android device, depends on context
    }

    #[test]
    fn test_user_id_detection() {
        let user_id = ProfileChecker::get_current_user_id();
        println!("Current user ID: {}", user_id);
        
        // Should be 0 on primary profile
        // 10+ on secondary or work profile
    }

    #[test]
    fn test_work_profile_detection() {
        let is_work = ProfileChecker::is_work_profile();
        println!("Is work profile: {}", is_work);
        
        // Should be false on personal device
        // True if running in managed work profile
    }
}
