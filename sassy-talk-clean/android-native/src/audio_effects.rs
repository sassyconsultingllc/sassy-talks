//! Hardware audio effects on the capture chain — AEC, NS, AGC. Ported
//! from the Kotlin `audio/AudioEffectsManager.kt`. Calls
//! `AcousticEchoCanceler.create(sessionId)`, `NoiseSuppressor.create`,
//! `AutomaticGainControl.create` via JNI on the recorder's
//! `audioSessionId`.
//!
//! Defaults for PTT (`DeviceQuirks` may override):
//!   AEC = OFF — half-duplex; AEC just adds processing artifacts
//!   NS  = OFF — aggressive OEM NS erases quiet voices
//!   AGC = ON  — levels out distance-to-mic variation
//!
//! Effects stay alive (and held via `GlobalRef`) for the lifetime of the
//! `AppliedEffects` value; drop it to release them.

use jni::objects::GlobalRef;
use log::{info, warn};

use crate::device_quirks::EffectsConfig;
use crate::jni_bridge::get_jvm;

pub struct AppliedEffects {
    _aec: Option<GlobalRef>,
    _ns: Option<GlobalRef>,
    _agc: Option<GlobalRef>,
    pub aec_active: bool,
    pub ns_active: bool,
    pub agc_active: bool,
}

impl AppliedEffects {
    pub fn none() -> Self {
        Self { _aec: None, _ns: None, _agc: None, aec_active: false, ns_active: false, agc_active: false }
    }
}

/// Apply `config` to the recorder identified by `session_id`. Returns the
/// effects that were actually active (some devices return `isAvailable()=true`
/// but enabled stays false — trust the applied state, not the config).
pub fn apply(session_id: i32, config: &EffectsConfig) -> AppliedEffects {
    let vm = match get_jvm() {
        Ok(v) => v,
        Err(_) => return AppliedEffects::none(),
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return AppliedEffects::none(),
    };

    let aec = if config.enable_aec {
        try_create_effect(&mut env, "android/media/audiofx/AcousticEchoCanceler", session_id)
    } else { None };
    let ns = if config.enable_ns {
        try_create_effect(&mut env, "android/media/audiofx/NoiseSuppressor", session_id)
    } else { None };
    let agc = if config.enable_agc {
        try_create_effect(&mut env, "android/media/audiofx/AutomaticGainControl", session_id)
    } else { None };

    let applied = AppliedEffects {
        aec_active: aec.is_some(),
        ns_active: ns.is_some(),
        agc_active: agc.is_some(),
        _aec: aec,
        _ns: ns,
        _agc: agc,
    };
    info!(
        "AudioEffects applied: AEC={} NS={} AGC={} (requested AEC={} NS={} AGC={})",
        applied.aec_active, applied.ns_active, applied.agc_active,
        config.enable_aec, config.enable_ns, config.enable_agc
    );
    applied
}

fn try_create_effect(env: &mut jni::JNIEnv, class_name: &str, session_id: i32) -> Option<GlobalRef> {
    use jni::objects::JValue;

    let cls = env.find_class(class_name).ok()?;

    // Static `isAvailable()` first — emulator and many OEM HALs return false.
    let avail = env.call_static_method(&cls, "isAvailable", "()Z", &[]).ok()?.z().ok()?;
    if !avail {
        warn!("AudioEffects: {} not available on this device", class_name);
        return None;
    }

    // Static `create(int sessionId)` returns the subclass or null.
    let sig = match class_name {
        "android/media/audiofx/AcousticEchoCanceler" => "(I)Landroid/media/audiofx/AcousticEchoCanceler;",
        "android/media/audiofx/NoiseSuppressor" => "(I)Landroid/media/audiofx/NoiseSuppressor;",
        "android/media/audiofx/AutomaticGainControl" => "(I)Landroid/media/audiofx/AutomaticGainControl;",
        _ => return None,
    };
    let obj = env.call_static_method(&cls, "create", sig, &[JValue::Int(session_id)]).ok()?.l().ok()?;
    if obj.is_null() {
        warn!("AudioEffects: {}.create returned null for sessionId={}", class_name, session_id);
        return None;
    }

    // setEnabled(true). The setter returns int (status), but we don't check
    // — the result of `isEnabled` is more reliable.
    let _ = env.call_method(&obj, "setEnabled", "(Z)I", &[JValue::Bool(jni::sys::JNI_TRUE)]);

    let enabled = env.call_method(&obj, "getEnabled", "()Z", &[])
        .ok()
        .and_then(|v| v.z().ok())
        .unwrap_or(false);
    if !enabled {
        warn!("AudioEffects: {} setEnabled stuck at false (driver may have rejected)", class_name);
        return None;
    }

    env.new_global_ref(&obj).ok()
}
