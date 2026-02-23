/// JNI Bridge Module - Connects Rust to Android APIs
///
/// This module provides safe Rust wrappers around Android Java APIs via JNI.
/// Implements bridges for: Audio, PackageManager, UI

use jni::{
    JNIEnv,
    objects::{JClass, JObject, JString, JValue, GlobalRef},
    sys::{jboolean, jbyte, JNI_TRUE, JNI_FALSE},
    JavaVM,
};
use std::sync::Arc;
use log::{error, info, warn};

/// Global JavaVM instance (initialized once, thread-safe)
static JAVA_VM: std::sync::OnceLock<Arc<JavaVM>> = std::sync::OnceLock::new();

/// Initialize global JavaVM reference
pub fn init_jvm(vm: JavaVM) {
    let _ = JAVA_VM.set(Arc::new(vm));
    info!("JNI: JavaVM initialized");
}

/// Get JavaVM instance
pub fn get_jvm() -> Result<Arc<JavaVM>, String> {
    JAVA_VM.get().cloned().ok_or_else(|| "JavaVM not initialized".to_string())
}

//==============================================================================
// AUDIO JNI BRIDGE
//==============================================================================

/// Android AudioRecord bridge
pub struct AndroidAudioRecord {
    recorder: GlobalRef,
}

impl AndroidAudioRecord {
    /// Create AudioRecord instance
    pub fn new(sample_rate: i32, channel_config: i32, audio_format: i32, buffer_size: i32) -> Result<Self, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let recorder_class = env.find_class("android/media/AudioRecord")
            .map_err(|e| format!("Failed to find AudioRecord class: {}", e))?;
        
        let source_class = env.find_class("android/media/MediaRecorder$AudioSource")
            .map_err(|e| format!("Failed to find AudioSource class: {}", e))?;
        
        let mic_field = env.get_static_field(&source_class, "MIC", "I")
            .map_err(|e| format!("Failed to get MIC field: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        
        let recorder = env.new_object(
            &recorder_class,
            "(IIIII)V",
            &[
                JValue::Int(mic_field),
                JValue::Int(sample_rate),
                JValue::Int(channel_config),
                JValue::Int(audio_format),
                JValue::Int(buffer_size),
            ]
        )
        .map_err(|e| format!("Failed to create AudioRecord: {}", e))?;
        
        let global_ref = env.new_global_ref(&recorder)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(Self { recorder: global_ref })
    }
    
    /// Get minimum buffer size
    pub fn get_min_buffer_size(sample_rate: i32, channel_config: i32, audio_format: i32) -> Result<i32, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let recorder_class = env.find_class("android/media/AudioRecord")
            .map_err(|e| format!("Failed to find AudioRecord class: {}", e))?;
        
        let size = env.call_static_method(
            recorder_class,
            "getMinBufferSize",
            "(III)I",
            &[
                JValue::Int(sample_rate),
                JValue::Int(channel_config),
                JValue::Int(audio_format),
            ]
        )
        .map_err(|e| format!("Failed to get min buffer size: {}", e))?
        .i()
        .map_err(|e| format!("Failed to convert result: {}", e))?;
        
        Ok(size)
    }
    
    /// Start recording
    pub fn start_recording(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.recorder.as_obj(), "startRecording", "()V", &[])
            .map_err(|e| format!("Failed to start recording: {}", e))?;
        
        Ok(())
    }
    
    /// Stop recording
    pub fn stop(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.recorder.as_obj(), "stop", "()V", &[])
            .map_err(|e| format!("Failed to stop recording: {}", e))?;
        
        Ok(())
    }
    
    /// Read audio data
    pub fn read(&self, buffer: &mut [i16]) -> Result<usize, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let jarray = env.new_short_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create short array: {}", e))?;
        
        // Create JObject reference without consuming jarray
        let jarray_obj = unsafe { JObject::from_raw(jarray.as_raw()) };
        
        let bytes_read = env.call_method(
            self.recorder.as_obj(),
            "read",
            "([SII)I",
            &[
                JValue::Object(&jarray_obj),
                JValue::Int(0),
                JValue::Int(buffer.len() as i32),
            ]
        )
        .map_err(|e| format!("Failed to read: {}", e))?
        .i()
        .map_err(|e| format!("Failed to convert result: {}", e))?;
        
        if bytes_read <= 0 {
            return Ok(0);
        }
        
        env.get_short_array_region(&jarray, 0, buffer)
            .map_err(|e| format!("Failed to copy shorts: {}", e))?;
        
        Ok(bytes_read as usize)
    }
    
    /// Release resources
    pub fn release(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.recorder.as_obj(), "release", "()V", &[])
            .map_err(|e| format!("Failed to release: {}", e))?;
        
        Ok(())
    }
}

/// Android AudioTrack bridge
pub struct AndroidAudioTrack {
    track: GlobalRef,
}

impl AndroidAudioTrack {
    /// Create AudioTrack instance
    pub fn new(sample_rate: i32, channel_config: i32, audio_format: i32, buffer_size: i32) -> Result<Self, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let track_class = env.find_class("android/media/AudioTrack")
            .map_err(|e| format!("Failed to find AudioTrack class: {}", e))?;
        
        let manager_class = env.find_class("android/media/AudioManager")
            .map_err(|e| format!("Failed to find AudioManager class: {}", e))?;
        
        let stream_music = env.get_static_field(&manager_class, "STREAM_MUSIC", "I")
            .map_err(|e| format!("Failed to get STREAM_MUSIC field: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        
        let mode_stream = env.get_static_field(&track_class, "MODE_STREAM", "I")
            .map_err(|e| format!("Failed to get MODE_STREAM field: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        
        let track = env.new_object(
            &track_class,
            "(IIIIII)V",
            &[
                JValue::Int(stream_music),
                JValue::Int(sample_rate),
                JValue::Int(channel_config),
                JValue::Int(audio_format),
                JValue::Int(buffer_size),
                JValue::Int(mode_stream),
            ]
        )
        .map_err(|e| format!("Failed to create AudioTrack: {}", e))?;
        
        let global_ref = env.new_global_ref(&track)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(Self { track: global_ref })
    }
    
    /// Start playback
    pub fn play(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.track.as_obj(), "play", "()V", &[])
            .map_err(|e| format!("Failed to start playback: {}", e))?;
        
        Ok(())
    }
    
    /// Stop playback
    pub fn stop(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.track.as_obj(), "stop", "()V", &[])
            .map_err(|e| format!("Failed to stop playback: {}", e))?;
        
        Ok(())
    }
    
    /// Write audio data
    pub fn write(&self, buffer: &[i16]) -> Result<usize, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let jarray = env.new_short_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create short array: {}", e))?;
        
        env.set_short_array_region(&jarray, 0, buffer)
            .map_err(|e| format!("Failed to copy shorts: {}", e))?;
        
        let bytes_written = env.call_method(
            self.track.as_obj(),
            "write",
            "([SII)I",
            &[
                JValue::Object(&jarray.into()),
                JValue::Int(0),
                JValue::Int(buffer.len() as i32),
            ]
        )
        .map_err(|e| format!("Failed to write: {}", e))?
        .i()
        .map_err(|e| format!("Failed to convert result: {}", e))?;
        
        Ok(bytes_written as usize)
    }
    
    /// Release resources
    pub fn release(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.track.as_obj(), "release", "()V", &[])
            .map_err(|e| format!("Failed to release: {}", e))?;
        
        Ok(())
    }
}

//==============================================================================
// JNI EXPORTS FOR KOTLIN/COMPOSE APP
//==============================================================================

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::sync::Mutex;

use crate::state::StateMachine;
use crate::session::SessionManager;
use crate::users::UserRegistry;

/// Global state for JNI mode (when used from Kotlin instead of egui)
static JNI_STATE: OnceLock<Arc<Mutex<JniAppState>>> = OnceLock::new();

struct JniAppState {
    state_machine: Option<StateMachine>,
    session_manager: SessionManager,
    user_registry: UserRegistry,
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    pending_key_exchange: Option<crate::crypto::KeyExchange>,
}

impl JniAppState {
    fn new() -> Self {
        let ptt_pressed = Arc::new(AtomicBool::new(false));
        let current_channel = Arc::new(AtomicU8::new(1));

        Self {
            state_machine: None,
            session_manager: SessionManager::new("SassyTalkie"),
            user_registry: UserRegistry::new(),
            ptt_pressed,
            current_channel,
            pending_key_exchange: None,
        }
    }

    fn initialize(&mut self) -> bool {
        info!("JNI: Initializing backend");

        let state_machine = StateMachine::new(
            Arc::clone(&self.ptt_pressed),
            Arc::clone(&self.current_channel),
        );

        match state_machine.initialize() {
            Ok(()) => {
                self.state_machine = Some(state_machine);
                info!("JNI: Backend initialized successfully");
                true
            }
            Err(e) => {
                error!("JNI: Failed to initialize: {}", e);
                false
            }
        }
    }
}

fn get_jni_state() -> &'static Arc<Mutex<JniAppState>> {
    JNI_STATE.get_or_init(|| Arc::new(Mutex::new(JniAppState::new())))
}

/// JNI: Initialize native backend
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeInit(
    env: JNIEnv,
    _class: JClass,
) -> jboolean {
    // Initialize logging
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("SassyTalk-JNI"),
    );
    
    info!("=== Sassy-Talk JNI Initializing ===");
    
    // Initialize JVM for JNI bridge
    if let Ok(vm) = env.get_java_vm() {
        init_jvm(vm);
        info!("JNI: JVM initialized");
    } else {
        error!("JNI: Failed to get JavaVM");
        return JNI_FALSE;
    }
    
    // Initialize app state
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    
    if guard.initialize() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// JNI: Start PTT transmission
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativePttStart(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: PTT Start");
    
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    
    guard.ptt_pressed.store(true, Ordering::Relaxed);
    
    if let Some(ref sm) = guard.state_machine {
        if let Err(e) = sm.on_ptt_press() {
            error!("JNI: Failed to start transmit: {}", e);
        }
    }
}

/// JNI: Stop PTT transmission
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativePttStop(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: PTT Stop");
    
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    
    guard.ptt_pressed.store(false, Ordering::Relaxed);
    
    if let Some(ref sm) = guard.state_machine {
        if let Err(e) = sm.on_ptt_release() {
            error!("JNI: Failed to stop transmit: {}", e);
        }
    }
}

/// JNI: Set channel
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetChannel(
    _env: JNIEnv,
    _class: JClass,
    channel: jbyte,
) {
    let ch = channel as u8;
    info!("JNI: Set channel to {}", ch);

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    guard.current_channel.store(ch, Ordering::Relaxed);
}

/// JNI: Get active transport type (0=None, 2=WiFi, 3=WifiDirect, 4=Cellular)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetTransport(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.get_active_transport() {
            crate::transport::ActiveTransport::None => 0,
            crate::transport::ActiveTransport::Wifi => 2,
            crate::transport::ActiveTransport::WifiDirect => 3,
            crate::transport::ActiveTransport::Cellular => 4,
        }
    } else {
        0
    }
}

/// JNI: Shutdown native backend
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeShutdown(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: Shutdown");

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let _ = sm.shutdown();
    }
    guard.state_machine = None;
}

/// JNI: Disconnect from current device
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeDisconnect(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    info!("JNI: Disconnect");

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.disconnect() {
            Ok(()) => JNI_TRUE,
            Err(e) => {
                error!("JNI: Disconnect failed: {}", e);
                JNI_FALSE
            }
        }
    } else {
        JNI_FALSE
    }
}

//==============================================================================
// SESSION / QR AUTH JNI EXPORTS
//==============================================================================

/// JNI: Generate a session QR code payload
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGenerateSessionQR<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    duration_hours: jni::sys::jint,
) -> JObject<'local> {
    info!("JNI: Generate session QR ({}h)", duration_hours);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = match guard.session_manager.generate_session_qr(duration_hours as u32) {
        Ok(qr_json) => {
            // Also set the crypto session on the transport
            if let Ok(crypto) = guard.session_manager.get_crypto_session() {
                if let Some(ref sm) = guard.state_machine {
                    sm.set_crypto_session(crypto);
                }
            }
            qr_json
        }
        Err(e) => {
            error!("JNI: Generate QR failed: {}", e);
            String::new()
        }
    };

    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Import a session from scanned QR code
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeImportSessionFromQR<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    qr_json: JString<'local>,
) -> jboolean {
    let json: String = match env.get_string(&qr_json) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    info!("JNI: Import session from QR");

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    match guard.session_manager.import_session(&json) {
        Ok(crypto) => {
            if let Some(ref sm) = guard.state_machine {
                sm.set_crypto_session(crypto);
            }
            info!("JNI: Session imported successfully");
            JNI_TRUE
        }
        Err(e) => {
            error!("JNI: Import session failed: {}", e);
            JNI_FALSE
        }
    }
}

/// JNI: Check if authenticated (valid session exists)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsAuthenticated(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if guard.session_manager.is_authenticated() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// JNI: Get session status as JSON
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetSessionStatus<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = guard.session_manager.get_session_status();
    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

//==============================================================================
// USER MANAGEMENT JNI EXPORTS (MUTE / FAVORITES)
//==============================================================================

/// JNI: Get all known users as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetUsers<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = guard.user_registry.to_json();
    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Set user mute status
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetMuted<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    user_id: JString<'local>,
    muted: jboolean,
) {
    let id: String = match env.get_string(&user_id) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    guard.user_registry.set_muted(&id, muted == JNI_TRUE);
}

/// JNI: Set user favorite status
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetFavorite<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    user_id: JString<'local>,
    favorite: jboolean,
) {
    let id: String = match env.get_string(&user_id) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    guard.user_registry.set_favorite(&id, favorite == JNI_TRUE);
}

//==============================================================================
// EXTENDED JNI EXPORTS - BT/WiFi status, permissions, user registration
//==============================================================================

/// JNI: Get app state (0=Init, 1=Ready, 2=Connecting, 3=Connected, 4=TX, 5=RX, 6=Disconnecting, 7=Error)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetAppState(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.get_state() {
            crate::state::AppState::Initializing => 0,
            crate::state::AppState::Ready => 1,
            crate::state::AppState::Connecting => 2,
            crate::state::AppState::Connected => 3,
            crate::state::AppState::Transmitting => 4,
            crate::state::AppState::Receiving => 5,
            crate::state::AppState::Disconnecting => 6,
            crate::state::AppState::Error => 7,
        }
    } else {
        0
    }
}

/// JNI: Clear the active session
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeClearSession(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: Clear session");

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    guard.session_manager.clear_session();
}

/// JNI: Register a user in the registry (called when a peer connects)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeRegisterUser<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    user_id: JString<'local>,
    user_name: JString<'local>,
) {
    let id: String = match env.get_string(&user_id) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let name: String = match env.get_string(&user_name) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    guard.user_registry.register_user(&id, &name);

    // Also check muted/favorite status for logging
    let is_muted = guard.user_registry.is_muted(&id);
    let is_fav = guard.user_registry.is_favorite(&id);
    info!("JNI: Registered user {} ({}) muted={} fav={}", name, id, is_muted, is_fav);
}

/// JNI: Get favorites as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetFavorites<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let favs = guard.user_registry.favorites();
    let others = guard.user_registry.others();

    let json = serde_json::json!({
        "favorites": favs,
        "others": others,
    }).to_string();

    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Derive user ID from session key (for consistent identity)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeDeriveUserId<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    session_key_b64: JString<'local>,
) -> JObject<'local> {
    let key_b64: String = match env.get_string(&session_key_b64) {
        Ok(s) => s.into(),
        Err(_) => return JObject::null(),
    };

    let key_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &key_b64,
    ) {
        Ok(b) => b,
        Err(_) => return JObject::null(),
    };

    let user_id = crate::users::UserRegistry::derive_user_id(&key_bytes);

    env.new_string(&user_id)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Generate a fresh pre-shared key (base64 encoded)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGeneratePsk<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let psk = crate::crypto::generate_psk();
    let psk_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &psk,
    );

    env.new_string(&psk_b64)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Set encryption from a pre-shared key (base64 encoded)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetPsk<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    psk_b64: JString<'local>,
) -> jboolean {
    let key_b64: String = match env.get_string(&psk_b64) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    let key_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &key_b64,
    ) {
        Ok(b) if b.len() == 32 => b,
        Ok(b) => {
            error!("JNI: PSK wrong length: {} (expected 32)", b.len());
            return JNI_FALSE;
        }
        Err(e) => {
            error!("JNI: PSK decode failed: {}", e);
            return JNI_FALSE;
        }
    };

    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&key_bytes);

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        sm.set_psk(&key_array);
        info!("JNI: PSK encryption set");
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// JNI: Check permissions via Android runtime (returns JSON with status)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCheckPermissions<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let mut pm = crate::permissions::PermissionManager::new();
    let all_granted = pm.check_permissions();

    let perms = pm.get_permissions();
    let json = serde_json::json!({
        "all_granted": all_granted,
        "record_audio": format!("{:?}", perms.record_audio),
        "has_critical": pm.has_critical_permissions(),
    }).to_string();

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Handle a permission result callback from the Activity
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeOnPermissionResult<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    permission: JString<'local>,
    granted: jboolean,
) {
    let perm: String = match env.get_string(&permission) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    let mut pm = crate::permissions::PermissionManager::new();
    pm.on_permission_result(&perm, granted == JNI_TRUE);

    let explanation = pm.get_permission_explanation(&perm);
    info!("JNI: Permission {} = {} ({})", perm, granted == JNI_TRUE, explanation);
}

/// JNI: Get WiFi transport state (0=Inactive, 1=Discovering, 2=Active, 3=Error)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetWifiState(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.get_wifi_state() {
            crate::wifi_transport::WifiState::Inactive => 0,
            crate::wifi_transport::WifiState::Discovering => 1,
            crate::wifi_transport::WifiState::Active => 2,
            crate::wifi_transport::WifiState::Error => 3,
        }
    } else {
        0
    }
}

/// JNI: Start ECDH key exchange - returns local public key as base64
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeKeyExchangeInit<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    info!("JNI: Key exchange init");

    let kx = crate::crypto::KeyExchange::new();
    let pub_key = kx.public_key_bytes();
    let pub_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &pub_key,
    );

    // Store the key exchange in JNI state for completion
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.pending_key_exchange = Some(kx);

    drop(guard);

    env.new_string(&pub_b64)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Complete ECDH key exchange with remote public key (base64)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeKeyExchangeComplete<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    remote_pub_b64: JString<'local>,
) -> jboolean {
    let remote_b64: String = match env.get_string(&remote_pub_b64) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    let remote_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &remote_b64,
    ) {
        Ok(b) if b.len() == 32 => b,
        Ok(b) => {
            error!("JNI: Remote pubkey wrong length: {} (expected 32)", b.len());
            return JNI_FALSE;
        }
        Err(e) => {
            error!("JNI: Remote pubkey decode failed: {}", e);
            return JNI_FALSE;
        }
    };

    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&remote_bytes);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let kx = match guard.pending_key_exchange.take() {
        Some(kx) => kx,
        None => {
            error!("JNI: No pending key exchange (call nativeKeyExchangeInit first)");
            return JNI_FALSE;
        }
    };

    match kx.complete(&key_array) {
        Ok(crypto) => {
            if let Some(ref sm) = guard.state_machine {
                sm.set_crypto_session(crypto);
            }
            info!("JNI: ECDH key exchange completed successfully");
            JNI_TRUE
        }
        Err(e) => {
            error!("JNI: Key exchange failed: {}", e);
            JNI_FALSE
        }
    }
}

/// JNI: Get missing permissions as JSON array of strings
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetMissingPermissions<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let pm = crate::permissions::PermissionManager::new();
    let missing = pm.request_permissions();

    let json = serde_json::to_string(&missing).unwrap_or_else(|_| "[]".to_string());

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Get permission rationale explanation for a specific permission
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetPermissionRationale<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    permission: JString<'local>,
) -> JObject<'local> {
    let perm: String = match env.get_string(&permission) {
        Ok(s) => s.into(),
        Err(_) => return JObject::null(),
    };

    let explanation = crate::permissions::show_permission_rationale(&perm);

    env.new_string(&explanation)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Get WiFi peers as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetWifiPeers<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = if let Some(ref sm) = guard.state_machine {
        let peers = sm.get_wifi_peers();
        let arr: Vec<serde_json::Value> = peers.iter().map(|p| {
            serde_json::json!({
                "address": p.address.to_string(),
                "device_name": p.device_name,
                "channel": p.channel,
            })
        }).collect();
        serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
    } else {
        "[]".to_string()
    };

    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Check if PTT is currently active
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsPttActive(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        if sm.is_ptt_active() { JNI_TRUE } else { JNI_FALSE }
    } else {
        JNI_FALSE
    }
}

/// JNI: Initialize WiFi transport explicitly
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeInitWifi(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.init_wifi() {
            Ok(_) => {
                info!("JNI: WiFi transport initialized");
                JNI_TRUE
            }
            Err(e) => {
                error!("JNI: WiFi init failed: {}", e);
                JNI_FALSE
            }
        }
    } else {
        JNI_FALSE
    }
}

/// JNI: Get device name from transport manager
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetDeviceName<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let name = if let Some(ref sm) = guard.state_machine {
        sm.get_device_name()
    } else {
        "Unknown".to_string()
    };

    drop(guard);

    env.new_string(&name)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Set device display name (called from Kotlin with the actual Android device model)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetDeviceName(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
) {
    let name_str: String = match env.get_string(&name) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    info!("JNI: Set device name to '{}'", name_str);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref mut sm) = guard.state_machine {
        sm.set_device_name(name_str);
    }
}

//==============================================================================
// AUDIO CACHE JNI EXPORTS (DANE.COM-STYLE MULTI-SPEAKER REPLAY)
//==============================================================================

/// JNI: Get audio cache status as JSON
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetCacheStatus<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = if let Some(ref sm) = guard.state_machine {
        let cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.status_json()
    } else {
        r#"{"mode":"Live","queued_utterances":0}"#.to_string()
    };

    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Skip current utterance in playback queue
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSkipCurrentUtterance(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: Skip current utterance");

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let mut cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.skip_current();
    }
}

/// JNI: Set audio cache mode (0=Live, 1=Queue, 2=Replay)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetCacheMode(
    _env: JNIEnv,
    _class: JClass,
    mode: jbyte,
) {
    let cache_mode = match mode {
        0 => crate::audio_cache::CacheMode::Live,
        1 => crate::audio_cache::CacheMode::Queue,
        2 => crate::audio_cache::CacheMode::Replay,
        _ => return,
    };

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let mut cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.set_mode(cache_mode);
    }
}

/// JNI: Clear all cached audio
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeClearAudioCache(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: Clear audio cache");

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let mut cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.clear();
    }
}

/// JNI: Replay an utterance from history by index
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeReplayUtterance(
    _env: JNIEnv,
    _class: JClass,
    index: jni::sys::jint,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let mut cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.replay_from_history(index as usize) {
            info!("JNI: Replaying utterance at index {}", index);
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    } else {
        JNI_FALSE
    }
}

/// JNI: Update user info in the audio cache (sync from UserRegistry)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSyncCacheUserInfo(
    _env: JNIEnv,
    _class: JClass,
) {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let mut cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());

        // Parse user registry JSON to sync mute/favorite status into cache
        let users_json = guard.user_registry.to_json();
        if let Ok(users) = serde_json::from_str::<Vec<serde_json::Value>>(&users_json) {
            for u in users {
                if let (Some(id), Some(name), Some(muted), Some(fav)) = (
                    u["id"].as_str(),
                    u["name"].as_str(),
                    u["is_muted"].as_bool(),
                    u["is_favorite"].as_bool(),
                ) {
                    cache.update_user_info(id, name, fav, muted);
                }
            }
        }
    }
}

/// JNI: Check if encryption is active (QR auth completed)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsEncrypted(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        if sm.is_encrypted() { JNI_TRUE } else { JNI_FALSE }
    } else {
        JNI_FALSE
    }
}

//==============================================================================
// CELLULAR TRANSPORT JNI EXPORTS (WebSocket relay via Cloudflare)
//==============================================================================

/// JNI: Set the cellular relay room ID (from QR session_id)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularSetRoom(
    mut env: JNIEnv,
    _class: JClass,
    room_id: JString,
) {
    let room: String = match env.get_string(&room_id) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    info!("JNI: Cellular room set to '{}'", room);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref mut sm) = guard.state_machine {
        sm.set_cellular_room(room);
    }
}

/// JNI: Get the WebSocket URL for Kotlin to connect to
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularGetWsUrl<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let url = if let Some(ref sm) = guard.state_machine {
        sm.get_cellular_ws_url()
    } else {
        String::new()
    };

    env.new_string(&url).unwrap_or_else(|_| env.new_string("").unwrap())
}

/// JNI: Called when Kotlin WebSocket connects successfully
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularOnConnected(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: Cellular WebSocket connected");
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref mut sm) = guard.state_machine {
        if let Err(e) = sm.on_cellular_connected() {
            error!("Cellular connect failed: {}", e);
        }
    }
}

/// JNI: Called when Kotlin WebSocket disconnects
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularOnDisconnected(
    mut env: JNIEnv,
    _class: JClass,
    reason: JString,
) {
    let reason_str: String = env.get_string(&reason).map(|s| s.into()).unwrap_or_default();
    info!("JNI: Cellular disconnected: {}", reason_str);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref mut sm) = guard.state_machine {
        sm.on_cellular_disconnected(&reason_str);
    }
}

/// JNI: Called when Kotlin receives a binary message from the relay
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularOnMessage(
    env: JNIEnv,
    _class: JClass,
    data: jni::objects::JByteArray,
) {
    let bytes = match env.convert_byte_array(&data) {
        Ok(b) => b,
        Err(_) => return,
    };

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref mut sm) = guard.state_machine {
        sm.on_cellular_message(bytes);
    }
}

/// JNI: Called when Kotlin WebSocket has an error
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularOnError(
    mut env: JNIEnv,
    _class: JClass,
    error: JString,
) {
    let err_str: String = env.get_string(&error).map(|s| s.into()).unwrap_or_default();
    error!("JNI: Cellular error: {}", err_str);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref mut sm) = guard.state_machine {
        sm.on_cellular_error(&err_str);
    }
}

/// JNI: Poll outbound queue — returns byte array or null if empty
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularPollOutbound<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jni::objects::JByteArray<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        if let Some(data) = sm.poll_cellular_outbound() {
            return env.byte_array_from_slice(&data)
                .unwrap_or_else(|_| jni::objects::JByteArray::default());
        }
    }

    // Return null (empty default)
    jni::objects::JByteArray::default()
}

/// JNI: Get cellular stats JSON
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCellularGetStats<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let stats = if let Some(ref sm) = guard.state_machine {
        sm.get_cellular_stats()
    } else {
        "{}".to_string()
    };

    env.new_string(&stats).unwrap_or_else(|_| env.new_string("{}").unwrap())
}

/// JNI: Check if WiFi transport has discovered peers
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeHasWifiPeers(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        if sm.has_wifi_peers() { JNI_TRUE } else { JNI_FALSE }
    } else {
        JNI_FALSE
    }
}

//==============================================================================
// WIFI DIRECT JNI EXPORTS
//==============================================================================

/// JNI: WiFi Direct state changed (called by Kotlin BroadcastReceiver)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeOnWifiDirectStateChanged(
    _env: JNIEnv,
    _class: JClass,
    enabled: jboolean,
) {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let transport = sm.get_transport();
        let mut tm = transport.lock().unwrap_or_else(|e| e.into_inner());
        tm.wifi_direct_mut().on_state_changed(enabled == JNI_TRUE);
    }
}

/// JNI: WiFi Direct peers changed (called by Kotlin after requestPeers)
/// peers_json: JSON array of objects with device_name, device_address, is_group_owner
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeOnWifiDirectPeersChanged<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    peers_json: JString<'local>,
) {
    let json: String = match env.get_string(&peers_json) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    let peers: Vec<crate::wifi_direct::WifiDirectPeer> = match serde_json::from_str::<Vec<serde_json::Value>>(&json) {
        Ok(arr) => arr.iter().filter_map(|v| {
            Some(crate::wifi_direct::WifiDirectPeer {
                device_name: v["device_name"].as_str()?.to_string(),
                device_address: v["device_address"].as_str()?.to_string(),
                is_group_owner: v["is_group_owner"].as_bool().unwrap_or(false),
            })
        }).collect(),
        Err(_) => return,
    };

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let transport = sm.get_transport();
        let mut tm = transport.lock().unwrap_or_else(|e| e.into_inner());
        tm.wifi_direct_mut().on_peers_changed(peers);
    }
}

/// JNI: WiFi Direct connection changed (called by Kotlin BroadcastReceiver)
/// This is THE critical callback: when connected=true, we start multicast on the P2P network.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeOnWifiDirectConnectionChanged<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    connected: jboolean,
    is_owner: jboolean,
    group_owner_ip: JString<'local>,
    interface_name: JString<'local>,
) {
    let go_ip: Option<std::net::Ipv4Addr> = env.get_string(&group_owner_ip)
        .ok()
        .and_then(|s| {
            let s: String = s.into();
            s.parse().ok()
        });

    let iface: Option<String> = env.get_string(&interface_name)
        .ok()
        .map(|s| s.into());

    let is_connected = connected == JNI_TRUE;
    let is_group_owner = is_owner == JNI_TRUE;

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        // Update WiFi Direct state in transport manager
        {
            let transport = sm.get_transport();
            let mut tm = transport.lock().unwrap_or_else(|e| e.into_inner());
            tm.wifi_direct_mut().on_connection_changed(is_connected, is_group_owner, go_ip, iface);
        }

        // If connected, start multicast transport on the P2P network
        if is_connected {
            if let Err(e) = sm.on_wifi_direct_connected() {
                error!("JNI: Failed to start WiFi Direct transport: {}", e);
            }
        } else {
            sm.on_wifi_direct_disconnected();
        }
    }
}

/// JNI: WiFi Direct discovery started (called by Kotlin after discoverPeers)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeOnWifiDirectDiscoveryStarted(
    _env: JNIEnv,
    _class: JClass,
) {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let transport = sm.get_transport();
        let mut tm = transport.lock().unwrap_or_else(|e| e.into_inner());
        tm.wifi_direct_mut().on_discovery_started();
    }
}

/// JNI: Get WiFi Direct state (0=Disabled, 1=Available, 2=Discovering, 3=Connecting, 4=Connected, 5=Error)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetWifiDirectState(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.get_wifi_direct_state() {
            crate::wifi_direct::WifiDirectState::Disabled => 0,
            crate::wifi_direct::WifiDirectState::Available => 1,
            crate::wifi_direct::WifiDirectState::Discovering => 2,
            crate::wifi_direct::WifiDirectState::Connecting => 3,
            crate::wifi_direct::WifiDirectState::Connected => 4,
            crate::wifi_direct::WifiDirectState::Error => 5,
        }
    } else {
        0
    }
}

/// JNI: Get WiFi Direct peers as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetWifiDirectPeers<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = if let Some(ref sm) = guard.state_machine {
        let peers = sm.get_wifi_direct_peers();
        let arr: Vec<serde_json::Value> = peers.iter().map(|p| {
            serde_json::json!({
                "device_name": p.device_name,
                "device_address": p.device_address,
                "is_group_owner": p.is_group_owner,
            })
        }).collect();
        serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
    } else {
        "[]".to_string()
    };

    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Get WiFi Direct group role (0=None, 1=Owner, 2=Client)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetWifiDirectRole(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.get_wifi_direct_role() {
            crate::wifi_direct::GroupRole::None => 0,
            crate::wifi_direct::GroupRole::Owner => 1,
            crate::wifi_direct::GroupRole::Client => 2,
        }
    } else {
        0
    }
}

/// JNI: Connect via WiFi multicast directly (cross-platform mode, shared WiFi network)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeConnectWifiMulticast(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    info!("JNI: Connect via WiFi multicast (cross-platform)");

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.connect_wifi_multicast() {
            Ok(()) => JNI_TRUE,
            Err(e) => {
                error!("JNI: WiFi multicast connect failed: {}", e);
                JNI_FALSE
            }
        }
    } else {
        JNI_FALSE
    }
}

/// JNI: Check if WiFi Direct has discovered peers
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeHasWifiDirectPeers(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        if sm.has_wifi_direct_peers() { JNI_TRUE } else { JNI_FALSE }
    } else {
        JNI_FALSE
    }
}
