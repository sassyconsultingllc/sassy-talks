/// JNI Bridge - Android ↔ Rust Interface
/// Class: com.sassyconsulting.sassytalkie.SassyTalkNative

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue, GlobalRef};
use jni::sys::{jboolean, jbyte, jint, jstring, JNI_TRUE, JNI_FALSE};
use jni::JavaVM;
use std::sync::{Mutex, Arc};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use log::{error, info};

use crate::state::{StateMachine, AppState};
use crate::transport::ActiveTransport;
use crate::wifi_transport::WifiState;
use crate::session::SessionManager;
use crate::users::UserRegistry;
use crate::audio_cache::{AudioCache, CacheMode};
use crate::crypto::{CryptoSession, KeyExchange, generate_psk};
use crate::permissions::PermissionManager;

// ── Global JVM ──
use std::sync::OnceLock;
static GLOBAL_JVM: OnceLock<JavaVM> = OnceLock::new();

pub fn init_jvm(vm: JavaVM) {
    let _ = GLOBAL_JVM.set(vm);
}

pub fn get_jvm() -> Result<&'static JavaVM, String> {
    GLOBAL_JVM.get().ok_or_else(|| "JVM not initialized".into())
}

// ── Global App State ──
static APP: OnceLock<AppGlobals> = OnceLock::new();

struct AppGlobals {
    state_machine: Mutex<Option<StateMachine>>,
    session_manager: Mutex<SessionManager>,
    user_registry: Mutex<UserRegistry>,
    audio_cache: Mutex<AudioCache>,
    permission_manager: Mutex<PermissionManager>,
    ptt_pressed: AtomicBool,
    current_channel: AtomicU8,
    crypto: Mutex<Option<CryptoSession>>,
    key_exchange: Mutex<Option<KeyExchange>>,
}

fn app() -> &'static AppGlobals {
    APP.get().expect("App not initialized")
}

fn init_app() {
    let _ = APP.set(AppGlobals {
        state_machine: Mutex::new(None),
        session_manager: Mutex::new(SessionManager::new("SassyTalkie-Android")),
        user_registry: Mutex::new(UserRegistry::new()),
        audio_cache: Mutex::new(AudioCache::new()),
        permission_manager: Mutex::new(PermissionManager::new()),
        ptt_pressed: AtomicBool::new(false),
        current_channel: AtomicU8::new(1),
        crypto: Mutex::new(None),
        key_exchange: Mutex::new(None),
    });
}

// ── JNI Wrapper Types (unchanged from before) ──

pub struct AndroidBluetoothAdapter { pub global_ref: GlobalRef }
impl AndroidBluetoothAdapter {
    pub fn get_default() -> Result<Self, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let cls = env.find_class("android/bluetooth/BluetoothAdapter").map_err(|e| format!("{}", e))?;
        let a = env.call_static_method(cls, "getDefaultAdapter", "()Landroid/bluetooth/BluetoothAdapter;", &[])
            .map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        if a.is_null() { return Err("No BT adapter".into()); }
        Ok(Self { global_ref: env.new_global_ref(a).map_err(|e| format!("{}", e))? })
    }
    pub fn is_enabled(&self) -> Result<bool, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        Ok(env.call_method(&self.global_ref, "isEnabled", "()Z", &[]).map_err(|e| format!("{}", e))?.z().map_err(|e| format!("{}", e))?)
    }
    pub fn enable(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "enable", "()Z", &[]).map_err(|e| format!("{}", e))?;
        Ok(())
    }
    pub fn get_bonded_devices(&self) -> Result<Vec<AndroidBluetoothDevice>, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let set = env.call_method(&self.global_ref, "getBondedDevices", "()Ljava/util/Set;", &[])
            .map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        let iter = env.call_method(&set, "iterator", "()Ljava/util/Iterator;", &[])
            .map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        let mut devs = Vec::new();
        loop {
            let has = env.call_method(&iter, "hasNext", "()Z", &[]).map_err(|e| format!("{}", e))?.z().map_err(|e| format!("{}", e))?;
            if !has { break; }
            let d = env.call_method(&iter, "next", "()Ljava/lang/Object;", &[]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
            devs.push(AndroidBluetoothDevice { global_ref: env.new_global_ref(d).map_err(|e| format!("{}", e))? });
        }
        Ok(devs)
    }
    pub fn create_rfcomm_server(&self, name: &str, uuid: &str) -> Result<AndroidBluetoothServerSocket, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let n = env.new_string(name).map_err(|e| format!("{}", e))?;
        let uc = env.find_class("java/util/UUID").map_err(|e| format!("{}", e))?;
        let us = env.new_string(uuid).map_err(|e| format!("{}", e))?;
        let uo = env.call_static_method(uc, "fromString", "(Ljava/lang/String;)Ljava/util/UUID;", &[JValue::Object(&us.into())]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        let ss = env.call_method(&self.global_ref, "listenUsingRfcommWithServiceRecord", "(Ljava/lang/String;Ljava/util/UUID;)Landroid/bluetooth/BluetoothServerSocket;", &[JValue::Object(&n.into()), JValue::Object(&uo)]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        Ok(AndroidBluetoothServerSocket { global_ref: env.new_global_ref(ss).map_err(|e| format!("{}", e))? })
    }
}

pub struct AndroidBluetoothDevice { pub global_ref: GlobalRef }
impl AndroidBluetoothDevice {
    pub fn get_name(&self) -> Result<String, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let n = env.call_method(&self.global_ref, "getName", "()Ljava/lang/String;", &[]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        Ok(env.get_string(&JString::from(n)).map_err(|e| format!("{}", e))?.into())
    }
    pub fn get_address(&self) -> Result<String, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let a = env.call_method(&self.global_ref, "getAddress", "()Ljava/lang/String;", &[]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        Ok(env.get_string(&JString::from(a)).map_err(|e| format!("{}", e))?.into())
    }
    pub fn create_rfcomm_socket(&self, uuid: &str) -> Result<AndroidBluetoothSocket, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let uc = env.find_class("java/util/UUID").map_err(|e| format!("{}", e))?;
        let us = env.new_string(uuid).map_err(|e| format!("{}", e))?;
        let uo = env.call_static_method(uc, "fromString", "(Ljava/lang/String;)Ljava/util/UUID;", &[JValue::Object(&us.into())]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        let s = env.call_method(&self.global_ref, "createRfcommSocketToServiceRecord", "(Ljava/util/UUID;)Landroid/bluetooth/BluetoothSocket;", &[JValue::Object(&uo)]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        Ok(AndroidBluetoothSocket { global_ref: env.new_global_ref(s).map_err(|e| format!("{}", e))? })
    }
}

pub struct AndroidBluetoothSocket { pub global_ref: GlobalRef }
impl AndroidBluetoothSocket {
    pub fn connect(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "connect", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
    pub fn close(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "close", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
    pub fn get_input_stream(&self) -> Result<AndroidInputStream, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let s = env.call_method(&self.global_ref, "getInputStream", "()Ljava/io/InputStream;", &[]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        Ok(AndroidInputStream { global_ref: env.new_global_ref(s).map_err(|e| format!("{}", e))? })
    }
    pub fn get_output_stream(&self) -> Result<AndroidOutputStream, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let s = env.call_method(&self.global_ref, "getOutputStream", "()Ljava/io/OutputStream;", &[]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        Ok(AndroidOutputStream { global_ref: env.new_global_ref(s).map_err(|e| format!("{}", e))? })
    }
}

pub struct AndroidBluetoothServerSocket { pub global_ref: GlobalRef }
impl AndroidBluetoothServerSocket {
    pub fn accept(&self) -> Result<AndroidBluetoothSocket, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let s = env.call_method(&self.global_ref, "accept", "()Landroid/bluetooth/BluetoothSocket;", &[]).map_err(|e| format!("{}", e))?.l().map_err(|e| format!("{}", e))?;
        Ok(AndroidBluetoothSocket { global_ref: env.new_global_ref(s).map_err(|e| format!("{}", e))? })
    }
    pub fn close(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "close", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
}

pub struct AndroidInputStream { pub global_ref: GlobalRef }
impl AndroidInputStream {
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let jbuf = env.new_byte_array(buf.len() as i32).map_err(|e| format!("{}", e))?;
        let jbuf_obj: JObject = unsafe { JObject::from_raw(jbuf.as_raw()) };
        let n = env.call_method(&self.global_ref, "read", "([B)I", &[JValue::Object(&jbuf_obj)]).map_err(|e| format!("{}", e))?.i().map_err(|e| format!("{}", e))?;
        if n < 0 { return Err("EOF".into()); }
        let mut tmp = vec![0i8; n as usize];
        env.get_byte_array_region(&jbuf, 0, &mut tmp).map_err(|e| format!("{}", e))?;
        for (i, &b) in tmp.iter().enumerate() { buf[i] = b as u8; }
        Ok(n as usize)
    }
}

pub struct AndroidOutputStream { pub global_ref: GlobalRef }
impl AndroidOutputStream {
    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let jdata: Vec<i8> = data.iter().map(|&b| b as i8).collect();
        let jbuf = env.new_byte_array(data.len() as i32).map_err(|e| format!("{}", e))?;
        env.set_byte_array_region(&jbuf, 0, &jdata).map_err(|e| format!("{}", e))?;
        let jbuf_obj: JObject = unsafe { JObject::from_raw(jbuf.as_raw()) };
        env.call_method(&self.global_ref, "write", "([B)V", &[JValue::Object(&jbuf_obj)]).map_err(|e| format!("{}", e))?;
        Ok(())
    }
    pub fn flush(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "flush", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
}

pub struct AndroidAudioRecord { pub global_ref: GlobalRef }
impl AndroidAudioRecord {
    pub fn get_min_buffer_size(sr: i32, ch: i32, fmt: i32) -> Result<i32, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let cls = env.find_class("android/media/AudioRecord").map_err(|e| format!("{}", e))?;
        Ok(env.call_static_method(cls, "getMinBufferSize", "(III)I", &[JValue::Int(sr), JValue::Int(ch), JValue::Int(fmt)]).map_err(|e| format!("{}", e))?.i().map_err(|e| format!("{}", e))?)
    }
    pub fn new(sr: i32, ch: i32, fmt: i32, bufsz: i32) -> Result<Self, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let cls = env.find_class("android/media/AudioRecord").map_err(|e| format!("{}", e))?;
        let obj = env.new_object(cls, "(IIIII)V", &[JValue::Int(1), JValue::Int(sr), JValue::Int(ch), JValue::Int(fmt), JValue::Int(bufsz)]).map_err(|e| format!("{}", e))?;
        Ok(Self { global_ref: env.new_global_ref(obj).map_err(|e| format!("{}", e))? })
    }
    pub fn start_recording(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "startRecording", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
    pub fn stop(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "stop", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
    pub fn read(&self, buf: &mut [i16]) -> Result<usize, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let jbuf = env.new_short_array(buf.len() as i32).map_err(|e| format!("{}", e))?;
        let jbuf_obj: JObject = unsafe { JObject::from_raw(jbuf.as_raw()) };
        let n = env.call_method(&self.global_ref, "read", "([SII)I", &[JValue::Object(&jbuf_obj), JValue::Int(0), JValue::Int(buf.len() as i32)]).map_err(|e| format!("{}", e))?.i().map_err(|e| format!("{}", e))?;
        if n > 0 { env.get_short_array_region(&jbuf, 0, &mut buf[..n as usize]).map_err(|e| format!("{}", e))?; }
        Ok(n.max(0) as usize)
    }
    pub fn release(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "release", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
}

pub struct AndroidAudioTrack { pub global_ref: GlobalRef }
impl AndroidAudioTrack {
    pub fn new(sr: i32, ch: i32, fmt: i32, bufsz: i32) -> Result<Self, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let cls = env.find_class("android/media/AudioTrack").map_err(|e| format!("{}", e))?;
        let obj = env.new_object(cls, "(IIIIII)V", &[JValue::Int(3), JValue::Int(sr), JValue::Int(ch), JValue::Int(fmt), JValue::Int(bufsz), JValue::Int(1)]).map_err(|e| format!("{}", e))?;
        Ok(Self { global_ref: env.new_global_ref(obj).map_err(|e| format!("{}", e))? })
    }
    pub fn play(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "play", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
    pub fn stop(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "stop", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
    pub fn write(&self, buf: &[i16]) -> Result<usize, String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        let jbuf = env.new_short_array(buf.len() as i32).map_err(|e| format!("{}", e))?;
        env.set_short_array_region(&jbuf, 0, buf).map_err(|e| format!("{}", e))?;
        let jbuf_obj: JObject = unsafe { JObject::from_raw(jbuf.as_raw()) };
        let n = env.call_method(&self.global_ref, "write", "([SII)I", &[JValue::Object(&jbuf_obj), JValue::Int(0), JValue::Int(buf.len() as i32)]).map_err(|e| format!("{}", e))?.i().map_err(|e| format!("{}", e))?;
        Ok(n.max(0) as usize)
    }
    pub fn release(&self) -> Result<(), String> {
        let vm = get_jvm()?; let mut env = vm.attach_current_thread().map_err(|e| format!("{}", e))?;
        env.call_method(&self.global_ref, "release", "()V", &[]).map_err(|e| format!("{}", e))?; Ok(())
    }
}

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════
fn js(env: &mut JNIEnv, s: JString) -> String { env.get_string(&s).map(|s| s.into()).unwrap_or_default() }
fn tj(env: &mut JNIEnv, s: &str) -> jstring { env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()) }
fn b(v: bool) -> jboolean { if v { JNI_TRUE } else { JNI_FALSE } }

// Macro for the class path prefix

// ══════════════════════════════════════════════════════════
// JNI EXPORTS — 42 methods matching SassyTalkNative.kt
// ══════════════════════════════════════════════════════════

// ── Lifecycle ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeInit(_env: JNIEnv, _class: JClass) -> jboolean {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("SassyTalk"));
    init_app();
    let a = app();
    let ptt = Arc::new(AtomicBool::new(false));
    let ch = Arc::new(AtomicU8::new(1));
    let sm = StateMachine::new(ptt, ch);
    match sm.initialize() {
        Ok(_) => { *a.state_machine.lock().unwrap() = Some(sm); b(true) }
        Err(e) => { error!("Init failed: {}", e); b(false) }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeShutdown(_env: JNIEnv, _class: JClass) {
    let a = app();
    if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        let _ = sm.disconnect();
    }
    *a.state_machine.lock().unwrap() = None;
    info!("Shutdown complete");
}

// ── PTT ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativePttStart(_env: JNIEnv, _class: JClass) {
    let a = app();
    a.ptt_pressed.store(true, Ordering::Relaxed);
    if let Some(sm) = a.state_machine.lock().unwrap().as_ref() { let _ = sm.on_ptt_press(); }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativePttStop(_env: JNIEnv, _class: JClass) {
    let a = app();
    a.ptt_pressed.store(false, Ordering::Relaxed);
    if let Some(sm) = a.state_machine.lock().unwrap().as_ref() { let _ = sm.on_ptt_release(); }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetChannel(_env: JNIEnv, _class: JClass, channel: jbyte) {
    app().current_channel.store(channel as u8, Ordering::Relaxed);
}

// ── Transport ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetTransport(_env: JNIEnv, _class: JClass) -> jbyte {
    let a = app();
    if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        match sm.get_active_transport() { ActiveTransport::None => 0, ActiveTransport::Bluetooth => 1, ActiveTransport::Wifi => 2 }
    } else { 0 }
}

// ── Device Management ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetPairedDevices(mut env: JNIEnv, _class: JClass) -> jstring {
    let a = app();
    let json = if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        match sm.get_paired_devices() {
            Ok(devs) => {
                let arr: Vec<String> = devs.iter().map(|d| {
                    format!("{{\"name\":\"{}\",\"address\":\"{}\"}}", d.name, d.address)
                }).collect();
                format!("[{}]", arr.join(","))
            }
            Err(_) => "[]".into()
        }
    } else { "[]".into() };
    tj(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeConnectDevice(mut env: JNIEnv, _class: JClass, address: JString) -> jboolean {
    let addr = js(&mut env, address);
    let a = app();
    b(a.state_machine.lock().unwrap().as_ref().map(|sm| sm.connect_to_device(&addr).is_ok()).unwrap_or(false))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeStartListening(_env: JNIEnv, _class: JClass) -> jboolean {
    let a = app();
    b(a.state_machine.lock().unwrap().as_ref().map(|sm| sm.start_listening().is_ok()).unwrap_or(false))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeDisconnect(_env: JNIEnv, _class: JClass) -> jboolean {
    let a = app();
    b(a.state_machine.lock().unwrap().as_ref().map(|sm| sm.disconnect().is_ok()).unwrap_or(false))
}

// ── QR Auth / Session ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGenerateSessionQR(mut env: JNIEnv, _class: JClass, hours: jint) -> jstring {
    let a = app();
    let r = a.session_manager.lock().unwrap().generate_session_qr(hours as u32);
    tj(&mut env, &r.unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e)))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeImportSessionFromQR(mut env: JNIEnv, _class: JClass, qr_json: JString) -> jboolean {
    let data = js(&mut env, qr_json);
    let a = app();
    b(a.session_manager.lock().unwrap().import_session(&data).is_ok())
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsAuthenticated(_env: JNIEnv, _class: JClass) -> jboolean {
    b(app().session_manager.lock().unwrap().is_authenticated())
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetSessionStatus(mut env: JNIEnv, _class: JClass) -> jstring {
    tj(&mut env, &app().session_manager.lock().unwrap().get_session_status())
}

// ── User Management ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetUsers(mut env: JNIEnv, _class: JClass) -> jstring {
    tj(&mut env, &app().user_registry.lock().unwrap().to_json())
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetMuted(mut env: JNIEnv, _class: JClass, user_id: JString, muted: jboolean) {
    let id = js(&mut env, user_id);
    app().user_registry.lock().unwrap().set_muted(&id, muted == JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetFavorite(mut env: JNIEnv, _class: JClass, user_id: JString, fav: jboolean) {
    let id = js(&mut env, user_id);
    app().user_registry.lock().unwrap().set_favorite(&id, fav == JNI_TRUE);
}

// ── BT Status ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsBluetoothEnabled(_env: JNIEnv, _class: JClass) -> jboolean {
    let a = app();
    b(a.state_machine.lock().unwrap().as_ref().map(|sm| sm.is_bluetooth_enabled()).unwrap_or(false))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeEnableBluetooth(_env: JNIEnv, _class: JClass) -> jboolean {
    let a = app();
    b(a.state_machine.lock().unwrap().as_ref().map(|sm| sm.enable_bluetooth().is_ok()).unwrap_or(false))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetConnectedDevice(mut env: JNIEnv, _class: JClass) -> jstring {
    let a = app();
    let json = if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        sm.get_connected_device().map(|d| format!("{{\"name\":\"{}\",\"address\":\"{}\"}}", d.name, d.address)).unwrap_or_else(|| "{}".into())
    } else { "{}".into() };
    tj(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetAppState(_env: JNIEnv, _class: JClass) -> jbyte {
    let a = app();
    if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        match sm.get_state() {
            AppState::Initializing => 0, AppState::Ready => 1, AppState::Connecting => 2,
            AppState::Connected => 3, AppState::Transmitting => 4, AppState::Receiving => 5,
            AppState::Disconnecting => 6, AppState::Error => 7,
        }
    } else { 0 }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeClearSession(_env: JNIEnv, _class: JClass) {
    app().session_manager.lock().unwrap().clear_session();
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeRegisterUser(mut env: JNIEnv, _class: JClass, user_id: JString, user_name: JString) {
    let id = js(&mut env, user_id);
    let name = js(&mut env, user_name);
    app().user_registry.lock().unwrap().register_user(&id, &name);
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetFavorites(mut env: JNIEnv, _class: JClass) -> jstring {
    let reg = app().user_registry.lock().unwrap();
    let favs = reg.favorites();
    let arr: Vec<String> = favs.iter().map(|u| format!("{{\"id\":\"{}\",\"name\":\"{}\"}}", u.id, u.name)).collect();
    tj(&mut env, &format!("[{}]", arr.join(",")))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeDeriveUserId(mut env: JNIEnv, _class: JClass, session_key: JString) -> jstring {
    let key = js(&mut env, session_key);
    let id = crate::users::UserRegistry::derive_user_id(key.as_bytes());
    tj(&mut env, &id)
}

// ── Crypto ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGeneratePsk(mut env: JNIEnv, _class: JClass) -> jstring {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let psk = generate_psk();
    tj(&mut env, &STANDARD.encode(&psk))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetPsk(mut env: JNIEnv, _class: JClass, psk: JString) -> jboolean {
    let psk_str = js(&mut env, psk);
    use base64::{Engine, engine::general_purpose::STANDARD};
    let a = app();
    match STANDARD.decode(&psk_str) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            *a.crypto.lock().unwrap() = Some(CryptoSession::from_psk(&key));
            b(true)
        }
        _ => b(false)
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeKeyExchangeInit(mut env: JNIEnv, _class: JClass) -> jstring {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let kx = KeyExchange::new();
    let pub_b64 = STANDARD.encode(&kx.public_key_bytes());
    *app().key_exchange.lock().unwrap() = Some(kx);
    tj(&mut env, &pub_b64)
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeKeyExchangeComplete(mut env: JNIEnv, _class: JClass, remote_pub: JString) -> jboolean {
    let rpub = js(&mut env, remote_pub);
    use base64::{Engine, engine::general_purpose::STANDARD};
    let a = app();
    let kx = a.key_exchange.lock().unwrap().take();
    match (kx, STANDARD.decode(&rpub)) {
        (Some(kx), Ok(bytes)) if bytes.len() == 32 => {
            let mut remote = [0u8; 32];
            remote.copy_from_slice(&bytes);
            match kx.complete(&remote) {
                Ok(session) => { *a.crypto.lock().unwrap() = Some(session); b(true) }
                Err(_) => b(false)
            }
        }
        _ => b(false)
    }
}

// ── Permissions ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeCheckPermissions(mut env: JNIEnv, _class: JClass) -> jstring {
    let a = app();
    let pm = a.permission_manager.lock().unwrap();
    let perms = pm.get_permissions();
    let json = format!("{{\"bluetooth_connect\":\"{:?}\",\"bluetooth_scan\":\"{:?}\",\"bluetooth_advertise\":\"{:?}\",\"record_audio\":\"{:?}\"}}",
        perms.bluetooth_connect, perms.bluetooth_scan, perms.bluetooth_advertise, perms.record_audio);
    tj(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeOnPermissionResult(mut env: JNIEnv, _class: JClass, perm: JString, granted: jboolean) {
    let p = js(&mut env, perm);
    app().permission_manager.lock().unwrap().on_permission_result(&p, granted == JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetMissingPermissions(mut env: JNIEnv, _class: JClass) -> jstring {
    let missing = app().permission_manager.lock().unwrap().request_permissions();
    let arr: Vec<String> = missing.iter().map(|s| format!("\"{}\"", s)).collect();
    tj(&mut env, &format!("[{}]", arr.join(",")))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetPermissionRationale(mut env: JNIEnv, _class: JClass, perm: JString) -> jstring {
    let p = js(&mut env, perm);
    tj(&mut env, &crate::permissions::show_permission_rationale(&p))
}

// ── WiFi ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetWifiState(_env: JNIEnv, _class: JClass) -> jbyte {
    let a = app();
    if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        match sm.wifi_state() {
            WifiState::Inactive => 0, WifiState::Discovering => 1,
            WifiState::Active => 2, WifiState::Error => 3,
        }
    } else { 0 }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetWifiPeers(mut env: JNIEnv, _class: JClass) -> jstring {
    let a = app();
    let json = if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        sm.get_wifi_peers_json()
    } else { "[]".into() };
    tj(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeHasWifiPeers(_env: JNIEnv, _class: JClass) -> jboolean {
    let a = app();
    b(a.state_machine.lock().unwrap().as_ref().map(|sm| sm.has_wifi_peers()).unwrap_or(false))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeInitWifi(_env: JNIEnv, _class: JClass) -> jboolean {
    let a = app();
    b(a.state_machine.lock().unwrap().as_ref().map(|sm| sm.init_wifi().is_ok()).unwrap_or(false))
}

// ── BT State / PTT / Device ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetBtState(_env: JNIEnv, _class: JClass) -> jbyte {
    // 0=Disconnected, 1=Connecting, 2=Connected, 3=Listening
    let a = app();
    if let Some(sm) = a.state_machine.lock().unwrap().as_ref() {
        match sm.get_state() {
            AppState::Connecting => 1, AppState::Connected | AppState::Transmitting | AppState::Receiving => 2,
            _ => 0,
        }
    } else { 0 }
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsPttActive(_env: JNIEnv, _class: JClass) -> jboolean {
    b(app().ptt_pressed.load(Ordering::Relaxed))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetDeviceName(mut env: JNIEnv, _class: JClass) -> jstring {
    let a = app();
    let name = a.state_machine.lock().unwrap().as_ref()
        .map(|sm| sm.get_device_name())
        .unwrap_or_else(|| "SassyTalkie-Android".into());
    tj(&mut env, &name)
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsEncrypted(_env: JNIEnv, _class: JClass) -> jboolean {
    let a = app();
    let transport_encrypted = a.state_machine.lock().unwrap().as_ref().map(|sm| sm.is_encrypted()).unwrap_or(false);
    let session_auth = a.session_manager.lock().unwrap().is_authenticated();
    let has_crypto = a.crypto.lock().unwrap().is_some();
    b(transport_encrypted || session_auth || has_crypto)
}

// ── Audio Cache ──

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetCacheStatus(mut env: JNIEnv, _class: JClass) -> jstring {
    tj(&mut env, &app().audio_cache.lock().unwrap().status_json())
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSkipCurrentUtterance(_env: JNIEnv, _class: JClass) {
    app().audio_cache.lock().unwrap().skip_current();
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSetCacheMode(_env: JNIEnv, _class: JClass, mode: jbyte) {
    let m = match mode { 0 => CacheMode::Live, 1 => CacheMode::Queue, _ => CacheMode::Replay };
    app().audio_cache.lock().unwrap().set_mode(m);
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeClearAudioCache(_env: JNIEnv, _class: JClass) {
    app().audio_cache.lock().unwrap().clear();
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeReplayUtterance(_env: JNIEnv, _class: JClass, index: jint) -> jboolean {
    b(app().audio_cache.lock().unwrap().replay_from_history(index as usize))
}

#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSyncCacheUserInfo(_env: JNIEnv, _class: JClass) {
    let a = app();
    let registry = a.user_registry.lock().unwrap();
    let mut cache = a.audio_cache.lock().unwrap();
    let json = registry.to_json();
    // Parse user profiles and sync to cache
    if let Ok(users) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
        for u in &users {
            let id = u["id"].as_str().unwrap_or("");
            let name = u["name"].as_str().unwrap_or("");
            let is_fav = u["is_favorite"].as_bool().unwrap_or(false);
            let is_muted = u["is_muted"].as_bool().unwrap_or(false);
            cache.update_user_info(id, name, is_fav, is_muted);
        }
    }
}
