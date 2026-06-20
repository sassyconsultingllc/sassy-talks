//! Audio output routing — engages `AudioManager.MODE_IN_COMMUNICATION` and
//! optionally forces speakerphone on, so received voice bypasses the OEM
//! media post-processing chain. Ported from the Kotlin
//! `audio/AudioOutputRouter.kt`.
//!
//! Lifecycle: call `engage_comm_mode` when the playback session opens;
//! call `release` to restore the prior mode and speakerphone state.

use jni::objects::{GlobalRef, JValue};
use log::{info, warn};
use std::sync::Mutex;

use crate::jni_bridge::get_jvm;

/// Cached AudioManager reference + saved state, so we can restore on
/// release. One global router — there's only one AudioManager per app.
static ROUTER: Mutex<Option<RouterState>> = Mutex::new(None);

struct RouterState {
    am: GlobalRef,
    saved_mode: i32,
    saved_speaker_on: bool,
    active: bool,
}

/// Cache the Android application Context (set during JNI init). Required to
/// obtain the system `AudioManager`. Without a Context, we can't engage
/// comm-mode routing — the function becomes a no-op.
static APP_CONTEXT: std::sync::OnceLock<GlobalRef> = std::sync::OnceLock::new();

pub fn init_context(global_ctx: GlobalRef) {
    let _ = APP_CONTEXT.set(global_ctx);
    info!("audio_routing: app context cached");
}

pub fn engage_comm_mode(force_speaker: bool) -> Result<(), String> {
    let vm = get_jvm()?;
    let mut env = vm.attach_current_thread().map_err(|e| format!("attach: {}", e))?;

    let ctx_ref = APP_CONTEXT.get().ok_or_else(|| {
        "audio_routing: app Context not initialized; call init_context() during nativeInit".to_string()
    })?;

    let audio_jstr = env.new_string("audio").map_err(|e| format!("new_string audio: {}", e))?;
    let am_obj = env.call_method(
        ctx_ref.as_obj(),
        "getSystemService",
        "(Ljava/lang/String;)Ljava/lang/Object;",
        &[JValue::Object(&audio_jstr.into())],
    )
        .map_err(|e| format!("getSystemService audio: {}", e))?
        .l()
        .map_err(|e| format!("getSystemService.l: {}", e))?;
    if am_obj.is_null() {
        return Err("getSystemService(audio) returned null".to_string());
    }
    let am_ref = env.new_global_ref(&am_obj).map_err(|e| format!("new_global_ref am: {}", e))?;

    // Read & save current mode and speakerphone state.
    let saved_mode = env.call_method(am_ref.as_obj(), "getMode", "()I", &[])
        .map_err(|e| format!("getMode: {}", e))?
        .i()
        .map_err(|e| format!("getMode.i: {}", e))?;
    let saved_speaker = env.call_method(am_ref.as_obj(), "isSpeakerphoneOn", "()Z", &[])
        .map_err(|e| format!("isSpeakerphoneOn: {}", e))?
        .z()
        .map_err(|e| format!("isSpeakerphoneOn.z: {}", e))?;

    let am_class = env.find_class("android/media/AudioManager")
        .map_err(|e| format!("find_class AudioManager: {}", e))?;
    let mode_in_comm = env.get_static_field(&am_class, "MODE_IN_COMMUNICATION", "I")
        .map_err(|e| format!("MODE_IN_COMMUNICATION: {}", e))?
        .i()
        .map_err(|e| format!("MODE_IN_COMMUNICATION.i: {}", e))?;

    env.call_method(am_ref.as_obj(), "setMode", "(I)V", &[JValue::Int(mode_in_comm)])
        .map_err(|e| format!("setMode: {}", e))?;
    env.call_method(
        am_ref.as_obj(),
        "setSpeakerphoneOn",
        "(Z)V",
        &[JValue::Bool(if force_speaker { jni::sys::JNI_TRUE } else { jni::sys::JNI_FALSE })],
    )
        .map_err(|e| format!("setSpeakerphoneOn: {}", e))?;

    let mut guard = ROUTER.lock().unwrap();
    *guard = Some(RouterState {
        am: am_ref,
        saved_mode,
        saved_speaker_on: saved_speaker,
        active: true,
    });
    info!("audio_routing: engaged MODE_IN_COMMUNICATION (speakerphone={})", force_speaker);
    Ok(())
}

pub fn release() {
    let mut guard = ROUTER.lock().unwrap();
    let state = match guard.take() {
        Some(s) if s.active => s,
        _ => return,
    };
    let vm = match get_jvm() {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };

    if let Err(e) = env.call_method(state.am.as_obj(), "setMode", "(I)V", &[JValue::Int(state.saved_mode)]) {
        warn!("audio_routing release: setMode failed: {}", e);
    }
    if let Err(e) = env.call_method(
        state.am.as_obj(),
        "setSpeakerphoneOn",
        "(Z)V",
        &[JValue::Bool(if state.saved_speaker_on { jni::sys::JNI_TRUE } else { jni::sys::JNI_FALSE })],
    ) {
        warn!("audio_routing release: setSpeakerphoneOn failed: {}", e);
    }
    info!("audio_routing: released, restored mode={}, speaker={}", state.saved_mode, state.saved_speaker_on);
}

pub fn is_active() -> bool {
    ROUTER.lock().unwrap().as_ref().map(|s| s.active).unwrap_or(false)
}
