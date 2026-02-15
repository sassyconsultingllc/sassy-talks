/// JNI Bridge Module - Connects Rust to Android APIs
/// 
/// This module provides safe Rust wrappers around Android Java APIs via JNI.
/// Implements bridges for: Bluetooth, Audio, PackageManager, UI

use jni::{
    JNIEnv,
    objects::{JClass, JObject, JString, JValue, GlobalRef, JObjectArray},
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
// BLUETOOTH JNI BRIDGE
//==============================================================================

/// Android BluetoothAdapter bridge
pub struct AndroidBluetoothAdapter {
    adapter: GlobalRef,
}

impl AndroidBluetoothAdapter {
    /// Get default Bluetooth adapter
    pub fn get_default() -> Result<Self, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
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
        
        let global_ref = env.new_global_ref(&adapter)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(Self { adapter: global_ref })
    }
    
    /// Check if Bluetooth is enabled
    pub fn is_enabled(&self) -> Result<bool, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
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
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
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
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let devices_set = env.call_method(
            self.adapter.as_obj(),
            "getBondedDevices",
            "()Ljava/util/Set;",
            &[]
        )
        .map_err(|e| format!("Failed to get bonded devices: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let devices_array = env.call_method(
            &devices_set,
            "toArray",
            "()[Ljava/lang/Object;",
            &[]
        )
        .map_err(|e| format!("Failed to convert to array: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let array: JObjectArray = devices_array.into();
        let len = env.get_array_length(&array)
            .map_err(|e| format!("Failed to get array length: {}", e))?;
        
        let mut devices = Vec::new();
        for i in 0..len {
            let device = env.get_object_array_element(&array, i)
                .map_err(|e| format!("Failed to get device {}: {}", i, e))?;
            
            let global_ref = env.new_global_ref(&device)
                .map_err(|e| format!("Failed to create global ref: {}", e))?;
            
            devices.push(AndroidBluetoothDevice { device: global_ref });
        }
        
        Ok(devices)
    }
    
    /// Create RFCOMM server socket
    pub fn create_rfcomm_server(&self, name: &str, uuid: &str) -> Result<AndroidBluetoothServerSocket, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let uuid_class = env.find_class("java/util/UUID")
            .map_err(|e| format!("Failed to find UUID class: {}", e))?;
        
        let uuid_jstr = env.new_string(uuid)
            .map_err(|e| format!("Failed to create UUID string: {}", e))?;
        
        let uuid_obj = env.call_static_method(
            uuid_class,
            "fromString",
            "(Ljava/lang/String;)Ljava/util/UUID;",
            &[JValue::Object(&uuid_jstr.into())]
        )
        .map_err(|e| format!("Failed to parse UUID: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert UUID: {}", e))?;
        
        let name_jstr = env.new_string(name)
            .map_err(|e| format!("Failed to create name string: {}", e))?;
        
        let server_socket = env.call_method(
            self.adapter.as_obj(),
            "listenUsingRfcommWithServiceRecord",
            "(Ljava/lang/String;Ljava/util/UUID;)Landroid/bluetooth/BluetoothServerSocket;",
            &[JValue::Object(&name_jstr.into()), JValue::Object(&uuid_obj)]
        )
        .map_err(|e| format!("Failed to create server socket: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(&server_socket)
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
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let name = env.call_method(
            self.device.as_obj(),
            "getName",
            "()Ljava/lang/String;",
            &[]
        )
        .map_err(|e| format!("Failed to get name: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let name_jstr = JString::from(name);
        let name_str: String = env.get_string(&name_jstr)
            .map_err(|e| format!("Failed to convert to string: {}", e))?
            .into();
        
        Ok(name_str)
    }
    
    /// Get device address (MAC address)
    pub fn get_address(&self) -> Result<String, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let address = env.call_method(
            self.device.as_obj(),
            "getAddress",
            "()Ljava/lang/String;",
            &[]
        )
        .map_err(|e| format!("Failed to get address: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let addr_jstr = JString::from(address);
        let address_str: String = env.get_string(&addr_jstr)
            .map_err(|e| format!("Failed to convert to string: {}", e))?
            .into();
        
        Ok(address_str)
    }
    
    /// Create RFCOMM socket to this device
    pub fn create_rfcomm_socket(&self, uuid: &str) -> Result<AndroidBluetoothSocket, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let uuid_class = env.find_class("java/util/UUID")
            .map_err(|e| format!("Failed to find UUID class: {}", e))?;
        
        let uuid_jstr = env.new_string(uuid)
            .map_err(|e| format!("Failed to create UUID string: {}", e))?;
        
        let uuid_obj = env.call_static_method(
            uuid_class,
            "fromString",
            "(Ljava/lang/String;)Ljava/util/UUID;",
            &[JValue::Object(&uuid_jstr.into())]
        )
        .map_err(|e| format!("Failed to parse UUID: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert UUID: {}", e))?;
        
        let socket = env.call_method(
            self.device.as_obj(),
            "createRfcommSocketToServiceRecord",
            "(Ljava/util/UUID;)Landroid/bluetooth/BluetoothSocket;",
            &[JValue::Object(&uuid_obj)]
        )
        .map_err(|e| format!("Failed to create socket: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(&socket)
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
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.socket.as_obj(), "connect", "()V", &[])
            .map_err(|e| format!("Failed to connect: {}", e))?;
        
        Ok(())
    }
    
    /// Close socket
    pub fn close(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.socket.as_obj(), "close", "()V", &[])
            .map_err(|e| format!("Failed to close: {}", e))?;
        
        Ok(())
    }
    
    /// Get input stream
    pub fn get_input_stream(&self) -> Result<AndroidInputStream, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let stream = env.call_method(
            self.socket.as_obj(),
            "getInputStream",
            "()Ljava/io/InputStream;",
            &[]
        )
        .map_err(|e| format!("Failed to get input stream: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(&stream)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(AndroidInputStream { stream: global_ref })
    }
    
    /// Get output stream
    pub fn get_output_stream(&self) -> Result<AndroidOutputStream, String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let stream = env.call_method(
            self.socket.as_obj(),
            "getOutputStream",
            "()Ljava/io/OutputStream;",
            &[]
        )
        .map_err(|e| format!("Failed to get output stream: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(&stream)
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
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let socket = env.call_method(
            self.socket.as_obj(),
            "accept",
            "()Landroid/bluetooth/BluetoothSocket;",
            &[]
        )
        .map_err(|e| format!("Failed to accept: {}", e))?
        .l()
        .map_err(|e| format!("Failed to convert to object: {}", e))?;
        
        let global_ref = env.new_global_ref(&socket)
            .map_err(|e| format!("Failed to create global ref: {}", e))?;
        
        Ok(AndroidBluetoothSocket { socket: global_ref })
    }
    
    /// Close server socket
    pub fn close(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.socket.as_obj(), "close", "()V", &[])
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
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let jarray = env.new_byte_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create byte array: {}", e))?;
        
        // Create JObject reference without consuming jarray
        let jarray_obj = unsafe { JObject::from_raw(jarray.as_raw()) };
        
        let bytes_read = env.call_method(
            self.stream.as_obj(),
            "read",
            "([B)I",
            &[JValue::Object(&jarray_obj)]
        )
        .map_err(|e| format!("Failed to read: {}", e))?
        .i()
        .map_err(|e| format!("Failed to convert result: {}", e))?;
        
        if bytes_read <= 0 {
            return Ok(0);
        }
        
        let mut temp = vec![0i8; bytes_read as usize];
        env.get_byte_array_region(&jarray, 0, &mut temp)
            .map_err(|e| format!("Failed to copy bytes: {}", e))?;
        
        for (i, &b) in temp.iter().enumerate() {
            buffer[i] = b as u8;
        }
        
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
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        let jarray = env.new_byte_array(buffer.len() as i32)
            .map_err(|e| format!("Failed to create byte array: {}", e))?;
        
        let temp: Vec<i8> = buffer.iter().map(|&b| b as i8).collect();
        env.set_byte_array_region(&jarray, 0, &temp)
            .map_err(|e| format!("Failed to copy bytes: {}", e))?;
        
        env.call_method(
            self.stream.as_obj(),
            "write",
            "([B)V",
            &[JValue::Object(&jarray.into())]
        )
        .map_err(|e| format!("Failed to write: {}", e))?;
        
        Ok(())
    }
    
    /// Flush output stream
    pub fn flush(&self) -> Result<(), String> {
        let vm = get_jvm()?;
        let mut env = vm.attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;
        
        env.call_method(self.stream.as_obj(), "flush", "()V", &[])
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

/// JNI: Get active transport type (0=None, 1=Bluetooth, 2=WiFi)
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
            crate::transport::ActiveTransport::Bluetooth => 1,
            crate::transport::ActiveTransport::Wifi => 2,
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

/// JNI: Get paired Bluetooth devices as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetPairedDevices<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = if let Some(ref sm) = guard.state_machine {
        match sm.get_paired_devices() {
            Ok(devices) => {
                let arr: Vec<serde_json::Value> = devices.iter().map(|d| {
                    serde_json::json!({"name": d.name, "address": d.address, "paired": d.paired})
                }).collect();
                serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
            }
            Err(e) => {
                error!("JNI: getPairedDevices failed: {}", e);
                "[]".to_string()
            }
        }
    } else {
        "[]".to_string()
    };

    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

/// JNI: Connect to a Bluetooth device by address
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeConnectDevice<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    address: JString<'local>,
) -> jboolean {
    let addr: String = match env.get_string(&address) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    info!("JNI: Connecting to {}", addr);

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.connect_to_device(&addr) {
            Ok(()) => JNI_TRUE,
            Err(e) => {
                error!("JNI: Connect failed: {}", e);
                JNI_FALSE
            }
        }
    } else {
        JNI_FALSE
    }
}

/// JNI: Start listening for incoming connections
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeStartListening(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    info!("JNI: Start listening");

    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.start_listening() {
            Ok(()) => JNI_TRUE,
            Err(e) => {
                error!("JNI: Listen failed: {}", e);
                JNI_FALSE
            }
        }
    } else {
        JNI_FALSE
    }
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

/// JNI: Check if Bluetooth is enabled
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeIsBluetoothEnabled(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        if sm.is_bluetooth_enabled() { JNI_TRUE } else { JNI_FALSE }
    } else {
        JNI_FALSE
    }
}

/// JNI: Enable Bluetooth
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeEnableBluetooth(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.enable_bluetooth() {
            Ok(()) => JNI_TRUE,
            Err(e) => {
                warn!("JNI: Enable BT failed: {}", e);
                JNI_FALSE
            }
        }
    } else {
        JNI_FALSE
    }
}

/// JNI: Get connected device info as JSON (name + address) or empty string
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetConnectedDevice<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let json = if let Some(ref sm) = guard.state_machine {
        match sm.get_connected_device() {
            Some(device) => serde_json::json!({
                "name": device.name,
                "address": device.address
            }).to_string(),
            None => "{}".to_string(),
        }
    } else {
        "{}".to_string()
    };

    drop(guard);

    env.new_string(&json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}

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
        "bluetooth_connect": format!("{:?}", perms.bluetooth_connect),
        "bluetooth_scan": format!("{:?}", perms.bluetooth_scan),
        "bluetooth_advertise": format!("{:?}", perms.bluetooth_advertise),
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

/// JNI: Get Bluetooth connection state (0=Disconnected, 1=Connecting, 2=Connected, 3=Listening)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetBtState(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(ref sm) = guard.state_machine {
        match sm.get_bt_state() {
            crate::bluetooth::ConnectionState::Disconnected => 0,
            crate::bluetooth::ConnectionState::Connecting => 1,
            crate::bluetooth::ConnectionState::Connected => 2,
            crate::bluetooth::ConnectionState::Listening => 3,
        }
    } else {
        0
    }
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
        let cache = sm.audio_cache().lock().unwrap_or_else(|e| e.into_inner());
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
        let mut cache = sm.audio_cache().lock().unwrap_or_else(|e| e.into_inner());
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
        let mut cache = sm.audio_cache().lock().unwrap_or_else(|e| e.into_inner());
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
        let mut cache = sm.audio_cache().lock().unwrap_or_else(|e| e.into_inner());
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
        let mut cache = sm.audio_cache().lock().unwrap_or_else(|e| e.into_inner());
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
        let mut cache = sm.audio_cache().lock().unwrap_or_else(|e| e.into_inner());

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
