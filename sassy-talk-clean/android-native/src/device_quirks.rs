// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-CBS5ONHLPWCN
//! Per-OEM audio tuning profiles. Ported from the Kotlin `audio/DeviceQuirks.kt`
//! so the Rust audio pipeline benefits from the same field-collected
//! workarounds. Profile selection happens once at startup based on
//! `Build.MANUFACTURER` + `Build.MODEL` + `Build.DEVICE`.
//!
//! Known bad behavior the profiles work around:
//!   - Motorola: aggressive OEM Noise Suppression + Dolby Atmos on STREAM_MUSIC
//!     mangle voice. Disable all hardware effects. Prefer MIC over
//!     VOICE_RECOGNITION (modem-path on G-series is flaky). Force comm-mode
//!     speaker routing. Larger recorder buffer to absorb HAL stalls.
//!   - Samsung: NS too aggressive on quiet speakers. Enable AGC only.
//!   - Xiaomi: MIUI Sound Enhance mangles voice; force comm-mode output.
//!
//! Reporting: emit the active profile's `notes` to telemetry on session
//! start so we can correlate complaints with profiles in the field.

use std::sync::OnceLock;

/// Standard `MediaRecorder.AudioSource` constants. Resolved at runtime from
/// the JVM if available; fall back to documented integers.
const SRC_DEFAULT: i32 = 0;
const SRC_MIC: i32 = 1;
const SRC_VOICE_RECOGNITION: i32 = 6;
const SRC_VOICE_COMMUNICATION: i32 = 7;

#[derive(Debug, Clone)]
pub struct EffectsConfig {
    pub enable_aec: bool,
    pub enable_ns: bool,
    pub enable_agc: bool,
}

impl Default for EffectsConfig {
    /// PTT defaults: AEC off (half-duplex doesn't need it), NS off (OEM NS
    /// too aggressive on average), AGC on (helps distance-to-mic variation).
    fn default() -> Self {
        Self { enable_aec: false, enable_ns: false, enable_agc: true }
    }
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub effects: EffectsConfig,
    /// Force `AudioManager.MODE_IN_COMMUNICATION` + speakerphone-on while
    /// the playback session is open. Bypasses STREAM_MUSIC OEM
    /// post-processing on devices that do it aggressively (Moto, Xiaomi).
    pub output_force_comm_mode: bool,
    /// Order in which `MediaRecorder.AudioSource` constants are tried until
    /// one yields a STATE_INITIALIZED `AudioRecord`.
    pub source_fallback_chain: Vec<i32>,
    /// Multiplier applied to `AudioRecord.getMinBufferSize()` for the
    /// recorder. Higher = more headroom against HAL stalls / TX thread
    /// stalls before sample drop. 4× is the safe default.
    pub record_buffer_multiplier: i32,
    /// Multiplier applied to `AudioTrack.getMinBufferSize()` for the
    /// player. 4× gives ~80–160 ms of headroom — enough to ride out a
    /// realistic jitter spike without adding meaningful latency.
    pub player_buffer_multiplier: i32,
    pub notes: &'static str,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            effects: EffectsConfig::default(),
            output_force_comm_mode: false,
            source_fallback_chain: default_source_chain(),
            record_buffer_multiplier: 4,
            player_buffer_multiplier: 4,
            notes: "Stock defaults.",
        }
    }
}

fn default_source_chain() -> Vec<i32> {
    vec![SRC_VOICE_RECOGNITION, SRC_MIC, SRC_DEFAULT]
}

static ACTIVE_PROFILE: OnceLock<Profile> = OnceLock::new();

/// Returns the active profile, initializing it from `Build.*` on first call.
pub fn current() -> &'static Profile {
    ACTIVE_PROFILE.get_or_init(|| {
        let (mfr, model, device) = read_build_props();
        let profile = pick_profile(&mfr, &model, &device);
        log::info!(
            "DeviceQuirks: manufacturer='{}' model='{}' device='{}' → {}",
            mfr, model, device, profile.notes
        );
        profile
    })
}

/// Force a specific profile (for tests). Has no effect after `current()`
/// has been called.
#[cfg(test)]
pub fn force_for_test(p: Profile) {
    let _ = ACTIVE_PROFILE.set(p);
}

fn pick_profile(mfr: &str, _model: &str, device: &str) -> Profile {
    let m = mfr.to_lowercase();
    let d = device.to_lowercase();
    if m.contains("motorola") || d.starts_with("moto") {
        moto_profile()
    } else if m.contains("samsung") {
        samsung_profile()
    } else if m.contains("xiaomi") || m.contains("redmi") {
        xiaomi_profile()
    } else {
        Profile::default()
    }
}

fn moto_profile() -> Profile {
    Profile {
        effects: EffectsConfig { enable_aec: false, enable_ns: false, enable_agc: false },
        output_force_comm_mode: true,
        // MIC first on Moto — VOICE_RECOGNITION routes through the modem
        // path which is flaky on G-series.
        source_fallback_chain: vec![SRC_MIC, SRC_VOICE_RECOGNITION, SRC_DEFAULT],
        record_buffer_multiplier: 6,
        player_buffer_multiplier: 6,
        notes: "Moto: disable all effects (HAL too aggressive). \
                Force comm-mode output to bypass Dolby Atmos. \
                Prefer MIC over VOICE_RECOGNITION (modem path is flaky on G series). \
                Larger record/player buffers to absorb HAL stalls.",
    }
}

fn samsung_profile() -> Profile {
    Profile {
        effects: EffectsConfig { enable_aec: false, enable_ns: false, enable_agc: true },
        output_force_comm_mode: false,
        notes: "Samsung: NS too aggressive on quiet speakers; AGC works fine.",
        ..Profile::default()
    }
}

fn xiaomi_profile() -> Profile {
    Profile {
        effects: EffectsConfig { enable_aec: false, enable_ns: false, enable_agc: true },
        output_force_comm_mode: true,
        notes: "Xiaomi: MIUI sound enhance mangles voice; force comm mode output.",
        ..Profile::default()
    }
}

fn read_build_props() -> (String, String, String) {
    // Read android.os.Build.{MANUFACTURER,MODEL,DEVICE} via JNI. If the
    // JVM is unavailable (running unit tests) return empty strings so the
    // default profile is picked.
    let vm = match crate::jni_bridge::get_jvm() {
        Ok(vm) => vm,
        Err(_) => return (String::new(), String::new(), String::new()),
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return (String::new(), String::new(), String::new()),
    };
    let build_class = match env.find_class("android/os/Build") {
        Ok(c) => c,
        Err(_) => return (String::new(), String::new(), String::new()),
    };
    let mfr = read_string_field(&mut env, &build_class, "MANUFACTURER");
    let model = read_string_field(&mut env, &build_class, "MODEL");
    let device = read_string_field(&mut env, &build_class, "DEVICE");
    (mfr, model, device)
}

fn read_string_field(env: &mut jni::JNIEnv, cls: &jni::objects::JClass, name: &str) -> String {
    let val = match env.get_static_field(cls, name, "Ljava/lang/String;") {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    let obj = match val.l() {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    let jstr = jni::objects::JString::from(obj);
    let s: String = env.get_string(&jstr)
        .map(|js| js.into())
        .unwrap_or_default();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_has_voice_recognition_first() {
        let p = Profile::default();
        assert_eq!(p.source_fallback_chain.first(), Some(&SRC_VOICE_RECOGNITION));
    }

    #[test]
    fn moto_picked_for_motorola_manufacturer() {
        let p = pick_profile("Motorola", "moto g power 5G", "obiwan");
        assert!(p.notes.starts_with("Moto"));
        assert_eq!(p.source_fallback_chain.first(), Some(&SRC_MIC));
    }

    #[test]
    fn moto_picked_for_moto_device_prefix() {
        let p = pick_profile("Generic", "G Stylus", "moto_xyz");
        assert!(p.notes.starts_with("Moto"));
    }

    #[test]
    fn samsung_picked_for_samsung_manufacturer() {
        let p = pick_profile("samsung", "SM-G998U", "g0q");
        assert!(p.notes.starts_with("Samsung"));
        assert!(p.effects.enable_agc);
        assert!(!p.effects.enable_ns);
    }

    #[test]
    fn xiaomi_forces_comm_mode() {
        let p = pick_profile("Xiaomi", "Mi 11", "venus");
        assert!(p.output_force_comm_mode);
    }

    #[test]
    fn unknown_oem_gets_default_profile() {
        let p = pick_profile("Acme", "Phone Plus", "acme1");
        assert!(p.notes.starts_with("Stock"));
    }
}
