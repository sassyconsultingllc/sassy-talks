/// Audio Engine - Cross-Platform Audio I/O
/// 
/// Uses CPAL for portability across Windows, Mac, Linux
/// Handles microphone input and speaker output with ring buffers

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, Stream};
use ringbuf::HeapRb;
use ringbuf::traits::{Producer, Consumer, Split};
use ringbuf::wrap::caching::{CachingProd, CachingCons};
use send_wrapper::SendWrapper;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info};
use thiserror::Error;

/// Type aliases for ring buffer producer/consumer (ringbuf 0.4 API)
type HeapProducer<T> = CachingProd<Arc<HeapRb<T>>>;
type HeapConsumer<T> = CachingCons<Arc<HeapRb<T>>>;

/// Wrapper for Stream to make it Send+Sync (cpal::Stream is !Send on Windows)
/// SAFETY: Stream is only accessed from the thread that created it
type SendableStream = SendWrapper<Stream>;

/// Audio sample rate (Opus requires 48kHz)
pub const SAMPLE_RATE: u32 = 48000;

/// Frame size for 20ms at 48kHz
pub const FRAME_SIZE: usize = 960;

/// Ring buffer size (1 second of audio)
const BUFFER_SIZE: usize = SAMPLE_RATE as usize;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("No audio device found")]
    NoDevice,
    
    #[error("Failed to get device config: {0}")]
    ConfigError(String),
    
    #[error("Failed to build stream: {0}")]
    StreamError(String),
    
    #[error("Device not supported: {0}")]
    UnsupportedDevice(String),
    
    #[error("Audio engine not initialized")]
    NotInitialized,
}

/// Audio device information
#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub device_type: String, // "input" or "output"
}

/// Audio engine state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioState {
    Idle,
    Recording,
    Playing,
}

/// Audio engine for cross-platform audio I/O
pub struct AudioEngine {
    // CPAL host
    host: Host,
    
    // Devices
    input_device: Arc<Mutex<Option<Device>>>,
    output_device: Arc<Mutex<Option<Device>>>,
    
    // Streams (wrapped for Send+Sync)
    input_stream: Arc<Mutex<Option<SendableStream>>>,
    output_stream: Arc<Mutex<Option<SendableStream>>>,
    
    // Ring buffers for audio data
    input_producer: Arc<Mutex<Option<HeapProducer<i16>>>>,
    input_consumer: Arc<Mutex<Option<HeapConsumer<i16>>>>,
    output_producer: Arc<Mutex<Option<HeapProducer<i16>>>>,
    output_consumer: Arc<Mutex<Option<HeapConsumer<i16>>>>,
    
    // State
    state: Arc<Mutex<AudioState>>,
    recording: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    
    // Volume
    input_volume: Arc<Mutex<f32>>,
    output_volume: Arc<Mutex<f32>>,
}

impl AudioEngine {
    /// Create new audio engine
    pub fn new() -> Result<Self, AudioError> {
        info!("Initializing audio engine");
        
        let host = cpal::default_host();
        
        Ok(Self {
            host,
            input_device: Arc::new(Mutex::new(None)),
            output_device: Arc::new(Mutex::new(None)),
            input_stream: Arc::new(Mutex::new(None)),
            output_stream: Arc::new(Mutex::new(None)),
            input_producer: Arc::new(Mutex::new(None)),
            input_consumer: Arc::new(Mutex::new(None)),
            output_producer: Arc::new(Mutex::new(None)),
            output_consumer: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(AudioState::Idle)),
            recording: Arc::new(AtomicBool::new(false)),
            playing: Arc::new(AtomicBool::new(false)),
            input_volume: Arc::new(Mutex::new(1.0)),
            output_volume: Arc::new(Mutex::new(1.0)),
        })
    }

    /// Get list of input devices
    pub fn get_input_devices(&self) -> Vec<AudioDeviceInfo> {
        let mut devices = Vec::new();
        
        if let Ok(input_devices) = self.host.input_devices() {
            let default_device = self.host.default_input_device();
            let default_name = default_device.as_ref()
                .and_then(|d| d.name().ok());
            
            for device in input_devices {
                if let Ok(name) = device.name() {
                    devices.push(AudioDeviceInfo {
                        name: name.clone(),
                        is_default: Some(name.clone()) == default_name,
                        device_type: "input".to_string(),
                    });
                }
            }
        }
        
        devices
    }

    /// Get list of output devices
    pub fn get_output_devices(&self) -> Vec<AudioDeviceInfo> {
        let mut devices = Vec::new();
        
        if let Ok(output_devices) = self.host.output_devices() {
            let default_device = self.host.default_output_device();
            let default_name = default_device.as_ref()
                .and_then(|d| d.name().ok());
            
            for device in output_devices {
                if let Ok(name) = device.name() {
                    devices.push(AudioDeviceInfo {
                        name: name.clone(),
                        is_default: Some(name.clone()) == default_name,
                        device_type: "output".to_string(),
                    });
                }
            }
        }
        
        devices
    }

    /// Set input device by name
    pub fn set_input_device(&mut self, device_name: &str) -> Result<(), AudioError> {
        info!("Setting input device: {}", device_name);
        
        let device = if device_name == "default" {
            self.host.default_input_device()
        } else {
            self.host.input_devices()
                .ok()
                .and_then(|devices| {
                    devices.filter(|d| d.name().ok().as_deref() == Some(device_name))
                        .next()
                })
        }.ok_or(AudioError::NoDevice)?;
        
        *self.input_device.lock().unwrap() = Some(device);
        Ok(())
    }

    /// Set output device by name
    pub fn set_output_device(&mut self, device_name: &str) -> Result<(), AudioError> {
        info!("Setting output device: {}", device_name);
        
        let device = if device_name == "default" {
            self.host.default_output_device()
        } else {
            self.host.output_devices()
                .ok()
                .and_then(|devices| {
                    devices.filter(|d| d.name().ok().as_deref() == Some(device_name))
                        .next()
                })
        }.ok_or(AudioError::NoDevice)?;
        
        *self.output_device.lock().unwrap() = Some(device);
        Ok(())
    }

    /// Initialize input stream
    fn init_input_stream(&self) -> Result<(), AudioError> {
        let device = self.input_device.lock().unwrap()
            .clone()
            .or_else(|| self.host.default_input_device())
            .ok_or(AudioError::NoDevice)?;
        
        let config = device.default_input_config()
            .map_err(|e| AudioError::ConfigError(e.to_string()))?;
        
        info!("Input device: {}", device.name().unwrap_or_default());
        info!("Input config: {:?}", config);
        
        // Create ring buffer
        let rb = HeapRb::<i16>::new(BUFFER_SIZE);
        let (producer, consumer) = rb.split();
        
        *self.input_producer.lock().unwrap() = Some(producer);
        *self.input_consumer.lock().unwrap() = Some(consumer);
        
        let producer = Arc::clone(&self.input_producer);
        let volume = Arc::clone(&self.input_volume);
        let recording = Arc::clone(&self.recording);
        
        let stream = device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !recording.load(Ordering::Relaxed) {
                    return;
                }
                
                let vol = *volume.lock().unwrap();
                let mut prod = producer.lock().unwrap();
                
                if let Some(ref mut p) = *prod {
                    for &sample in data {
                        let scaled = (sample * vol * 32767.0) as i16;
                        let _ = p.try_push(scaled);
                    }
                }
            },
            |err| error!("Input stream error: {}", err),
            None,
        ).map_err(|e| AudioError::StreamError(e.to_string()))?;
        
        stream.play().map_err(|e| AudioError::StreamError(e.to_string()))?;
        *self.input_stream.lock().unwrap() = Some(SendWrapper::new(stream));
        
        Ok(())
    }

    /// Initialize output stream
    fn init_output_stream(&self) -> Result<(), AudioError> {
        let device = self.output_device.lock().unwrap()
            .clone()
            .or_else(|| self.host.default_output_device())
            .ok_or(AudioError::NoDevice)?;
        
        let config = device.default_output_config()
            .map_err(|e| AudioError::ConfigError(e.to_string()))?;
        
        info!("Output device: {}", device.name().unwrap_or_default());
        info!("Output config: {:?}", config);
        
        // Create ring buffer
        let rb = HeapRb::<i16>::new(BUFFER_SIZE);
        let (producer, consumer) = rb.split();
        
        *self.output_producer.lock().unwrap() = Some(producer);
        *self.output_consumer.lock().unwrap() = Some(consumer);
        
        let consumer = Arc::clone(&self.output_consumer);
        let volume = Arc::clone(&self.output_volume);
        let playing = Arc::clone(&self.playing);
        
        let stream = device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                if !playing.load(Ordering::Relaxed) {
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                    return;
                }
                
                let vol = *volume.lock().unwrap();
                let mut cons = consumer.lock().unwrap();
                
                if let Some(ref mut c) = *cons {
                    for sample in data.iter_mut() {
                        if let Some(s) = c.try_pop() {
                            *sample = (s as f32 / 32767.0) * vol;
                        } else {
                            *sample = 0.0;
                        }
                    }
                }
            },
            |err| error!("Output stream error: {}", err),
            None,
        ).map_err(|e| AudioError::StreamError(e.to_string()))?;
        
        stream.play().map_err(|e| AudioError::StreamError(e.to_string()))?;
        *self.output_stream.lock().unwrap() = Some(SendWrapper::new(stream));
        
        Ok(())
    }

    /// Start recording audio
    pub fn start_recording(&self) -> Result<(), AudioError> {
        info!("Starting audio recording");
        
        if self.input_stream.lock().unwrap().is_none() {
            self.init_input_stream()?;
        }
        
        self.recording.store(true, Ordering::Relaxed);
        *self.state.lock().unwrap() = AudioState::Recording;
        
        Ok(())
    }

    /// Stop recording audio
    pub fn stop_recording(&self) -> Result<(), AudioError> {
        info!("Stopping audio recording");
        
        self.recording.store(false, Ordering::Relaxed);
        *self.state.lock().unwrap() = AudioState::Idle;
        
        Ok(())
    }

    /// Start playing audio
    pub fn start_playing(&self) -> Result<(), AudioError> {
        info!("Starting audio playback");
        
        if self.output_stream.lock().unwrap().is_none() {
            self.init_output_stream()?;
        }
        
        self.playing.store(true, Ordering::Relaxed);
        *self.state.lock().unwrap() = AudioState::Playing;
        
        Ok(())
    }

    /// Stop playing audio
    pub fn stop_playing(&self) -> Result<(), AudioError> {
        info!("Stopping audio playback");
        
        self.playing.store(false, Ordering::Relaxed);
        *self.state.lock().unwrap() = AudioState::Idle;
        
        Ok(())
    }

    /// Read recorded audio samples
    pub fn read_samples(&self, buffer: &mut [i16]) -> usize {
        let mut consumer = self.input_consumer.lock().unwrap();
        
        if let Some(ref mut c) = *consumer {
            let mut count = 0;
            for sample in buffer.iter_mut() {
                if let Some(s) = c.try_pop() {
                    *sample = s;
                    count += 1;
                } else {
                    break;
                }
            }
            count
        } else {
            0
        }
    }

    /// Write audio samples for playback
    pub fn write_samples(&self, buffer: &[i16]) -> usize {
        let mut producer = self.output_producer.lock().unwrap();
        
        if let Some(ref mut p) = *producer {
            let mut count = 0;
            for &sample in buffer {
                if p.try_push(sample).is_ok() {
                    count += 1;
                } else {
                    break;
                }
            }
            count
        } else {
            0
        }
    }

    /// Set input volume (0.0 - 2.0)
    pub fn set_input_volume(&self, volume: f32) {
        *self.input_volume.lock().unwrap() = volume.clamp(0.0, 2.0);
    }

    /// Set output volume (0.0 - 2.0)
    pub fn set_output_volume(&self, volume: f32) {
        *self.output_volume.lock().unwrap() = volume.clamp(0.0, 2.0);
    }

    /// Get input volume
    pub fn get_input_volume(&self) -> f32 {
        *self.input_volume.lock().unwrap()
    }

    /// Get output volume
    pub fn get_output_volume(&self) -> f32 {
        *self.output_volume.lock().unwrap()
    }

    /// Get current audio state
    pub fn get_state(&self) -> AudioState {
        *self.state.lock().unwrap()
    }

    /// Check if currently recording
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Check if currently playing
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Release audio resources
    pub fn shutdown(&self) -> Result<(), AudioError> {
        info!("Shutting down audio engine");
        
        self.stop_recording()?;
        self.stop_playing()?;
        
        *self.input_stream.lock().unwrap() = None;
        *self.output_stream.lock().unwrap() = None;
        
        Ok(())
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_engine_creation() {
        let engine = AudioEngine::new();
        assert!(engine.is_ok());
    }

    #[test]
    fn test_device_enumeration() {
        let engine = AudioEngine::new().unwrap();
        let inputs = engine.get_input_devices();
        let outputs = engine.get_output_devices();
        
        // Should have at least one device on most systems
        println!("Input devices: {}", inputs.len());
        println!("Output devices: {}", outputs.len());
    }
}
