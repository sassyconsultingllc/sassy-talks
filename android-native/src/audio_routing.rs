// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-J56NH25INVZ3
//! Audio output routing for RX.
//!
//! `USAGE_VOICE_COMMUNICATION` / `MODE_IN_COMMUNICATION` default to the
//! **earpiece**. Walkie RX must default to the **loudspeaker**. This module
//! re-asserts that route (setCommunicationDevice on API 31+, setSpeakerphoneOn
//! everywhere, AudioTrack.setPreferredDevice) unless a real wired / BT / USB
//! headset is connected.
//!
//! Sticky preference: loudspeaker unless Settings toggled earpiece, or an
//! external accessory is present. Re-assert is a no-op when already on the
//! desired sink so focus/packet events do not flap.

use jni::objects::{GlobalRef, JObject, JObjectArray, JValue};
use jni::JNIEnv;
use log::{info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::audio_route_policy::{self, Target};
use crate::jni_bridge::get_jvm;

/// Cached AudioManager reference + saved state, so we can restore on
/// release. One global router — there's only one AudioManager per app.
static ROUTER: Mutex<Option<RouterState>> = Mutex::new(None);

/// User preference: loudspeaker (true, default) vs earpiece (false).
/// External accessories still win over this flag.
static DESIRED_SPEAKER: AtomicBool = AtomicBool::new(true);

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

pub fn set_desired_speaker(on: bool) {
    DESIRED_SPEAKER.store(on, Ordering::Relaxed);
}

pub fn desired_speaker() -> bool {
    DESIRED_SPEAKER.load(Ordering::Relaxed)
}

/// Engage comm-mode and apply [force_speaker] as the sticky preference.
pub fn engage_comm_mode(force_speaker: bool) -> Result<(), String> {
    set_desired_speaker(force_speaker);
    apply_route()
}

/// Re-apply the sticky preference without changing it. Safe to call on every
/// RX start / focus / PTT-release / TTS / STT / route-change.
pub fn reassert() -> Result<(), String> {
    apply_route()
}

fn apply_route() -> Result<(), String> {
    let force_speaker = desired_speaker();
    let vm = get_jvm()?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("attach: {}", e))?;

    let am_ref = obtain_audio_manager(&mut env)?;

    let first_engage = ROUTER
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| !s.active)
        .unwrap_or(true);

    let saved_mode = if first_engage {
        env.call_method(am_ref.as_obj(), "getMode", "()I", &[])
            .map_err(|e| format!("getMode: {}", e))?
            .i()
            .map_err(|e| format!("getMode.i: {}", e))?
    } else {
        0
    };
    let saved_speaker = if first_engage {
        env.call_method(am_ref.as_obj(), "isSpeakerphoneOn", "()Z", &[])
            .map_err(|e| format!("isSpeakerphoneOn: {}", e))?
            .z()
            .unwrap_or(false)
    } else {
        false
    };

    let sdk = sdk_int(&mut env).unwrap_or(0);
    set_mode_in_communication(&mut env, am_ref.as_obj())?;

    let types = collect_output_types(&mut env, am_ref.as_obj()).unwrap_or_default();
    let target = audio_route_policy::resolve(force_speaker, &types);
    let current = current_output_type(&mut env, am_ref.as_obj(), sdk);

    if audio_route_policy::should_apply(target, current) {
        match target {
            Target::Loudspeaker => {
                apply_builtin(&mut env, am_ref.as_obj(), sdk, true)?;
                info!("audio_routing: RX → loudspeaker (sdk={})", sdk);
            }
            Target::Earpiece => {
                apply_builtin(&mut env, am_ref.as_obj(), sdk, false)?;
                info!(
                    "audio_routing: RX → earpiece (user preference, sdk={})",
                    sdk
                );
            }
            Target::External => {
                clear_forced_builtin(&mut env, am_ref.as_obj(), sdk);
                info!("audio_routing: RX → external accessory (left in place)");
            }
        }
    }

    if first_engage {
        let mut guard = ROUTER.lock().unwrap();
        *guard = Some(RouterState {
            am: am_ref,
            saved_mode,
            saved_speaker_on: saved_speaker,
            active: true,
        });
    }
    Ok(())
}

/// Pin an AudioTrack to the same RX sink (setPreferredDevice). More reliable
/// than AudioManager flags on OEMs that ignore setSpeakerphoneOn.
pub fn pin_track(track: &JObject) -> Result<(), String> {
    if track.is_null() {
        return Ok(());
    }
    let vm = get_jvm()?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("attach: {}", e))?;
    let am_ref = obtain_audio_manager(&mut env)?;
    let types = collect_output_types(&mut env, am_ref.as_obj()).unwrap_or_default();
    let target = audio_route_policy::resolve(desired_speaker(), &types);
    match target {
        Target::Loudspeaker => {
            if let Some(dev) = find_output_device(
                &mut env,
                am_ref.as_obj(),
                &[
                    audio_route_policy::TYPE_BUILTIN_SPEAKER,
                    audio_route_policy::TYPE_BUILTIN_SPEAKER_SAFE,
                ],
            ) {
                let _ = env.call_method(
                    track,
                    "setPreferredDevice",
                    "(Landroid/media/AudioDeviceInfo;)Z",
                    &[JValue::Object(&dev)],
                );
            }
        }
        Target::Earpiece => {
            if let Some(dev) = find_output_device(
                &mut env,
                am_ref.as_obj(),
                &[audio_route_policy::TYPE_BUILTIN_EARPIECE],
            ) {
                let _ = env.call_method(
                    track,
                    "setPreferredDevice",
                    "(Landroid/media/AudioDeviceInfo;)Z",
                    &[JValue::Object(&dev)],
                );
            }
        }
        Target::External => {
            let null_dev = JObject::null();
            let _ = env.call_method(
                track,
                "setPreferredDevice",
                "(Landroid/media/AudioDeviceInfo;)Z",
                &[JValue::Object(&null_dev)],
            );
        }
    }
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

    if let Err(e) = env.call_method(
        state.am.as_obj(),
        "setMode",
        "(I)V",
        &[JValue::Int(state.saved_mode)],
    ) {
        warn!("audio_routing release: setMode failed: {}", e);
    }
    let sdk = sdk_int(&mut env).unwrap_or(0);
    if sdk >= 31 {
        let _ = env.call_method(state.am.as_obj(), "clearCommunicationDevice", "()V", &[]);
    }
    if let Err(e) = env.call_method(
        state.am.as_obj(),
        "setSpeakerphoneOn",
        "(Z)V",
        &[JValue::Bool(if state.saved_speaker_on {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        })],
    ) {
        warn!("audio_routing release: setSpeakerphoneOn failed: {}", e);
    }
    info!(
        "audio_routing: released, restored mode={}, speaker={}",
        state.saved_mode, state.saved_speaker_on
    );
}

pub fn is_active() -> bool {
    ROUTER
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.active)
        .unwrap_or(false)
}

fn obtain_audio_manager(env: &mut JNIEnv) -> Result<GlobalRef, String> {
    let ctx_ref = APP_CONTEXT.get().ok_or_else(|| {
        "audio_routing: app Context not initialized; call init_context() during nativeInit"
            .to_string()
    })?;
    let audio_jstr = env
        .new_string("audio")
        .map_err(|e| format!("new_string audio: {}", e))?;
    let am_obj = env
        .call_method(
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
    env.new_global_ref(&am_obj)
        .map_err(|e| format!("new_global_ref am: {}", e))
}

fn sdk_int(env: &mut JNIEnv) -> Result<i32, String> {
    let cls = env
        .find_class("android/os/Build$VERSION")
        .map_err(|e| format!("Build.VERSION: {}", e))?;
    env.get_static_field(&cls, "SDK_INT", "I")
        .map_err(|e| format!("SDK_INT: {}", e))?
        .i()
        .map_err(|e| format!("SDK_INT.i: {}", e))
}

fn set_mode_in_communication(env: &mut JNIEnv, am: &JObject) -> Result<(), String> {
    let am_class = env
        .find_class("android/media/AudioManager")
        .map_err(|e| format!("find_class AudioManager: {}", e))?;
    let mode_in_comm = env
        .get_static_field(&am_class, "MODE_IN_COMMUNICATION", "I")
        .map_err(|e| format!("MODE_IN_COMMUNICATION: {}", e))?
        .i()
        .map_err(|e| format!("MODE_IN_COMMUNICATION.i: {}", e))?;
    env.call_method(am, "setMode", "(I)V", &[JValue::Int(mode_in_comm)])
        .map_err(|e| format!("setMode: {}", e))?;
    Ok(())
}

fn collect_output_types(env: &mut JNIEnv, am: &JObject) -> Result<Vec<i32>, String> {
    // AudioManager.GET_DEVICES_OUTPUTS = 2 (API 23+)
    let arr_obj = env
        .call_method(
            am,
            "getDevices",
            "(I)[Landroid/media/AudioDeviceInfo;",
            &[JValue::Int(2)],
        )
        .map_err(|e| format!("getDevices: {}", e))?
        .l()
        .map_err(|e| format!("getDevices.l: {}", e))?;
    if arr_obj.is_null() {
        return Ok(Vec::new());
    }
    let arr = JObjectArray::from(arr_obj);
    let n = env
        .get_array_length(&arr)
        .map_err(|e| format!("get_array_length: {}", e))?;
    let mut types = Vec::with_capacity(n as usize);
    for i in 0..n {
        let dev = env
            .get_object_array_element(&arr, i)
            .map_err(|e| format!("get_object_array_element: {}", e))?;
        if dev.is_null() {
            continue;
        }
        let t = env
            .call_method(&dev, "getType", "()I", &[])
            .map_err(|e| format!("getType: {}", e))?
            .i()
            .unwrap_or(0);
        types.push(t);
    }
    Ok(types)
}

fn current_output_type(env: &mut JNIEnv, am: &JObject, sdk: i32) -> Option<i32> {
    if sdk >= 31 {
        let dev = env
            .call_method(
                am,
                "getCommunicationDevice",
                "()Landroid/media/AudioDeviceInfo;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        if dev.is_null() {
            return None;
        }
        return env.call_method(&dev, "getType", "()I", &[]).ok()?.i().ok();
    }
    let speaker_on = env
        .call_method(am, "isSpeakerphoneOn", "()Z", &[])
        .ok()?
        .z()
        .ok()?;
    if speaker_on {
        Some(audio_route_policy::TYPE_BUILTIN_SPEAKER)
    } else {
        Some(audio_route_policy::TYPE_BUILTIN_EARPIECE)
    }
}

fn find_output_device<'a>(
    env: &mut JNIEnv<'a>,
    am: &JObject,
    wanted: &[i32],
) -> Option<JObject<'a>> {
    let arr_obj = env
        .call_method(
            am,
            "getDevices",
            "(I)[Landroid/media/AudioDeviceInfo;",
            &[JValue::Int(2)],
        )
        .ok()?
        .l()
        .ok()?;
    if arr_obj.is_null() {
        return None;
    }
    let arr = JObjectArray::from(arr_obj);
    let n = env.get_array_length(&arr).ok()?;
    for i in 0..n {
        let dev = env.get_object_array_element(&arr, i).ok()?;
        if dev.is_null() {
            continue;
        }
        let t = env
            .call_method(&dev, "getType", "()I", &[])
            .ok()?
            .i()
            .ok()?;
        if wanted.contains(&t) {
            return Some(dev);
        }
    }
    None
}

fn apply_builtin(env: &mut JNIEnv, am: &JObject, sdk: i32, speaker: bool) -> Result<(), String> {
    if sdk >= 31 {
        let wanted = if speaker {
            [
                audio_route_policy::TYPE_BUILTIN_SPEAKER,
                audio_route_policy::TYPE_BUILTIN_SPEAKER_SAFE,
            ]
        } else {
            [
                audio_route_policy::TYPE_BUILTIN_EARPIECE,
                audio_route_policy::TYPE_BUILTIN_EARPIECE,
            ]
        };
        if let Some(dev) = find_output_device(env, am, &wanted) {
            let ok = env.call_method(
                am,
                "setCommunicationDevice",
                "(Landroid/media/AudioDeviceInfo;)Z",
                &[JValue::Object(&dev)],
            );
            match ok {
                Ok(v) => {
                    if v.z().unwrap_or(false) {
                        // also poke the legacy flag — some OEMs still honor it
                        let _ = set_speakerphone(env, am, speaker);
                        return Ok(());
                    }
                    warn!("audio_routing: setCommunicationDevice returned false");
                }
                Err(e) => warn!("audio_routing: setCommunicationDevice: {}", e),
            }
        } else {
            warn!(
                "audio_routing: no built-in {} device listed",
                if speaker { "speaker" } else { "earpiece" }
            );
        }
    }
    set_speakerphone(env, am, speaker)
}

fn set_speakerphone(env: &mut JNIEnv, am: &JObject, on: bool) -> Result<(), String> {
    env.call_method(
        am,
        "setSpeakerphoneOn",
        "(Z)V",
        &[JValue::Bool(if on {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        })],
    )
    .map_err(|e| format!("setSpeakerphoneOn: {}", e))?;
    Ok(())
}

fn clear_forced_builtin(env: &mut JNIEnv, am: &JObject, sdk: i32) {
    if sdk >= 31 {
        if let Err(e) = env.call_method(am, "clearCommunicationDevice", "()V", &[]) {
            warn!("audio_routing: clearCommunicationDevice: {}", e);
        }
    }
    let _ = set_speakerphone(env, am, false);
}
