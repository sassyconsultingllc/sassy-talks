use crate::jni_bridge::get_jvm;
use jni::objects::{JObject, JValue};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PermissionState { Granted, Denied, Unknown }

pub struct AppPermissions {
    pub bluetooth_connect: PermissionState, pub bluetooth_scan: PermissionState,
    pub bluetooth_advertise: PermissionState, pub record_audio: PermissionState,
}
impl AppPermissions {
    pub fn new() -> Self { Self { bluetooth_connect: PermissionState::Unknown, bluetooth_scan: PermissionState::Unknown, bluetooth_advertise: PermissionState::Unknown, record_audio: PermissionState::Unknown } }
    pub fn all_granted(&self) -> bool { self.bluetooth_connect == PermissionState::Granted && self.bluetooth_scan == PermissionState::Granted && self.record_audio == PermissionState::Granted }
    pub fn get_missing_permissions(&self) -> Vec<String> {
        let mut m = Vec::new();
        if self.bluetooth_connect != PermissionState::Granted { m.push("android.permission.BLUETOOTH_CONNECT".into()); }
        if self.bluetooth_scan != PermissionState::Granted { m.push("android.permission.BLUETOOTH_SCAN".into()); }
        if self.bluetooth_advertise != PermissionState::Granted { m.push("android.permission.BLUETOOTH_ADVERTISE".into()); }
        if self.record_audio != PermissionState::Granted { m.push("android.permission.RECORD_AUDIO".into()); }
        m
    }
}

const PERMISSION_GRANTED: i32 = 0;

pub struct PermissionManager { permissions: AppPermissions }

impl PermissionManager {
    pub fn new() -> Self { Self { permissions: AppPermissions::new() } }
    fn check_permission_jni(&self, permission: &str) -> PermissionState {
        let vm = match get_jvm() { Ok(v) => v, Err(_) => return PermissionState::Unknown };
        let mut env = match vm.attach_current_thread() { Ok(e) => e, Err(_) => return PermissionState::Unknown };
        let atc = match env.find_class("android/app/ActivityThread") { Ok(c) => c, Err(_) => return PermissionState::Unknown };
        let app = match env.call_static_method(atc, "currentApplication", "()Landroid/app/Application;", &[]) {
            Ok(r) => match r.l() { Ok(o) => o, Err(_) => return PermissionState::Unknown }, Err(_) => return PermissionState::Unknown };
        let ctx = match env.call_method(&app, "getApplicationContext", "()Landroid/content/Context;", &[]) {
            Ok(r) => match r.l() { Ok(o) => o, Err(_) => return PermissionState::Unknown }, Err(_) => return PermissionState::Unknown };
        let perm_str = match env.new_string(permission) { Ok(s) => s, Err(_) => return PermissionState::Unknown };
        let perm_obj: JObject = unsafe { JObject::from_raw(perm_str.as_raw()) };
        let result = match env.call_method(&ctx, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&perm_obj)]) {
            Ok(r) => match r.i() { Ok(i) => i, Err(_) => return PermissionState::Unknown }, Err(_) => return PermissionState::Unknown };
        if result == PERMISSION_GRANTED { PermissionState::Granted } else { PermissionState::Denied }
    }
    pub fn check_permissions(&mut self) -> bool {
        self.permissions.bluetooth_connect = self.check_permission_jni("android.permission.BLUETOOTH_CONNECT");
        self.permissions.bluetooth_scan = self.check_permission_jni("android.permission.BLUETOOTH_SCAN");
        self.permissions.bluetooth_advertise = self.check_permission_jni("android.permission.BLUETOOTH_ADVERTISE");
        self.permissions.record_audio = self.check_permission_jni("android.permission.RECORD_AUDIO");
        self.permissions.all_granted()
    }
    pub fn request_permissions(&self) -> Vec<String> { self.permissions.get_missing_permissions() }
    #[allow(dead_code)]
    pub fn on_permission_result(&mut self, permission: &str, granted: bool) {
        let state = if granted { PermissionState::Granted } else { PermissionState::Denied };
        match permission {
            "android.permission.BLUETOOTH_CONNECT" => self.permissions.bluetooth_connect = state,
            "android.permission.BLUETOOTH_SCAN" => self.permissions.bluetooth_scan = state,
            "android.permission.BLUETOOTH_ADVERTISE" => self.permissions.bluetooth_advertise = state,
            "android.permission.RECORD_AUDIO" => self.permissions.record_audio = state,
            _ => {}
        }
    }
    pub fn get_permissions(&self) -> &AppPermissions { &self.permissions }
    #[allow(dead_code)]
    pub fn has_critical_permissions(&self) -> bool { self.permissions.all_granted() }
    pub fn get_permission_explanation(&self, permission: &str) -> String {
        match permission {
            "android.permission.BLUETOOTH_CONNECT" => "Bluetooth Connect required for peer connections.".into(),
            "android.permission.BLUETOOTH_SCAN" => "Bluetooth Scan required to discover nearby devices.".into(),
            "android.permission.BLUETOOTH_ADVERTISE" => "Bluetooth Advertise required for discoverability.".into(),
            "android.permission.RECORD_AUDIO" => "Microphone required for voice transmission.".into(),
            _ => format!("Permission {} required.", permission),
        }
    }
}
impl Default for PermissionManager { fn default() -> Self { Self::new() } }

pub fn show_permission_rationale(permission: &str) -> String {
    PermissionManager::new().get_permission_explanation(permission)
}
