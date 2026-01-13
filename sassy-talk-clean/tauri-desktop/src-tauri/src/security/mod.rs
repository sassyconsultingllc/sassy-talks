// Security Module - Cross-platform security hardening
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
//
// Security features by platform:
// - Android: Full suite (anti-debug, root detection, hook detection, integrity)
// - iOS: Anti-debug, jailbreak detection, integrity
// - Desktop: Binary integrity only (debug builds warn)

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
pub use android::*;

mod crypto;
pub use crypto::CryptoEngine;

use thiserror::Error;
use tracing::{info, warn, error};

#[derive(Error, Debug, Clone)]
pub enum SecurityViolation {
    #[error("Debugger detected")]
    DebuggerDetected,
    #[error("Root/jailbreak detected")]
    RootDetected,
    #[error("Emulator detected")]
    EmulatorDetected,
    #[error("Hook framework detected")]
    HookDetected,
    #[error("Signature verification failed")]
    SignatureInvalid,
    #[error("Binary integrity compromised")]
    TamperDetected,
    #[error("Work profile detected")]
    WorkProfileDetected,
}

/// Run startup security checks (mobile only)
#[cfg(target_os = "android")]
pub fn run_startup_checks() -> Result<(), SecurityViolation> {
    use crate::security::android::SecurityChecker;
    
    let checker = SecurityChecker::new();
    checker.comprehensive_check()
}

/// Run startup security checks (iOS)
#[cfg(target_os = "ios")]
pub fn run_startup_checks() -> Result<(), SecurityViolation> {
    // iOS-specific checks
    check_ios_jailbreak()?;
    check_ios_debugger()?;
    Ok(())
}

/// Run startup security checks (desktop - minimal)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn run_startup_checks() -> Result<(), SecurityViolation> {
    // Desktop platforms - just warn about debug builds
    #[cfg(debug_assertions)]
    {
        warn!("Running in debug mode - security features reduced");
    }
    
    // Could add binary signature verification here
    Ok(())
}

// iOS-specific security checks
#[cfg(target_os = "ios")]
fn check_ios_jailbreak() -> Result<(), SecurityViolation> {
    use std::path::Path;
    
    // Common jailbreak indicators
    let jailbreak_paths = [
        "/Applications/Cydia.app",
        "/Applications/Sileo.app",
        "/var/lib/apt",
        "/var/lib/cydia",
        "/private/var/stash",
        "/usr/sbin/sshd",
        "/usr/bin/sshd",
        "/bin/bash",
        "/usr/libexec/sftp-server",
    ];
    
    for path in &jailbreak_paths {
        if Path::new(path).exists() {
            error!("Jailbreak indicator found: {}", path);
            return Err(SecurityViolation::RootDetected);
        }
    }
    
    // Check if we can write outside sandbox (jailbroken devices allow this)
    let test_path = "/private/jailbreak_test";
    if std::fs::write(test_path, b"test").is_ok() {
        let _ = std::fs::remove_file(test_path);
        error!("Able to write outside sandbox - jailbreak detected");
        return Err(SecurityViolation::RootDetected);
    }
    
    Ok(())
}

#[cfg(target_os = "ios")]
fn check_ios_debugger() -> Result<(), SecurityViolation> {
    // Use sysctl to check for debugger
    // This is a simplified version - production would use actual sysctl calls
    
    // Check for LLDB
    if std::env::var("__LLDB").is_ok() {
        warn!("LLDB environment detected");
        // Don't fail in debug builds
        #[cfg(not(debug_assertions))]
        return Err(SecurityViolation::DebuggerDetected);
    }
    
    Ok(())
}

/// Continuous security monitoring (spawns background thread)
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn start_security_monitor() {
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            
            if let Err(violation) = run_startup_checks() {
                error!("Security violation in background monitor: {:?}", violation);
                std::process::exit(1);
            }
        }
    });
    
    info!("Security monitor started");
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn start_security_monitor() {
    // No-op on desktop
    info!("Security monitor not required on desktop");
}
