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

/// Cached TranscriptionBridge class GlobalRef (resolved during nativeInit)
static TRANSCRIPTION_BRIDGE_CLASS: std::sync::OnceLock<GlobalRef> = std::sync::OnceLock::new();

/// Store the TranscriptionBridge class reference
pub fn init_transcription_bridge_class(global_ref: GlobalRef) {
    let _ = TRANSCRIPTION_BRIDGE_CLASS.set(global_ref);
    info!("JNI: TranscriptionBridge class cached");
}

/// Get the cached TranscriptionBridge class reference
pub fn get_transcription_bridge_class() -> Option<&'static GlobalRef> {
    TRANSCRIPTION_BRIDGE_CLASS.get()
}

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
    /// Get the integer value of a `MediaRecorder.AudioSource` constant by name.
    /// Returns None if the field doesn't exist on this SDK level.
    pub fn audio_source_id(name: &str) -> Option<i32> {
        let vm = get_jvm().ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let source_class = env.find_class("android/media/MediaRecorder$AudioSource").ok()?;
        let val = env.get_static_field(&source_class, name, "I").ok()?.i().ok()?;
        Some(val)
    }

    /// Create AudioRecord instance with an explicit audio source (e.g.
    /// `MediaRecorder.AudioSource.VOICE_RECOGNITION`). Caller picks the
    /// source from `DeviceQuirks::current().source_fallback_chain` and
    /// retries with the next on `STATE_UNINITIALIZED`.
    pub fn new_with_source(audio_source: i32, sample_rate: i32, channel_config: i32, audio_format: i32, buffer_size: i32) -> Result<Self, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;

        let recorder_class = env.find_class("android/media/AudioRecord")
            .map_err(|e| format!("Failed to find AudioRecord class: {}", e))?;

        let recorder = env.new_object(
            &recorder_class,
            "(IIIII)V",
            &[
                JValue::Int(audio_source),
                JValue::Int(sample_rate),
                JValue::Int(channel_config),
                JValue::Int(audio_format),
                JValue::Int(buffer_size),
            ]
        )
        .map_err(|e| format!("Failed to create AudioRecord: {}", e))?;

        // Verify the recorder actually initialized (some OEM HALs reject
        // certain sources silently — STATE_UNINITIALIZED = 0).
        let state_initialized = env.get_static_field(&recorder_class, "STATE_INITIALIZED", "I")
            .map_err(|e| format!("Failed to get STATE_INITIALIZED: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert STATE_INITIALIZED: {}", e))?;
        let state = env.call_method(&recorder, "getState", "()I", &[])
            .map_err(|e| format!("Failed to call getState: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert getState: {}", e))?;
        if state != state_initialized {
            let _ = env.call_method(&recorder, "release", "()V", &[]);
            return Err(format!("AudioRecord state={} (not STATE_INITIALIZED) for source={}", state, audio_source));
        }

        let global_ref = env.new_global_ref(&recorder)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;

        Ok(Self { recorder: global_ref })
    }

    /// Get the AudioRecord's session ID — needed to attach AEC/NS/AGC
    /// effects to this specific recording chain.
    pub fn audio_session_id(&self) -> Result<i32, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        env.call_method(self.recorder.as_obj(), "getAudioSessionId", "()I", &[])
            .map_err(|e| format!("Failed getAudioSessionId: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert sessionId: {}", e))
    }

    /// Legacy constructor — defaults to MIC. Prefer `new_with_source` so
    /// per-device fallback chains pick the right source.
    pub fn new(sample_rate: i32, channel_config: i32, audio_format: i32, buffer_size: i32) -> Result<Self, String> {
        let mic = Self::audio_source_id("MIC").unwrap_or(1);
        Self::new_with_source(mic, sample_rate, channel_config, audio_format, buffer_size)
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

        // Push a local-ref frame so the short[] we allocate each call is freed
        // on return. The audio thread stays attached for the life of the app,
        // so without this the local-ref table overflows after thousands of frames.
        env.push_local_frame(4)
            .map_err(|e| format!("Failed to push local frame: {}", e))?;

        let result: Result<usize, String> = (|| {
            let jarray = env.new_short_array(buffer.len() as i32)
                .map_err(|e| format!("Failed to create short array: {}", e))?;
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
        })();

        unsafe { let _ = env.pop_local_frame(&JObject::null()); }
        result
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
    /// Minimum AudioTrack buffer size for the given format. AudioTrack has
    /// different buffer-size requirements than AudioRecord, so we must query
    /// the correct class — otherwise playback under-allocates and glitches.
    pub fn get_min_buffer_size(sample_rate: i32, channel_config: i32, audio_format: i32) -> Result<i32, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        let track_class = env.find_class("android/media/AudioTrack")
            .map_err(|e| format!("Failed to find AudioTrack class: {}", e))?;
        let size = env.call_static_method(
            track_class,
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

    /// Create AudioTrack instance.
    ///
    /// Uses the modern `AudioAttributes` + `AudioFormat` builder constructor
    /// with `USAGE_VOICE_COMMUNICATION` + `CONTENT_TYPE_SPEECH`. This
    /// intentionally bypasses the OEM media post-processing chain (Dolby
    /// Atmos on Motorola, Adapt Sound on Samsung, MIUI Sound Enhance on
    /// Xiaomi) which spectrally mangles 20 ms VoIP voice frames into the
    /// "robotic / slowed-down" artifact users hear. The legacy
    /// `STREAM_MUSIC, MODE_STREAM` constructor we used before routed audio
    /// through that chain by default and was the single biggest contributor
    /// to bad voice quality.
    pub fn new(sample_rate: i32, channel_config: i32, audio_format: i32, buffer_size: i32) -> Result<Self, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;

        let track_class = env.find_class("android/media/AudioTrack")
            .map_err(|e| format!("Failed to find AudioTrack class: {}", e))?;
        let attrs_class = env.find_class("android/media/AudioAttributes")
            .map_err(|e| format!("Failed to find AudioAttributes class: {}", e))?;
        let attrs_builder_class = env.find_class("android/media/AudioAttributes$Builder")
            .map_err(|e| format!("Failed to find AudioAttributes.Builder class: {}", e))?;
        let _format_class = env.find_class("android/media/AudioFormat")
            .map_err(|e| format!("Failed to find AudioFormat class: {}", e))?;
        let format_builder_class = env.find_class("android/media/AudioFormat$Builder")
            .map_err(|e| format!("Failed to find AudioFormat.Builder class: {}", e))?;

        let usage_voice_comm = env.get_static_field(&attrs_class, "USAGE_VOICE_COMMUNICATION", "I")
            .map_err(|e| format!("Failed to get USAGE_VOICE_COMMUNICATION: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        let content_speech = env.get_static_field(&attrs_class, "CONTENT_TYPE_SPEECH", "I")
            .map_err(|e| format!("Failed to get CONTENT_TYPE_SPEECH: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;

        let mode_stream = env.get_static_field(&track_class, "MODE_STREAM", "I")
            .map_err(|e| format!("Failed to get MODE_STREAM field: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        // `AUDIO_SESSION_ID_GENERATE` lives on `AudioManager`, not
        // `AudioTrack` — looking it up on AudioTrack throws
        // NoSuchFieldError and crashes audio init. Value is 0, documented
        // public API since L (API 21).
        let session_id_none: i32 = 0;

        // Build AudioAttributes(usage=VOICE_COMMUNICATION, contentType=SPEECH)
        let attrs_builder = env.new_object(&attrs_builder_class, "()V", &[])
            .map_err(|e| format!("Failed to create AudioAttributes.Builder: {}", e))?;
        let attrs_builder = env.call_method(
            &attrs_builder,
            "setUsage",
            "(I)Landroid/media/AudioAttributes$Builder;",
            &[JValue::Int(usage_voice_comm)],
        )
            .map_err(|e| format!("Failed setUsage: {}", e))?
            .l()
            .map_err(|e| format!("Failed setUsage return: {}", e))?;
        let attrs_builder = env.call_method(
            &attrs_builder,
            "setContentType",
            "(I)Landroid/media/AudioAttributes$Builder;",
            &[JValue::Int(content_speech)],
        )
            .map_err(|e| format!("Failed setContentType: {}", e))?
            .l()
            .map_err(|e| format!("Failed setContentType return: {}", e))?;
        let audio_attrs = env.call_method(
            &attrs_builder,
            "build",
            "()Landroid/media/AudioAttributes;",
            &[],
        )
            .map_err(|e| format!("Failed AudioAttributes.build: {}", e))?
            .l()
            .map_err(|e| format!("Failed AudioAttributes.build return: {}", e))?;

        // Build AudioFormat(encoding=PCM_16, sampleRate, channelMask=channel_config)
        let format_builder = env.new_object(&format_builder_class, "()V", &[])
            .map_err(|e| format!("Failed to create AudioFormat.Builder: {}", e))?;
        let format_builder = env.call_method(
            &format_builder,
            "setEncoding",
            "(I)Landroid/media/AudioFormat$Builder;",
            &[JValue::Int(audio_format)],
        )
            .map_err(|e| format!("Failed setEncoding: {}", e))?
            .l()
            .map_err(|e| format!("Failed setEncoding return: {}", e))?;
        let format_builder = env.call_method(
            &format_builder,
            "setSampleRate",
            "(I)Landroid/media/AudioFormat$Builder;",
            &[JValue::Int(sample_rate)],
        )
            .map_err(|e| format!("Failed setSampleRate: {}", e))?
            .l()
            .map_err(|e| format!("Failed setSampleRate return: {}", e))?;
        let format_builder = env.call_method(
            &format_builder,
            "setChannelMask",
            "(I)Landroid/media/AudioFormat$Builder;",
            &[JValue::Int(channel_config)],
        )
            .map_err(|e| format!("Failed setChannelMask: {}", e))?
            .l()
            .map_err(|e| format!("Failed setChannelMask return: {}", e))?;
        let audio_format_obj = env.call_method(
            &format_builder,
            "build",
            "()Landroid/media/AudioFormat;",
            &[],
        )
            .map_err(|e| format!("Failed AudioFormat.build: {}", e))?
            .l()
            .map_err(|e| format!("Failed AudioFormat.build return: {}", e))?;

        // AudioTrack(AudioAttributes, AudioFormat, bufferSizeInBytes, mode, sessionId)
        let track = env.new_object(
            &track_class,
            "(Landroid/media/AudioAttributes;Landroid/media/AudioFormat;III)V",
            &[
                JValue::Object(&audio_attrs),
                JValue::Object(&audio_format_obj),
                JValue::Int(buffer_size),
                JValue::Int(mode_stream),
                JValue::Int(session_id_none),
            ],
        )
        .map_err(|e| format!("Failed to create AudioTrack (voice): {}", e))?;

        let global_ref = env.new_global_ref(&track)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;

        info!("AudioTrack created with USAGE_VOICE_COMMUNICATION + CONTENT_TYPE_SPEECH (bypasses OEM media post-processing)");
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

        // Bound local refs per call so the playback thread's ref table
        // doesn't grow unbounded across every written audio frame.
        env.push_local_frame(4)
            .map_err(|e| format!("Failed to push local frame: {}", e))?;

        let result: Result<usize, String> = (|| {
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
        })();

        unsafe { let _ = env.pop_local_frame(&JObject::null()); }
        result
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
use crate::codec::{VoiceEncoder, VoiceDecoder, CODEC_FRAME_SIZE};
use crate::audio_pipeline;

/// Global state for JNI mode (when used from Kotlin instead of egui)
static JNI_STATE: OnceLock<Arc<Mutex<JniAppState>>> = OnceLock::new();

struct JniAppState {
    state_machine: Option<StateMachine>,
    session_manager: SessionManager,
    /// Pre-init user registry. Once a StateMachine exists, the canonical
    /// registry is the StateMachine's (the RX thread reads/writes that one);
    /// use [`JniAppState::active_user_registry`] rather than touching this
    /// field directly so mute/favorite/register never split across two
    /// instances. Kept as a shared handle so the accessor can return an owned
    /// `Arc` in both the pre-init and running cases.
    user_registry: Arc<Mutex<UserRegistry>>,
    cohort_history: crate::cohort_history::CohortHistory,
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    current_subchannel: Arc<AtomicU8>,
    pending_key_exchange: Option<crate::crypto::KeyExchange>,
    /// BT TX buffer: Kotlin reads encoded frames from here for RFCOMM transmission
    bt_tx_buffer: Arc<Mutex<Option<Vec<u8>>>>,
    /// BT codec instances (stateful ADPCM, persisted across JNI calls)
    bt_encoder: VoiceEncoder,
    bt_decoder: VoiceDecoder,
    /// Track whether BT mic capture is active for btEncodeFrame
    bt_recording: bool,
}

impl JniAppState {
    fn new() -> Self {
        let ptt_pressed = Arc::new(AtomicBool::new(false));
        let current_channel = Arc::new(AtomicU8::new(1));
        let current_subchannel = Arc::new(AtomicU8::new(0));

        Self {
            state_machine: None,
            session_manager: SessionManager::new("SassyTalkie"),
            user_registry: Arc::new(Mutex::new(UserRegistry::new())),
            cohort_history: crate::cohort_history::CohortHistory::new(
                crate::cohort_history::DEFAULT_HISTORY_CAP,
            ),
            ptt_pressed,
            current_channel,
            current_subchannel,
            pending_key_exchange: None,
            bt_tx_buffer: Arc::new(Mutex::new(None)),
            bt_encoder: VoiceEncoder::new(),
            bt_decoder: VoiceDecoder::new(),
            bt_recording: false,
        }
    }

    /// The canonical user registry to read/write for this call.
    ///
    /// When a StateMachine exists it owns the registry the RX thread consults
    /// to filter muted/favorited peers, so ALL JNI user operations
    /// (register/mute/favorite/list) must go through it. Before init we fall
    /// back to the local pre-init registry. Returning an owned `Arc` keeps
    /// callers from holding a borrow of `self` across the lock.
    fn active_user_registry(&self) -> Arc<Mutex<UserRegistry>> {
        match self.state_machine.as_ref() {
            Some(sm) => Arc::clone(sm.get_user_registry()),
            None => Arc::clone(&self.user_registry),
        }
    }

    fn initialize(&mut self) -> bool {
        info!("JNI: Initializing backend");

        let state_machine = StateMachine::new(
            Arc::clone(&self.ptt_pressed),
            Arc::clone(&self.current_channel),
            Arc::clone(&self.current_subchannel),
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
    mut env: JNIEnv,
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

    // Cache TranscriptionBridge class reference so RX thread can call it via JNI.
    // Must be done on the main thread (which has the app classloader) because
    // native threads only have the system classloader and can't find app classes.
    {
        let class_name = "com/sassyconsulting/sassytalkie/TranscriptionBridge";
        match env.find_class(class_name) {
            Ok(cls) => {
                match env.new_global_ref(cls) {
                    Ok(global) => {
                        init_transcription_bridge_class(global);
                    }
                    Err(e) => {
                        warn!("JNI: Failed to create GlobalRef for TranscriptionBridge: {:?}", e);
                    }
                }
            }
            Err(e) => {
                warn!("JNI: TranscriptionBridge class not found (transcription disabled): {:?}", e);
                let _ = env.exception_clear();
            }
        }
    }

    // Initialize app state
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if guard.initialize() {
        // Warm up the per-OEM quirks profile so the first PTT doesn't pay the
        // JNI Build.* read latency.
        let _ = crate::device_quirks::current();
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// JNI: Cache the application Context. Called from Kotlin in
/// WalkieService.onCreate. Required by `audio_routing::engage_comm_mode`
/// to obtain `AudioManager` via `getSystemService`. Safe to call multiple
/// times — only the first call sticks.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeInitContext(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
) -> jboolean {
    if context.is_null() {
        warn!("nativeInitContext: null Context, ignoring");
        return JNI_FALSE;
    }
    match env.new_global_ref(&context) {
        Ok(global) => {
            crate::audio_routing::init_context(global);
            JNI_TRUE
        }
        Err(e) => {
            warn!("nativeInitContext: new_global_ref failed: {}", e);
            JNI_FALSE
        }
    }
}

/// JNI: Start PTT transmission (with connection guard)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativePttStart(
    _env: JNIEnv,
    _class: JClass,
) {
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    // Connection guard: check if any transport has connected peers
    let (bt_connected, transport_active) = if let Some(ref sm) = guard.state_machine {
        let active = sm.get_active_transport();
        let bt = active == crate::transport::ActiveTransport::Bluetooth;
        let has_transport = active != crate::transport::ActiveTransport::None;
        (bt, has_transport)
    } else {
        (false, false)
    };

    let channel = guard.current_channel.load(Ordering::Relaxed);
    info!("PTT START pressed — BT connected: {}, transport active: {}, channel: {}",
          bt_connected, transport_active, channel);

    if !transport_active && !bt_connected {
        warn!("PTT blocked: no connected peers (transport=None, BT=false)");
        return;
    }

    guard.ptt_pressed.store(true, Ordering::SeqCst);

    if let Some(ref sm) = guard.state_machine {
        if let Err(e) = sm.on_ptt_press() {
            error!("JNI: Failed to start transmit: {}", e);
        }
    }

    // Reset BT encoder state for new transmission
    guard.bt_encoder.reset();
    guard.bt_recording = false;
    info!("Native PTT started");
}

/// JNI: Stop PTT transmission
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativePttStop(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: PTT Stop");

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    guard.ptt_pressed.store(false, Ordering::SeqCst);

    // Stop BT mic capture if active
    if guard.bt_recording {
        if let Some(ref sm) = guard.state_machine {
            let audio = sm.get_audio();
            if let Ok(eng) = audio.lock() {
                let _ = eng.stop_recording();
            }
        }
        guard.bt_recording = false;
    }

    if let Some(ref sm) = guard.state_machine {
        if let Err(e) = sm.on_ptt_release() {
            error!("JNI: Failed to stop transmit: {}", e);
        }
    }

    // Clear BT TX buffer
    if let Ok(mut buf) = guard.bt_tx_buffer.lock() {
        *buf = None;
    };
}

/// JNI: Set PTT buffer mode (true = buffer and burst on release, false = live stream)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetPttBufferMode(
    _env: JNIEnv,
    _class: JClass,
    buffer_mode: jni::sys::jboolean,
) {
    crate::audio_pipeline::set_ptt_buffer_mode(buffer_mode != 0);
}

/// JNI: Get PTT buffer mode
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetPttBufferMode(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jboolean {
    if crate::audio_pipeline::get_ptt_buffer_mode() { 1 } else { 0 }
}

/// JNI: Enable/disable the per-frame TranscriptionBridge callback.
///
/// Off by default — the callback allocates a Java short[960] + attaches a
/// JNI thread per 20 ms audio frame, which produces enough GC pressure
/// to cause ~50-300 ms AudioTrack underruns on the RX path. Kotlin's
/// TranscriptionBridge.setEnabled() should call this so the JVM hop is
/// only paid when the feature is actually on.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetTranscriptionBridgeEnabled(
    _env: JNIEnv,
    _class: JClass,
    enabled: jboolean,
) {
    crate::audio_pipeline::set_transcription_bridge_enabled(enabled != 0);
}

/// JNI: Set mic input gain (gain × 100 — 100 = 1.0×, 200 = 2.0×). Clamped [25, 400].
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetMicGainX100(
    _env: JNIEnv,
    _class: JClass,
    gain_x100: jni::sys::jint,
) {
    crate::audio_pipeline::set_mic_gain_x100(gain_x100);
}

/// JNI: Get the current mic input gain × 100.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetMicGainX100(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jint {
    crate::audio_pipeline::get_mic_gain_x100()
}

// ── Noise suppression (audio_effects) ──────────────────────────────────────

/// JNI: Toggle the mic-path noise suppressor. Default OFF (zero cost disabled).
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetNoiseSuppressionEnabled(
    _env: JNIEnv,
    _class: JClass,
    enabled: jboolean,
) {
    crate::audio_effects::set_noise_suppression_enabled(enabled != 0);
}

/// JNI: True if the noise suppressor is currently engaged.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetNoiseSuppressionEnabled(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if crate::audio_effects::get_noise_suppression_enabled() { JNI_TRUE } else { JNI_FALSE }
}

/// JNI: Set max attenuation (dB) of the noise suppressor. Clamped on the Rust side.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetNoiseSuppressionAttenDb(
    _env: JNIEnv,
    _class: JClass,
    db: jni::sys::jint,
) {
    crate::audio_effects::set_noise_suppression_atten_db(db);
}

// ── Sealed sender (metadata resistance) ────────────────────────────────────

/// JNI: Enable/disable sealed-sender connection blinding for the cellular relay.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetSealedSenderEnabled(
    _env: JNIEnv,
    _class: JClass,
    enabled: jboolean,
) {
    crate::cellular_transport::set_sealed_enabled(enabled != 0);
}

/// JNI: Push the sealed context — the 32-byte session key (base64) and the
/// stable per-install peer id — used to derive per-epoch blinded room/peer
/// handles. Returns false if the key isn't a valid 32-byte base64 blob.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetSealedContext<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    key_b64: JString<'local>,
    peer_id: JString<'local>,
) -> jboolean {
    let key_b64: String = match env.get_string(&key_b64) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };
    let peer_id: String = env.get_string(&peer_id).map(|s| s.into()).unwrap_or_default();

    let key_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &key_b64,
    ) {
        Ok(b) if b.len() == 32 => b,
        Ok(b) => {
            error!("JNI: sealed key wrong length: {} (expected 32)", b.len());
            return JNI_FALSE;
        }
        Err(e) => {
            error!("JNI: sealed key decode failed: {}", e);
            return JNI_FALSE;
        }
    };
    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&key_bytes);
    crate::cellular_transport::set_sealed_context(key_array, peer_id);
    JNI_TRUE
}

/// JNI: Clear the sealed context (e.g. on session clear).
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeClearSealedContext(
    _env: JNIEnv,
    _class: JClass,
) {
    crate::cellular_transport::clear_sealed_context();
}

/// JNI: Set RX (playback) gain × 100. Clamped to [25, 400] on the Rust side.
/// Default 100 (1.0×) is a no-op fast path in `AudioEngine::write_audio`.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetRxGainX100(
    _env: JNIEnv,
    _class: JClass,
    gain_x100: jni::sys::jint,
) {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref sm) = guard.state_machine {
        if let Ok(eng) = sm.get_audio().lock() {
            eng.set_rx_gain_x100(gain_x100);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetRxGainX100(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jint {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref sm) = guard.state_machine {
        if let Ok(eng) = sm.get_audio().lock() {
            return eng.get_rx_gain_x100();
        }
    }
    100
}

/// JNI: Hard-mute or unmute RX playback (true half-duplex cut). Unlike
/// `nativeSetRxGainX100` (which floors at 0.25×), this fully silences incoming
/// audio. Used to cut RX while the local user is transmitting so the remote
/// stream can't acoustically feed back into the hot mic.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetRxMuted(
    _env: JNIEnv,
    _class: JClass,
    muted: jni::sys::jboolean,
) {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref sm) = guard.state_machine {
        if let Ok(eng) = sm.get_audio().lock() {
            eng.set_rx_muted(muted != 0);
        }
    }
}

/// JNI: Force speakerphone on or off for the current PTT session.
/// Backed by `audio_routing::engage_comm_mode(force_speaker)`. Returns true
/// on success, false on JNI failure (rare — usually missing AudioManager).
///
/// `true`  → loudspeaker (built-in main speaker, walkie-talkie default)
/// `false` → earpiece (small speaker, private listening)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetSpeakerphone(
    _env: JNIEnv,
    _class: JClass,
    on: jni::sys::jboolean,
) -> jni::sys::jboolean {
    let force_speaker = on != 0;
    match crate::audio_routing::engage_comm_mode(force_speaker) {
        Ok(()) => 1,
        Err(e) => {
            warn!("nativeSetSpeakerphone({}) failed: {}", force_speaker, e);
            0
        }
    }
}

/// JNI: True if our COMM mode is currently engaged. Doesn't directly tell
/// you speakerphone vs earpiece (that flag is internal to audio_routing),
/// but the UI uses its own persisted preference for the toggle state — this
/// is only for diagnostics / "is routing active?" checks.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsCommModeActive(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jboolean {
    if crate::audio_routing::is_active() { 1 } else { 0 }
}

/// JNI: Tune the Live-mode jitter buffer pre-buffer size. 3 = low-latency
/// (~60ms), 5 = balanced (default, ~100ms), 8 = smooth (~160ms). Clamped
/// to [2, 16] on the Rust side. Takes effect immediately for new frames.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetJitterPrebufferFrames(
    _env: JNIEnv,
    _class: JClass,
    frames: jni::sys::jint,
) {
    let f = frames.max(0) as usize;
    sassytalkie_core::audio_cache::set_live_jitter_prebuffer_frames(f);
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetJitterPrebufferFrames(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jint {
    sassytalkie_core::audio_cache::live_jitter_prebuffer_frames() as jni::sys::jint
}

/// JNI: Set squelch threshold in dBFS. 0 disables. Otherwise clamped to [-60, -10].
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetSquelchDbfs(
    _env: JNIEnv,
    _class: JClass,
    threshold: jni::sys::jint,
) {
    crate::audio_pipeline::set_squelch_dbfs(threshold);
}

/// JNI: Get the current squelch threshold (0 = disabled).
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetSquelchDbfs(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jint {
    crate::audio_pipeline::get_squelch_dbfs()
}

/// JNI: Replay utterance by unique ID.
///
/// Spawns a one-shot playback thread that drives AudioTrack directly. This
/// is independent of the RX thread (which only runs while a transport is
/// connected) — the old implementation set `audio_cache.now_playing` and
/// relied on the RX thread to drain it, so a user replaying after a
/// session ended would hear nothing.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeReplayById(
    _env: JNIEnv,
    _class: JClass,
    utterance_id: jni::sys::jlong,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let sm = match guard.state_machine.as_ref() {
        Some(sm) => sm,
        None => return JNI_FALSE,
    };

    // Snapshot the frames out of the cache; we don't want to hold the cache
    // lock across the playback thread's lifetime.
    let frames = {
        let cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        match cache.get_history_frames(utterance_id as u64) {
            Some(f) if !f.is_empty() => f,
            _ => {
                warn!("JNI: replay id={} not in history", utterance_id);
                return JNI_FALSE;
            }
        }
    };

    // Drop the JNI guard before spawning the playback thread so subsequent
    // JNI calls (clear, etc.) aren't blocked behind it. Clone the Arc so
    // the thread holds its own handle to the audio engine.
    let audio = Arc::clone(sm.get_audio());
    // Snapshot all three gate handles. All are cheap Arc clones; the
    // replay loop polls them without holding the engine mutex.
    //   - gate_clock: wall-clock ms of last write — used at startup to
    //     wait for "live RX has been quiet long enough"
    //   - write_seq: monotonic per-call counter — used inside the frame
    //     loop to detect RX intrusion (granular regardless of wall-clock
    //     resolution; two writes in the same ms still distinguishable)
    //   - playback_lock: self-exclusion so two replays can't overlap
    let (gate_clock, write_seq, playback_lock) = match audio.lock() {
        Ok(eng) => (eng.write_clock(), eng.write_seq_handle(), eng.playback_lock_handle()),
        Err(e) => {
            warn!("Replay: failed to snapshot gate handles: {}", e);
            return JNI_FALSE;
        }
    };

    // Pre-check the self-exclusion lock BEFORE spawning. Previously we
    // returned JNI_TRUE even when the spawned thread immediately bailed
    // because another replay held the lock — UI saw "Replaying X" but
    // nothing happened. Now we refuse synchronously and the snackbar
    // accurately reports the failure.
    if playback_lock.try_lock().is_err() {
        warn!("Replay {}: another replay is already running, declining", utterance_id);
        return JNI_FALSE;
    }
    drop(guard);

    info!("JNI: Replaying utterance id={} ({} frames)", utterance_id, frames.len());

    let spawn_result = std::thread::Builder::new()
        .name(format!("sassy-replay-{}", utterance_id))
        .spawn(move || {
            // ── Single-owner playback gate ───────────────────────────
            // Replay MUST defer to live RX traffic. Polls `gate_clock`:
            //   - If RX wrote in the last GATE_MIN_IDLE_MS → wait + retry.
            //   - If we've been waiting more than GATE_MAX_WAIT_MS total
            //     → give up. (User can re-tap once the channel quiets.)
            // RX never needs to know about replay — RX writes immediately
            // and bumps the clock; the replay thread sees that and yields.
            // This is the walkie-talkie convention: incoming traffic is
            // never preempted by a manual scrub action.
            const GATE_MIN_IDLE_MS: u64 = 200;
            const GATE_MAX_WAIT_MS: u64 = 3_000;

            // Re-acquire the self-exclusion lock inside the spawned thread.
            // The pre-spawn check already proved it was free; if something
            // else grabbed it in the microsecond between, we bail rather
            // than block (preserves the synchronous JNI return value).
            let _replay_guard = match playback_lock.try_lock() {
                Ok(g) => g,
                Err(_) => {
                    warn!("Replay {}: lost race for replay lock after spawn", utterance_id);
                    return;
                }
            };

            // Wait for live RX to quiet down before starting. Polls the
            // wall-clock idle; if RX is hot, we wait up to 3 s and bail
            // if it never quiets.
            if !crate::audio::wait_for_playback_idle(&gate_clock, GATE_MIN_IDLE_MS, GATE_MAX_WAIT_MS) {
                warn!(
                    "Replay {}: gave up after {}ms — live audio still active",
                    utterance_id, GATE_MAX_WAIT_MS
                );
                return;
            }

            // Start playback if it isn't already running; harmless if it is.
            if let Ok(eng) = audio.lock() {
                let _ = eng.start_playing();
            }

            // Frame loop with RX-intrusion detection via MONOTONIC SEQ.
            //
            // The atomic `write_seq` is incremented by EVERY successful
            // write — ours and RX's. After each of OUR writes we record
            // the resulting seq value (`last_self_seq`). Before the next
            // write, we re-read: if it advanced by more than the 1 our
            // own write contributed, someone else wrote between calls.
            // Granular per-call regardless of wall-clock resolution —
            // two writes within the same ms are still distinguishable
            // (the wall-clock approach used previously collapsed them).
            //
            // Result: live RX always wins (lock-free, unimpeded), and
            // replay never interleaves a single frame with live audio.
            // Either replay completes uninterrupted, or it bails the
            // instant RX takes over.
            let mut last_self_seq: u64 = 0;
            for samples in frames {
                if last_self_seq != 0 {
                    let now_seq = write_seq.load(std::sync::atomic::Ordering::Relaxed);
                    if now_seq != last_self_seq {
                        info!(
                            "Replay {}: yielding — RX wrote (seq {} -> {})",
                            utterance_id, last_self_seq, now_seq
                        );
                        break;
                    }
                }
                let eng = match audio.lock() {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("Replay: audio lock poisoned: {}", e);
                        break;
                    }
                };
                if let Err(e) = eng.write_audio(&samples) {
                    warn!("Replay: write_audio failed: {}", e);
                    break;
                }
                drop(eng);
                // After our write, write_seq advanced by exactly 1. Capture
                // that value — any further advance before our next iteration
                // means an RX writer slipped in.
                last_self_seq = write_seq.load(std::sync::atomic::Ordering::Relaxed);
            }
            info!("Replay {} complete", utterance_id);
            // _replay_guard drops here → another replay can run.
        })
        ;

    // Thread spawn failure (rare — OOM, ulimit) shouldn't panic inside
    // a JNI call. Translate to a JNI_FALSE return so the UI snackbar
    // reflects reality instead of saying "Replaying X" silently.
    match spawn_result {
        Ok(_) => JNI_TRUE,
        Err(e) => {
            warn!("Replay {}: failed to spawn thread: {}", utterance_id, e);
            JNI_FALSE
        }
    }
}

/// JNI: Get the ID of the most recently added history utterance
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeLastHistoryId(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jlong {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.last_history_id().map(|id| id as i64).unwrap_or(-1)
    } else {
        -1
    }
}

/// JNI: Set channel (syncs to both Rust pipeline and BT transport)
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

    guard.current_channel.store(ch, Ordering::SeqCst);
}

/// JNI: Set subchannel (0=Main, 1=A, 2=B)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetSubchannel(
    _env: JNIEnv,
    _class: JClass,
    subchannel: jbyte,
) {
    let sub = (subchannel as u8).min(2);
    info!("JNI: Set subchannel to {}", sub);

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.current_subchannel.store(sub, Ordering::SeqCst);
}

/// JNI: Get current channel (for Kotlin BT transport to include in frames)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetChannel(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.current_channel.load(Ordering::SeqCst) as jbyte
}

/// JNI: Encode one audio frame for BT transmission.
///
/// Reads mic → Opus encode → pack wire frame → return byte[] for Kotlin to send via RFCOMM.
/// Returns null if no audio data available.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeBtEncodeFrame<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jni::sys::jbyteArray {
    use jni::objects::JByteArray;

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if guard.state_machine.is_none() {
        return std::ptr::null_mut();
    }

    // Start mic recording if not already
    if !guard.bt_recording {
        if let Some(ref sm) = guard.state_machine {
            let audio = sm.get_audio();
            if let Ok(eng) = audio.lock() {
                match eng.start_recording() {
                    Ok(()) => {}
                    Err(e) => {
                        error!("BT TX: failed to start recording: {}", e);
                        return std::ptr::null_mut();
                    }
                }
            }
        }
        guard.bt_recording = true;
        info!("BT TX: started mic recording");
    }

    let sm = guard.state_machine.as_ref().unwrap();

    // Read one frame from mic
    let mut pcm_buffer = vec![0i16; CODEC_FRAME_SIZE];
    let samples_read = {
        let audio = sm.get_audio();
        if let Ok(eng) = audio.lock() {
            match eng.read_audio(&mut pcm_buffer) {
                Ok(n) => n,
                Err(_) => 0,
            }
        } else {
            0
        }
    };

    if samples_read < CODEC_FRAME_SIZE {
        return std::ptr::null_mut(); // Incomplete frame, caller should retry
    }

    // Apply mic gain + squelch + activity-log feed symmetrically with the
    // unified pipeline so BT users get the same Settings controls. If the
    // squelch threshold drops this frame, we return null and Kotlin will
    // simply not transmit it over RFCOMM.
    audio_pipeline::apply_mic_gain_public(&mut pcm_buffer[..CODEC_FRAME_SIZE]);
    if audio_pipeline::squelch_drops_frame(&pcm_buffer[..CODEC_FRAME_SIZE]) {
        return std::ptr::null_mut();
    }

    // Encode with Opus
    let compressed = guard.bt_encoder.encode(&pcm_buffer[..CODEC_FRAME_SIZE]);

    // Mirror the unified-pipeline activity-log feed so BT speakers also
    // appear in the timeline.
    let bridge_sender_id;
    let bridge_device_name;
    {
        let sm_for_bridge = guard.state_machine.as_ref();
        bridge_sender_id = sm_for_bridge.map(|s| s.get_local_sender_id()).unwrap_or_default();
        bridge_device_name = sm_for_bridge.map(|s| s.get_device_name()).unwrap_or_default();
    }
    audio_pipeline::call_transcription_bridge_public(
        &bridge_sender_id,
        &bridge_device_name,
        &pcm_buffer[..CODEC_FRAME_SIZE],
        false,
        false,
        true, // is_self — local BT transmit (timeline only, no "is speaking" UI)
    );

    // Pack wire frame
    let channel = guard.current_channel.load(Ordering::SeqCst);
    let subchannel = guard.current_subchannel.load(Ordering::SeqCst);
    let (sender_id, device_name) = if let Some(ref sm) = guard.state_machine {
        (sm.get_local_sender_id(), sm.get_device_name())
    } else {
        ("unknown".to_string(), "unknown".to_string())
    };
    let timestamp = audio_pipeline::now_ms();
    let wire_data = audio_pipeline::pack_wire_frame(channel, subchannel, &sender_id, &device_name, timestamp, &compressed);

    // Encrypt through the same AES-256-GCM path as WiFi/cellular.
    //
    // SECURITY + ROBUSTNESS: if there is no active crypto session, REFUSE to
    // transmit. The old behaviour was to fall back to sending plaintext —
    // which when paired with the matching "fallback to plaintext on decrypt
    // failure" on the receive side meant any peer with a mismatched key
    // (or no key) would receive AES-GCM ciphertext, feed it to the Opus
    // decoder as if it were a wire frame, and emit garbled noise. That is
    // the actual cause of the BT "garbled audio" symptom.
    let encrypted = match guard.state_machine.as_ref() {
        Some(sm) => {
            let transport = sm.get_transport();
            let mut tm = match transport.lock() {
                Ok(t) => t,
                Err(e) => {
                    error!("BT TX: transport lock poisoned: {}", e);
                    return std::ptr::null_mut();
                }
            };
            match tm.encrypt_raw(&wire_data) {
                Ok(enc) => enc,
                Err(e) => {
                    // No crypto session — drop the frame rather than leak
                    // plaintext over BT. UI should surface "authenticate
                    // via QR" if no session is established.
                    warn!("BT TX: encrypt failed ({}), dropping frame", e);
                    return std::ptr::null_mut();
                }
            }
        }
        None => return std::ptr::null_mut(),
    };

    // Return as byte array
    match env.byte_array_from_slice(&encrypted) {
        Ok(arr) => arr.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// JNI: Decode a BT-received audio frame and play it.
///
/// Kotlin passes raw bytes received from RFCOMM → unpack wire frame → ADPCM decode → play.
/// Returns true if frame was accepted, false if rejected (wrong channel, malformed, etc.)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeBtDecodeFrame<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    data: jni::sys::jbyteArray,
) -> jboolean {
    use jni::objects::JByteArray;

    // Convert Java byte[] to Rust Vec
    let j_data = unsafe { JByteArray::from_raw(data) };
    let raw_bytes = match env.convert_byte_array(&j_data) {
        Ok(b) => b,
        Err(_) => return JNI_FALSE,
    };

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    // Decrypt through the same AES-256-GCM path as WiFi/cellular.
    //
    // SECURITY + ROBUSTNESS: if decryption fails (no session, wrong key,
    // tampered packet, mismatched paring), DROP the frame. The old fallback
    // treated the ciphertext as plaintext, then handed those random-looking
    // bytes to unpack_wire_frame + the Opus decoder. The decoder happily
    // produced ~960 samples of garbled noise, which is the BT "garbled
    // mess" symptom users have been hearing. Match the WiFi/cellular path
    // which silently drops on decrypt failure (transport.rs:271-281).
    let decrypted = match guard.state_machine.as_ref() {
        Some(sm) => {
            let transport = sm.get_transport();
            let tm = match transport.lock() {
                Ok(t) => t,
                Err(e) => {
                    error!("BT RX: transport lock poisoned: {}", e);
                    return JNI_FALSE;
                }
            };
            match tm.decrypt_raw(&raw_bytes) {
                Ok(dec) => dec,
                Err(e) => {
                    warn!("BT RX: decrypt failed ({}), dropping packet", e);
                    return JNI_FALSE;
                }
            }
        }
        None => return JNI_FALSE,
    };

    // Unpack wire frame (now decrypted)
    let (channel, _subchannel, sender_id, device_name, timestamp, compressed) = match audio_pipeline::unpack_wire_frame(&decrypted) {
        Ok(parsed) => parsed,
        Err(e) => {
            warn!("btDecodeFrame: invalid wire frame: {}", e);
            return JNI_FALSE;
        }
    };

    // Filter by channel
    let my_channel = guard.current_channel.load(Ordering::SeqCst);
    if channel != my_channel {
        return JNI_FALSE;
    }

    // Validate compressed payload is non-empty (Opus uses variable-length frames)
    if compressed.is_empty() {
        warn!("btDecodeFrame: empty compressed payload");
        return JNI_FALSE;
    }

    // Auto-register sender and read mute/favorite from the CANONICAL registry
    // (the StateMachine's when running) so BT RX filtering and the activity-log
    // feed match the WiFi/cellular RX path instead of consulting a stale
    // pre-init copy.
    let (is_favorite, is_muted) = {
        let reg_arc = guard.active_user_registry();
        let mut reg = reg_arc.lock().unwrap_or_else(|e| e.into_inner());
        reg.register_user(&sender_id, &device_name);
        (reg.is_favorite(&sender_id), reg.is_muted(&sender_id))
    };

    // Decode ADPCM
    let pcm_samples = guard.bt_decoder.decode(&compressed);
    audio_pipeline::call_transcription_bridge_public(
        &sender_id,
        &device_name,
        &pcm_samples,
        is_favorite,
        is_muted,
        false, // remote — BT-received audio
    );

    // Feed into audio cache and play
    if let Some(ref sm) = guard.state_machine {
        // Feed audio cache
        let cache = sm.get_audio_cache();
        let mut cache_lock = cache.lock().unwrap_or_else(|e| e.into_inner());
        let passthrough = cache_lock.ingest_frame(&sender_id, timestamp, pcm_samples.clone());
        cache_lock.tick();

        let samples_to_play = if let Some(direct) = passthrough {
            Some(direct)
        } else {
            cache_lock.next_playback_frame().map(|(_, s)| s)
        };
        drop(cache_lock);

        if let Some(samples) = samples_to_play {
            let audio = sm.get_audio();
            if let Ok(eng) = audio.lock() {
                let _ = eng.start_playing();
                let _ = eng.write_audio(&samples);
            }
        }
    }

    JNI_TRUE
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
            crate::transport::ActiveTransport::Bluetooth => 5,
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
    // Legacy: generate for current channel with default name
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let channel = guard.current_channel.load(std::sync::atomic::Ordering::SeqCst);
    drop(guard);

    Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGenerateChannelQR(
        env, _class, channel as jni::sys::jint, duration_hours,
        std::ptr::null_mut(), // null group_name = use default
        std::ptr::null_mut(), // null cohort_id = mint fresh
    )
}

/// JNI: Generate session QR for a specific channel with optional group name + cohort_id
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGenerateChannelQR<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
    duration_hours: jni::sys::jint,
    group_name: jni::sys::jstring,
    cohort_id: jni::sys::jstring,
) -> JObject<'local> {
    let ch = channel as u8;
    let name: String = if !group_name.is_null() {
        let j_name = unsafe { JString::from_raw(group_name) };
        env.get_string(&j_name).map(|s| s.into()).unwrap_or_default()
    } else {
        String::new()
    };
    let cohort: Option<String> = if !cohort_id.is_null() {
        let j_cid = unsafe { JString::from_raw(cohort_id) };
        env.get_string(&j_cid).ok().map(|s| s.into())
    } else {
        None
    };

    info!("JNI: Generate session QR ch{} '{}' cohort={:?} ({}h)", ch, name, cohort, duration_hours);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let qr_json = match guard.session_manager.generate_session_qr_with_cohort(
        ch, duration_hours as u32, &name, cohort.as_deref(),
    ) {
        Ok(json) => json,
        Err(e) => {
            error!("JNI: Generate QR failed: {}", e);
            return env.new_string("").map(|s| s.into()).unwrap_or_else(|_| JObject::null());
        }
    };

    let (sid, cid) = match serde_json::from_str::<serde_json::Value>(&qr_json) {
        Ok(v) => (
            v["session_id"].as_str().unwrap_or("").to_string(),
            v["cohort_id"].as_str().unwrap_or("").to_string(),
        ),
        Err(_) => (String::new(), String::new()),
    };

    if !cid.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs()).unwrap_or(0);
        guard.cohort_history.upsert_host(ch, &name, Some(&cid), &sid, now);
    }

    if let Some(ref sm) = guard.state_machine {
        let mut tm = sm.get_transport().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(crypto) = guard.session_manager.get_crypto_for_channel(ch) {
            tm.set_crypto(crypto);
        }
    }

    // Sync the cellular relay room to the JUST-MINTED session_id so the host's
    // own WS targets the same room the joiner will scan into. Previously the
    // host only set crypto + persisted the session, but its WS stayed bound to
    // whatever room was set earlier (or none) — joiners would import with the
    // new session_id, connect to that room, and find an empty room because the
    // host was still on a different room. Symmetric counterpart to the call
    // already in nativeImportSessionFromQR.
    if !sid.is_empty() {
        if let Some(ref sm) = guard.state_machine {
            sm.set_cellular_room(sid.clone());
            info!("JNI: Cellular room synced to generated session_id {}", sid);
        }
    }

    drop(guard);

    env.new_string(&qr_json)
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
        Ok((channel, crypto, cohort_id)) => {
            if let Some(ref sm) = guard.state_machine {
                let mut tm = sm.get_transport().lock().unwrap_or_else(|e| e.into_inner());
                tm.set_crypto(crypto);
            }
            guard.current_channel.store(channel, std::sync::atomic::Ordering::SeqCst);

            let mut imported_sid = String::new();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json) {
                let host_dev = parsed["device"].as_str().unwrap_or("").to_string();
                let sid = parsed["session_id"].as_str().unwrap_or("").to_string();
                imported_sid = sid.clone();
                let gname = parsed["group_name"].as_str()
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("Channel {}", channel));
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs()).unwrap_or(0);
                guard.cohort_history.upsert_joiner(channel, &gname, Some(&cohort_id),
                                                   &host_dev, &sid, now);
            }

            // Sync the cellular relay room to the imported session_id so the
            // joiner targets the host's room on the next WS connect. Without
            // this, an already-connected WS stays bound to a stale room and
            // peers never see each other — Kotlin must still tear down and
            // re-establish the WS for the new room to take effect.
            if !imported_sid.is_empty() {
                if let Some(ref sm) = guard.state_machine {
                    sm.set_cellular_room(imported_sid.clone());
                    info!("JNI: Cellular room synced to imported session_id {}", imported_sid);
                }
            }

            info!("JNI: Session imported successfully for ch{} cohort {}", channel, cohort_id);
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

/// JNI: Get cohort history as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetCohortHistory<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let json = guard.cohort_history.to_json();
    drop(guard);
    env.new_string(&json).map(|s| s.into()).unwrap_or_else(|_| JObject::null())
}

/// JNI: Load cohort history from a previously-saved blob (called once on init)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeLoadCohortHistory<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    blob: JString<'local>,
) {
    let json: String = match env.get_string(&blob) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.cohort_history = crate::cohort_history::CohortHistory::load_from_json(
        &json, crate::cohort_history::DEFAULT_HISTORY_CAP,
    );
}

/// JNI: Remove a single cohort by id
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeRemoveCohort<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    cohort_id: JString<'local>,
) {
    let id: String = match env.get_string(&cohort_id) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.cohort_history.remove(&id);
}

/// JNI: Clear all cohort history
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeClearCohortHistory(
    _env: JNIEnv,
    _class: JClass,
) {
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.cohort_history.clear();
}

/// JNI: Get the active cohort_id for a channel (empty string if no active session there)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetActiveCohortId<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let cid = guard.session_manager.get_active_cohort_id(channel as u8).unwrap_or_default();
    drop(guard);
    env.new_string(&cid).map(|s| s.into()).unwrap_or_else(|_| JObject::null())
}

/// JNI: Snapshot participants for the active cohort on a given channel.
/// Called by Kotlin every ~30s while a session is active.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSnapshotCohortParticipants<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
    participants_json: JString<'local>,
) {
    let json: String = match env.get_string(&participants_json) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let participants: Vec<crate::cohort_history::ParticipantSnapshot> =
        serde_json::from_str(&json).unwrap_or_default();

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cid) = guard.session_manager.get_active_cohort_id(channel as u8) {
        guard.cohort_history.snapshot_participants(&cid, participants);
    }
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

    // Read from the canonical registry (the StateMachine's when running, where
    // the RX thread registers users), never a split pre-init copy.
    let reg_arc = guard.active_user_registry();
    let json = reg_arc.lock().unwrap_or_else(|e| e.into_inner()).to_json();
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
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    // Write to the canonical registry (the StateMachine's when running, where
    // the RX thread reads the mute filter), never a split pre-init copy.
    let reg_arc = guard.active_user_registry();
    reg_arc.lock().unwrap_or_else(|e| e.into_inner()).set_muted(&id, muted == JNI_TRUE);
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
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    // Write to the canonical registry (the StateMachine's when running, where
    // the RX thread reads the favorite ordering), never a split pre-init copy.
    let reg_arc = guard.active_user_registry();
    reg_arc.lock().unwrap_or_else(|e| e.into_inner()).set_favorite(&id, favorite == JNI_TRUE);
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

/// JNI: Get per-channel info as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetChannelInfo<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let json = guard.session_manager.get_channel_info();
    drop(guard);
    env.new_string(&json).map(|s| s.into()).unwrap_or_else(|_| JObject::null())
}

/// JNI: Set custom group name for a channel
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetGroupName<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
    name: JString<'local>,
) {
    let group_name: String = match env.get_string(&name) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.session_manager.set_group_name(channel as u8, &group_name);
}

/// JNI: Get group name for a specific channel
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetGroupName<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let name = guard.session_manager.get_group_name(channel as u8);
    drop(guard);
    env.new_string(&name).map(|s| s.into()).unwrap_or_else(|_| JObject::null())
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
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    // Register into the canonical registry so a manually-registered user shows
    // up in the same list the RX thread and nativeGetUsers consult.
    let reg_arc = guard.active_user_registry();
    let (is_muted, is_fav) = {
        let mut reg = reg_arc.lock().unwrap_or_else(|e| e.into_inner());
        reg.register_user(&id, &name);
        (reg.is_muted(&id), reg.is_favorite(&id))
    };
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

    let reg_arc = guard.active_user_registry();
    // `favorites()` / `others()` return references borrowed from the registry,
    // so build the JSON string while the lock is still held.
    let json = {
        let reg = reg_arc.lock().unwrap_or_else(|e| e.into_inner());
        serde_json::json!({
            "favorites": reg.favorites(),
            "others": reg.others(),
        }).to_string()
    };

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

/// JNI: Enable or disable client-side PCM mixing for 2..=6 concurrent
/// speakers. When disabled (default) the cache flips Live→Queue on overlap;
/// when enabled it flips Live→Mix and serializes only when the speaker count
/// exceeds MIX_MAX_SPEAKERS (6).
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetMixModeEnabled(
    _env: JNIEnv,
    _class: JClass,
    enabled: jni::sys::jboolean,
) {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let mut cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.set_mix_mode_enabled(enabled != 0);
    }
}

/// JNI: Returns true if mix mode is currently enabled. Used by the Settings
/// screen to render the initial toggle state.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsMixModeEnabled(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        let cache = sm.get_audio_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.is_mix_mode_enabled() { 1 } else { 0 }
    } else {
        0
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

        // Sync mute/favorite from the StateMachine's own registry — the same
        // canonical instance the RX thread and JNI user ops now use.
        let users_json = sm
            .get_user_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .to_json();
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

// ══════════════════════════════════════════════════════════════════════════════
// BLUETOOTH TRANSPORT JNI EXPORTS
// ══════════════════════════════════════════════════════════════════════════════

/// JNI: Called by Kotlin when BT RFCOMM connects to a peer
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeBtConnected(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: Bluetooth RFCOMM connected");

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        sm.on_bluetooth_connected();
    }
}

/// JNI: Called by Kotlin when BT RFCOMM disconnects
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeBtDisconnected(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("JNI: Bluetooth disconnected");

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        sm.on_bluetooth_disconnected();
    }
}

// Whisper transcription JNI exports removed — transcription module stripped to slim down codebase.
