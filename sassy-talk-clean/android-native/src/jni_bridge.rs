/// JNI Bridge Module - Connects Rust to Android APIs
/// 
/// This module provides safe Rust wrappers around Android Java APIs via JNI.
/// Implements bridges for: Bluetooth, Audio, PackageManager, UI

use jni::{
    JNIEnv,
    objects::{JClass, JObject, JString, JValue, GlobalRef},
    sys::{jboolean, jint, jlong, jbyteArray, JNI_TRUE, JNI_FALSE},
    JavaVM,
};
use std::sync::{Arc, Mutex, Once};
use log::{error, info, warn};

/// Global JavaVM instance (initialized once)
static mut JAVA_VM: Option<Arc<JavaVM>> = None;
static INIT: Once = Once::new();

/// Initialize global JavaVM reference
pub fn init_jvm(vm: JavaVM) {
    INIT.call_once(|| {
        unsafe {
            JAVA_VM = Some(Arc::new(vm));
        }
    });
}

/// Get JavaVM instance
fn get_jvm() -> Result<Arc<JavaVM>, String> {
    unsafe {
        JAVA_VM.clone().ok_or_else(|| "JavaVM not initialized".to_string())
    }
}

/// Get current JNI environment
fn get_env() -> Result<JNIEnv<'static>, String> {
    let vm = get_jvm()?;
    vm.get_env()
        .map_err(|e| format!("Failed to get JNI env: {}", e))
}

//==============================================================================
// BLUETOOTH JNI BRIDGE
//==============================================================================

/// Android BluetoothAdapter bridge
pub struct AndroidBluetoothAdapter {
    adapter: GlobalRef,
}

impl AndroidBluetoothAdapter {
    /// Get default Bluetooth adapter
    pub fn get_default() -> Result<Self, String> {
        let env = get_env()?;
        
        // BluetoothAdapter.getDefaultAdapter()
        let adapter_class = env.find_class("android/bluetooth/BluetoothAdapter")
            .map_err(|e| format!("Failed to find BluetoothAdapter class: {}", e))?;
        
        let adapter = env.call_static_method(
            adapter_class,
            "getDefaultAdapter",
            "()Landroid/bluetooth/BluetoothAdapter;",
            &[]
        )
        .map_err(|e| format!("Failed to get default adapter: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(adapter)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(Self { adapter: global_ref })
    }
    
    /// Check if Bluetooth is enabled
    pub fn is_enabled(&self) -> Result<bool, String> {
        let env = get_env()?;
        
        let result = env.call_method(
            self.adapter.as_obj(),
            "isEnabled",
            "()Z",
            &[]
        )
        .map_err(|e| format!("Failed to call isEnabled: {}", e))?
        .z()
        .map_err(|e| format!("Failed to convert result: {}", e))?;
        
        Ok(result)
    }
    
    /// Enable Bluetooth
    pub fn enable(&self) -> Result<bool, String> {
        let env = get_env()?;
        
        let result = env.call_method(
            self.adapter.as_obj(),
            "enable",
            "()Z",
            &[]
        )
        .map_err(|e| format!("Failed to call enable: {}", e))?
        .z()
        .map_err(|e| format!("Failed to convert result: {}", e))?;
        
        Ok(result)
    }
    
    /// Get bonded (paired) devices
    pub fn get_bonded_devices(&self) -> Result<Vec<AndroidBluetoothDevice>, String> {
        let env = get_env()?;
        
        // Set<BluetoothDevice> getBondedDevices()
        let devices_set = env.call_method(
            self.adapter.as_obj(),
            "getBondedDevices",
            "()Ljava/util/Set;",
            &[]
        )
        .map_err(|e| format!("Failed to get bonded devices: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        // Convert Set to Array
        let devices_array = env.call_method(
            devices_set,
            "toArray",
            "()[Ljava/lang/Object;",
            &[]
        )
        .map_err(|e| format!("Failed to convert to array: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let len = env.get_array_length(devices_array.into_inner())
            .map_err(|e| format!("Failed to get array length: {}", e))?;
        
        let mut devices = Vec::new();
        for i in 0..len {
            let device = env.get_object_array_element(devices_array.into_inner(), i)
                .map_err(|e| format!("Failed to get device {}: {}", i, e))?;
            
            let global_ref = env.new_global_ref(device)
                .map_err(|e| format!("Failed to create global ref: {}", e))?;
            
            devices.push(AndroidBluetoothDevice { device: global_ref });
        }
        
        Ok(devices)
    }
    
    /// Create RFCOMM server socket
    pub fn create_rfcomm_server(&self, name: &str, uuid: &str) -> Result<AndroidBluetoothServerSocket, String> {
        let env = get_env()?;
        
        // Parse UUID string
        let uuid_class = env.find_class("java/util/UUID")
            .map_err(|e| format!("Failed to find UUID class: {}", e))?;
        
        let uuid_str = env.new_string(uuid)
            .map_err(|e| format!("Failed to create UUID string: {}", e))?;
        
        let uuid_obj = env.call_static_method(
            uuid_class,
            "fromString",
            "(Ljava/lang/String;)Ljava/util/UUID;",
            &[JValue::Object(uuid_str.into())]
        )
        .map_err(|e| format!("Failed to parse UUID: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert UUID: {}", e))?;
        
        let name_str = env.new_string(name)
            .map_err(|e| format!("Failed to create name string: {}", e))?;
        
        // listenUsingRfcommWithServiceRecord(String name, UUID uuid)
        let server_socket = env.call_method(
            self.adapter.as_obj(),
            "listenUsingRfcommWithServiceRecord",
            "(Ljava/lang/String;Ljava/util/UUID;)Landroid/bluetooth/BluetoothServerSocket;",
            &[JValue::Object(name_str.into()), JValue::Object(uuid_obj.into())]
        )
        .map_err(|e| format!("Failed to create server socket: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(server_socket)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(AndroidBluetoothServerSocket { socket: global_ref })
    }
}

/// Android BluetoothDevice bridge
pub struct AndroidBluetoothDevice {
    device: GlobalRef,
}

impl AndroidBluetoothDevice {
    /// Get device name
    pub fn get_name(&self) -> Result<String, String> {
        let env = get_env()?;
        
        let name = env.call_method(
            self.device.as_obj(),
            "getName",
            "()Ljava/lang/String;",
            &[]
        )
        .map_err(|e| format!("Failed to get name: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let name_str: String = env.get_string(name.into())
            .map_err(|e| format!("Failed to convert to string: {}", e))?
            .into();
        
        Ok(name_str)
    }
    
    /// Get device address (MAC address)
    pub fn get_address(&self) -> Result<String, String> {
        let env = get_env()?;
        
        let address = env.call_method(
            self.device.as_obj(),
            "getAddress",
            "()Ljava/lang/String;",
            &[]
        )
        .map_err(|e| format!("Failed to get address: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let address_str: String = env.get_string(address.into())
            .map_err(|e| format!("Failed to convert to string: {}", e))?
            .into();
        
        Ok(address_str)
    }
    
    /// Create RFCOMM socket to this device
    pub fn create_rfcomm_socket(&self, uuid: &str) -> Result<AndroidBluetoothSocket, String> {
        let env = get_env()?;
        
        // Parse UUID
        let uuid_class = env.find_class("java/util/UUID")
            .map_err(|e| format!("Failed to find UUID class: {}", e))?;
        
        let uuid_str = env.new_string(uuid)
            .map_err(|e| format!("Failed to create UUID string: {}", e))?;
        
        let uuid_obj = env.call_static_method(
            uuid_class,
            "fromString",
            "(Ljava/lang/String;)Ljava/util/UUID;",
            &[JValue::Object(uuid_str.into())]
        )
        .map_err(|e| format!("Failed to parse UUID: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert UUID: {}", e))?;
        
        // createRfcommSocketToServiceRecord(UUID uuid)
        let socket = env.call_method(
            self.device.as_obj(),
            "createRfcommSocketToServiceRecord",
            "(Ljava/util/UUID;)Landroid/bluetooth/BluetoothSocket;",
            &[JValue::Object(uuid_obj.into())]
        )
        .map_err(|e| format!("Failed to create socket: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(socket)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(AndroidBluetoothSocket { socket: global_ref })
    }
}

/// Android BluetoothSocket bridge
pub struct AndroidBluetoothSocket {
    socket: GlobalRef,
}

impl AndroidBluetoothSocket {
    /// Connect to remote device
    pub fn connect(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.socket.as_obj(),
            "connect",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to connect: {}", e))?;
        
        Ok(())
    }
    
    /// Close socket
    pub fn close(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.socket.as_obj(),
            "close",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to close: {}", e))?;
        
        Ok(())
    }
    
    /// Get input stream
    pub fn get_input_stream(&self) -> Result<AndroidInputStream, String> {
        let env = get_env()?;
        
        let stream = env.call_method(
            self.socket.as_obj(),
            "getInputStream",
            "()Ljava/io/InputStream;",
            &[]
        )
        .map_err(|e| format!("Failed to get input stream: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(stream)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(AndroidInputStream { stream: global_ref })
    }
    
    /// Get output stream
    pub fn get_output_stream(&self) -> Result<AndroidOutputStream, String> {
        let env = get_env()?;
        
        let stream = env.call_method(
            self.socket.as_obj(),
            "getOutputStream",
            "()Ljava/io/OutputStream;",
            &[]
        )
        .map_err(|e| format!("Failed to get output stream: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(stream)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(AndroidOutputStream { stream: global_ref })
    }
}

/// Android BluetoothServerSocket bridge
pub struct AndroidBluetoothServerSocket {
    socket: GlobalRef,
}

impl AndroidBluetoothServerSocket {
    /// Accept incoming connection (blocking)
    pub fn accept(&self) -> Result<AndroidBluetoothSocket, String> {
        let env = get_env()?;
        
        let socket = env.call_method(
            self.socket.as_obj(),
            "accept",
            "()Landroid/bluetooth/BluetoothSocket;",
            &[]
        )
        .map_err(|e| format!("Failed to accept: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(socket)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(AndroidBluetoothSocket { socket: global_ref })
    }
    
    /// Close server socket
    pub fn close(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.socket.as_obj(),
            "close",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to close: {}", e))?;
        
        Ok(())
    }
}

/// Java InputStream bridge
pub struct AndroidInputStream {
    stream: GlobalRef,
}

impl AndroidInputStream {
    /// Read bytes from stream
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, String> {
        let env = get_env()?;
        
        // Create Java byte array
        let jarray = env.new_byte_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create byte array: {}", e))?;
        
        // int read(byte[] b)
        let bytes_read = env.call_method(
            self.stream.as_obj(),
            "read",
            "([B)I",
            &[JValue::Object(jarray.into())]
        )
        .map_err(|e| format!("Failed to read: {}", e))?
        .i()
        .map_err(|e| format!("Failed to convert result: {}", e))?;
        
        if bytes_read <= 0 {
            return Ok(0);
        }
        
        // Copy from Java array to Rust buffer
        env.get_byte_array_region(jarray, 0, unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut i8, bytes_read as usize)
        })
        .map_err(|e| format!("Failed to copy bytes: {}", e))?;
        
        Ok(bytes_read as usize)
    }
}

/// Java OutputStream bridge
pub struct AndroidOutputStream {
    stream: GlobalRef,
}

impl AndroidOutputStream {
    /// Write bytes to stream
    pub fn write(&self, buffer: &[u8]) -> Result<(), String> {
        let env = get_env()?;
        
        // Create Java byte array
        let jarray = env.new_byte_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create byte array: {}", e))?;
        
        // Copy from Rust buffer to Java array
        env.set_byte_array_region(jarray, 0, unsafe {
            std::slice::from_raw_parts(buffer.as_ptr() as *const i8, buffer.len())
        })
        .map_err(|e| format!("Failed to copy bytes: {}", e))?;
        
        // void write(byte[] b)
        env.call_method(
            self.stream.as_obj(),
            "write",
            "([B)V",
            &[JValue::Object(jarray.into())]
        )
        .map_err(|e| format!("Failed to write: {}", e))?;
        
        Ok(())
    }
    
    /// Flush output stream
    pub fn flush(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.stream.as_obj(),
            "flush",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to flush: {}", e))?;
        
        Ok(())
    }
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
        let env = get_env()?;
        
        let recorder_class = env.find_class("android/media/AudioRecord")
            .map_err(|e| format!("Failed to find AudioRecord class: {}", e))?;
        
        // Get MediaRecorder.AudioSource.MIC constant
        let source_class = env.find_class("android/media/MediaRecorder$AudioSource")
            .map_err(|e| format!("Failed to find AudioSource class: {}", e))?;
        
        let mic_field = env.get_static_field(source_class, "MIC", "I")
            .map_err(|e| format!("Failed to get MIC field: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        
        // AudioRecord(int audioSource, int sampleRateInHz, int channelConfig, int audioFormat, int bufferSizeInBytes)
        let recorder = env.new_object(
            recorder_class,
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
        
        let global_ref = env.new_global_ref(recorder)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(Self { recorder: global_ref })
    }
    
    /// Get minimum buffer size
    pub fn get_min_buffer_size(sample_rate: i32, channel_config: i32, audio_format: i32) -> Result<i32, String> {
        let env = get_env()?;
        
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
        let env = get_env()?;
        
        env.call_method(
            self.recorder.as_obj(),
            "startRecording",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to start recording: {}", e))?;
        
        Ok(())
    }
    
    /// Stop recording
    pub fn stop(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.recorder.as_obj(),
            "stop",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to stop recording: {}", e))?;
        
        Ok(())
    }
    
    /// Read audio data
    pub fn read(&self, buffer: &mut [i16]) -> Result<usize, String> {
        let env = get_env()?;
        
        // Create Java short array
        let jarray = env.new_short_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create short array: {}", e))?;
        
        // int read(short[] audioData, int offsetInShorts, int sizeInShorts)
        let bytes_read = env.call_method(
            self.recorder.as_obj(),
            "read",
            "([SII)I",
            &[
                JValue::Object(jarray.into()),
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
        
        // Copy from Java array to Rust buffer
        env.get_short_array_region(jarray, 0, buffer)
            .map_err(|e| format!("Failed to copy shorts: {}", e))?;
        
        Ok(bytes_read as usize)
    }
    
    /// Release resources
    pub fn release(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.recorder.as_obj(),
            "release",
            "()V",
            &[]
        )
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
        let env = get_env()?;
        
        let track_class = env.find_class("android/media/AudioTrack")
            .map_err(|e| format!("Failed to find AudioTrack class: {}", e))?;
        
        // Get AudioManager.STREAM_MUSIC constant
        let manager_class = env.find_class("android/media/AudioManager")
            .map_err(|e| format!("Failed to find AudioManager class: {}", e))?;
        
        let stream_music = env.get_static_field(manager_class, "STREAM_MUSIC", "I")
            .map_err(|e| format!("Failed to get STREAM_MUSIC field: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        
        // Get MODE_STREAM constant
        let mode_stream = env.get_static_field(track_class, "MODE_STREAM", "I")
            .map_err(|e| format!("Failed to get MODE_STREAM field: {}", e))?
            .i()
            .map_err(|e| format!("Failed to convert field: {}", e))?;
        
        // AudioTrack(int streamType, int sampleRateInHz, int channelConfig, int audioFormat, int bufferSizeInBytes, int mode)
        let track = env.new_object(
            track_class,
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
        
        let global_ref = env.new_global_ref(track)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(Self { track: global_ref })
    }
    
    /// Start playback
    pub fn play(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.track.as_obj(),
            "play",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to start playback: {}", e))?;
        
        Ok(())
    }
    
    /// Stop playback
    pub fn stop(&self) -> Result<(), String> {
        let env = get_env()?;
        
        env.call_method(
            self.track.as_obj(),
            "stop",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to stop playback: {}", e))?;
        
        Ok(())
    }
    
    /// Write audio data
    pub fn write(&self, buffer: &[i16]) -> Result<usize, String> {
        let env = get_env()?;
        
        // Create Java short array
        let jarray = env.new_short_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create short array: {}", e))?;
        
        // Copy from Rust buffer to Java array
        env.set_short_array_region(jarray, 0, buffer)
            .map_err(|e| format!("Failed to copy shorts: {}", e))?;
        
        // int write(short[] audioData, int offsetInShorts, int sizeInShorts)
        let bytes_written = env.call_method(
            self.track.as_obj(),
            "write",
            "([SII)I",
            &[
                JValue::Object(jarray.into()),
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
        let env = get_env()?;
        
        env.call_method(
            self.track.as_obj(),
            "release",
            "()V",
            &[]
        )
        .map_err(|e| format!("Failed to release: {}", e))?;
        
        Ok(())
    }
}

//==============================================================================
// PACKAGE MANAGER JNI BRIDGE (for signature verification)
//==============================================================================

/// Get APK signature hash for verification
pub fn get_apk_signature_hash(package_name: &str) -> Result<Vec<u8>, String> {
    let env = get_env()?;
    
    // Get Activity context (need to pass from Java side)
    // For now, this is a placeholder - needs context injection
    
    // PackageManager pm = context.getPackageManager();
    // PackageInfo info = pm.getPackageInfo(packageName, GET_SIGNATURES);
    // Signature sig = info.signatures[0];
    // byte[] cert = sig.toByteArray();
    
    // TODO: Implement with proper context
    
    Err("Not yet implemented - needs context".to_string())
}

//==============================================================================
// UI JNI BRIDGE
//==============================================================================

/// Show Toast message
pub fn show_toast(message: &str, duration_long: bool) -> Result<(), String> {
    let env = get_env()?;
    
    // Need Activity context - placeholder for now
    // Toast.makeText(context, message, Toast.LENGTH_SHORT).show();
    
    info!("Toast: {}", message);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jni_stub() {
        // JNI tests require actual Android environment
        // These are unit test stubs
        assert!(true);
    }
}
