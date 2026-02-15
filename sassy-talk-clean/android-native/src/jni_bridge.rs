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
use std::sync::{Arc, Once};
use log::{error, info};

/// Global JavaVM instance (initialized once)
static mut JAVA_VM: Option<Arc<JavaVM>> = None;
static INIT: Once = Once::new();

/// Initialize global JavaVM reference
pub fn init_jvm(vm: JavaVM) {
    INIT.call_once(|| {
        unsafe {
            JAVA_VM = Some(Arc::new(vm));
        }
        info!("JNI: JavaVM initialized");
    });
}

/// Get JavaVM instance
pub fn get_jvm() -> Result<Arc<JavaVM>, String> {
    unsafe {
        JAVA_VM.clone().ok_or_else(|| "JavaVM not initialized".to_string())
    }
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

/// Global state for JNI mode (when used from Kotlin instead of egui)
static JNI_STATE: OnceLock<Arc<Mutex<JniAppState>>> = OnceLock::new();

struct JniAppState {
    state_machine: Option<StateMachine>,
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
}

impl JniAppState {
    fn new() -> Self {
        let ptt_pressed = Arc::new(AtomicBool::new(false));
        let current_channel = Arc::new(AtomicU8::new(1));
        
        Self {
            state_machine: None,
            ptt_pressed,
            current_channel,
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
    let mut guard = state.lock().unwrap();
    
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
    let guard = state.lock().unwrap();
    
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
    let guard = state.lock().unwrap();
    
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
    let guard = state.lock().unwrap();

    guard.current_channel.store(ch, Ordering::Relaxed);
}

/// JNI: Get active transport type (0=None, 1=Bluetooth, 2=WiFi)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetTransport(
    _env: JNIEnv,
    _class: JClass,
) -> jbyte {
    let state = get_jni_state();
    let guard = state.lock().unwrap();

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
