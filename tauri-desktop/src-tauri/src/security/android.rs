// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-KMHVILUIGU66
use super::SecurityViolation;
use log::{error, info, warn};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Expected APK signature hash (SHA-256)
/// Replace with actual signature after signing
const EXPECTED_SIGNATURE: [u8; 32] = [
    0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
    0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18,
];

pub struct SecurityChecker {
    startup_time: std::time::Instant,
}

impl SecurityChecker {
    pub fn new() -> Self {
        Self {
            startup_time: std::time::Instant::now(),
        }
    }

    /// Check for debugger via ptrace and TracerPid
    pub fn check_debugger(&self) -> Result<(), SecurityViolation> {
        // Method 1: Check TracerPid in /proc/self/status
        if let Ok(status) = fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("TracerPid:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 && parts[1] != "0" {
                        error!("Debugger detected via TracerPid: {}", parts[1]);
                        return Err(SecurityViolation::DebuggerDetected);
                    }
                }
            }
        }

        // Method 2: Timing attack - debuggers slow down execution
        let start = std::time::Instant::now();
        let mut x = 0u64;
        for i in 0..10000 {
            x = x.wrapping_add(i);
        }
        let elapsed = start.elapsed();

        // Normal execution should be < 1ms, debugger makes it much slower
        if elapsed.as_micros() > 5000 {
            warn!("Suspicious execution timing: {:?}", elapsed);
            // Don't immediately fail, but log it
        }

        // Method 3: Check for debug build markers
        #[cfg(debug_assertions)]
        {
            warn!("Running in debug mode");
        }

        Ok(())
    }

    /// Comprehensive root detection
    pub fn check_root(&self) -> Result<(), SecurityViolation> {
        // Check for su binaries in common locations
        let su_paths = [
            "/system/app/Superuser.apk",
            "/sbin/su",
            "/system/bin/su",
            "/system/xbin/su",
            "/data/local/xbin/su",
            "/data/local/bin/su",
            "/system/sd/xbin/su",
            "/system/bin/failsafe/su",
            "/data/local/su",
            "/su/bin/su",
            "/system/xbin/daemonsu",
            "/system/etc/init.d/99SuperSUDaemon",
            "/dev/com.koushikdutta.superuser.daemon/",
            "/system/app/SuperSU",
            "/system/app/SuperSU.apk",
        ];

        for path in &su_paths {
            if Path::new(path).exists() {
                error!("Root binary found: {}", path);
                return Err(SecurityViolation::RootDetected);
            }
        }

        // Check for Magisk
        if Path::new("/sbin/.magisk").exists() {
            error!("Magisk detected");
            return Err(SecurityViolation::RootDetected);
        }

        // Check for root management apps by looking for their data directories
        let root_app_paths = [
            "/data/data/com.noshufou.android.su",
            "/data/data/com.thirdparty.superuser",
            "/data/data/eu.chainfire.supersu",
            "/data/data/com.koushikdutta.superuser",
            "/data/data/com.topjohnwu.magisk",
            "/data/data/com.kingroot.kinguser",
        ];

        for path in &root_app_paths {
            if Path::new(path).exists() {
                error!("Root management app detected: {}", path);
                return Err(SecurityViolation::RootDetected);
            }
        }

        // Check build properties for root indicators
        if let Ok(build_prop) = fs::read_to_string("/system/build.prop") {
            if build_prop.contains("ro.debuggable=1") {
                warn!("Debuggable system detected");
            }
        }

        // Check for test-keys (unofficial ROMs)
        if let Ok(ro_build_tags) = std::env::var("ro.build.tags") {
            if ro_build_tags.contains("test-keys") {
                warn!("Test-keys build detected (custom ROM)");
                // Not always root, so just warn
            }
        }

        Ok(())
    }

    /// Emulator detection
    pub fn check_emulator(&self) -> Result<(), SecurityViolation> {
        // Check CPU info for emulator signatures
        if let Ok(cpu_info) = fs::read_to_string("/proc/cpuinfo") {
            let emulator_signatures = ["goldfish", "ranchu", "vbox", "qemu", "virtual"];

            for signature in &emulator_signatures {
                if cpu_info.to_lowercase().contains(signature) {
                    error!("Emulator signature detected in CPU info: {}", signature);
                    return Err(SecurityViolation::EmulatorDetected);
                }
            }
        }

        // Check device fingerprint
        let emulator_fingerprints = [
            "generic",
            "unknown",
            "emulator",
            "sdk",
            "google_sdk",
            "vbox",
            "genymotion",
        ];

        // Check for specific files that exist only on emulators
        let emulator_files = [
            "/dev/socket/qemud",
            "/dev/qemu_pipe",
            "/system/lib/libc_malloc_debug_qemu.so",
            "/sys/qemu_trace",
            "/system/bin/qemu-props",
        ];

        for file in &emulator_files {
            if Path::new(file).exists() {
                error!("Emulator file detected: {}", file);
                return Err(SecurityViolation::EmulatorDetected);
            }
        }

        Ok(())
    }

    /// Hook detection (Frida, Xposed, Substrate)
    pub fn check_hooks(&self) -> Result<(), SecurityViolation> {
        // Check /proc/self/maps for injected libraries
        if let Ok(maps) = fs::read_to_string("/proc/self/maps") {
            let hook_signatures = [
                "frida",
                "xposed",
                "substrate",
                "libhook",
                "injector",
                "frida-agent",
                "frida-gadget",
                "xhook",
            ];

            for signature in &hook_signatures {
                if maps.to_lowercase().contains(signature) {
                    error!("Hook framework detected: {}", signature);
                    return Err(SecurityViolation::HookDetected);
                }
            }
        }

        // Check for Frida ports (default: 27042, 27043)
        // This requires network permission, so we'll skip for now
        // Could implement with tokio tcp stream connection attempt

        // Check for common Frida artifacts in environment
        if let Ok(vars) = std::env::vars().collect::<Vec<_>>() {
            for (key, value) in vars {
                if key.contains("FRIDA") || value.contains("frida") {
                    error!("Frida environment variable detected");
                    return Err(SecurityViolation::HookDetected);
                }
            }
        }

        Ok(())
    }

    /// Verify APK signature hasn't been tampered
    pub fn verify_signature(&self) -> Result<(), SecurityViolation> {
        // In production, this would verify the actual APK signature
        // For now, we'll do a basic check

        // Note: Actual implementation would use Android's PackageManager
        // via JNI to get signature and verify it matches EXPECTED_SIGNATURE

        // Placeholder: Always pass for development
        // TODO: Implement proper signature verification via JNI

        info!("Signature verification: OK (placeholder)");
        Ok(())
    }

    /// Integrity check via checksum
    pub fn verify_integrity(&self) -> Result<(), SecurityViolation> {
        // Check if critical files have been modified
        // This is a simplified version - production would check all .so files

        // TODO: Calculate checksum of libsassytalkie.so and compare

        Ok(())
    }

    /// Anti-tampering: Check if memory has been modified
    pub fn check_memory_integrity(&self) -> Result<(), SecurityViolation> {
        // In native Rust, we can check if critical memory regions are modified
        // This is more effective than Java/Kotlin since we have direct memory access

        // TODO: Implement memory region checksums

        Ok(())
    }

    /// Comprehensive security check - runs all checks
    pub fn comprehensive_check(&self) -> Result<(), SecurityViolation> {
        self.check_debugger()?;
        self.check_root()?;
        self.check_emulator()?;
        self.check_hooks()?;
        self.verify_signature()?;
        self.verify_integrity()?;
        self.check_memory_integrity()?;
        Ok(())
    }
}

/// Additional security utilities

/// Obfuscated string decoder
pub fn decode_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

// NOTE: the former `encrypt_audio`/`decrypt_audio` (repeating-key XOR keyed by a
// hardcoded constant) and `generate_device_key` were removed. They provided zero
// confidentiality and zero authentication and were a standing footgun for any
// caller that reached for an "encrypt" helper here. Real audio encryption is
// AES-256-GCM via `sassytalkie_core::crypto::CryptoSession` (HKDF-derived key,
// counter nonces, anti-replay) — used through `security::CryptoEngine`. There is
// no plaintext-XOR path on purpose.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_checker() {
        let checker = SecurityChecker::new();

        // These should pass on a normal device
        assert!(checker.check_debugger().is_ok());
        // Root and emulator checks might fail in dev environment
    }
}
