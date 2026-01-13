// Audio Engine - Cross-platform audio I/O
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tracing::{error, info, warn, debug};
use serde::Serialize;
use thiserror::Error;

// Use CPAL for cross-platform audio on desktop
#[cfg(not(target_os = "android"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Audio configuration constants
pub const SAMPLE_RATE: u32 = 48000; // Opus native rate
pub const CHANNELS: u16 = 1; // Mono
pub const BUFFER_SIZE: usize = 960; // 20ms @ 48kHz

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("No input device")]
    NoInputDevice,
    #[error("No output device")]
    NoOutputDevice,
    #[error("Config error: {0}")]
    ConfigError(String),
    #[error("Stream error: {0}")]
    StreamError(String),
}

/// Audio device info for UI
#[derive(Debug, Clone, Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_input: bool,
    pub is_default: bool,
}

/// Audio engine state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioState {
    Idle,
    Recording,
    Playing,
    RecordingAndPlaying,
}

/// Audio format configuration
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            buffer_size: BUFFER_SIZE,
        }
    }
}

/// Audio engine for recording and playback
pub struct AudioEngine {
    config: AudioConfig,
    state: Arc<Mutex<AudioState>>,
    is_recording: Arc<AtomicBool>,
    is_playing: Arc<AtomicBool>,
    input_volume: Arc<AtomicU32>,
    output_volume: Arc<AtomicU32>,
    selected_input: Option<String>,
    selected_output: Option<String>,
}

impl AudioEngine {
    pub fn new() -> Self {
        Self::with_config(AudioConfig::default())
    }

    pub fn with_config(config: AudioConfig) -> Self {
        info!("Initializing audio engine with config: {}Hz, {} channels", 
              config.sample_rate, config.channels);

        Self {
            config,
            state: Arc::new(Mutex::new(AudioState::Idle)),
            is_recording: Arc::new(AtomicBool::new(false)),
            is_playing: Arc::new(AtomicBool::new(false)),
            input_volume: Arc::new(AtomicU32::new(80)),
            output_volume: Arc::new(AtomicU32::new(80)),
            selected_input: None,
            selected_output: None,
        }
    }

    /// Start audio capture (recording)
    pub fn start_capture(&mut self) -> Result<(), String> {
        info!("Starting audio capture");

        if self.is_recording.load(Ordering::Relaxed) {
            warn!("Already recording");
            return Ok(());
        }

        self.is_recording.store(true, Ordering::Relaxed);
        
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::Playing => AudioState::RecordingAndPlaying,
            _ => AudioState::Recording,
        };

        Ok(())
    }

    /// Stop audio capture
    pub fn stop_capture(&mut self) {
        info!("Stopping audio capture");

        if !self.is_recording.load(Ordering::Relaxed) {
            return;
        }

        self.is_recording.store(false, Ordering::Relaxed);
        
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::RecordingAndPlaying => AudioState::Playing,
            _ => AudioState::Idle,
        };
    }

    /// Start audio playback
    pub fn start_playback(&mut self) -> Result<(), String> {
        info!("Starting audio playback");

        if self.is_playing.load(Ordering::Relaxed) {
            warn!("Already playing");
            return Ok(());
        }

        self.is_playing.store(true, Ordering::Relaxed);
        
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::Recording => AudioState::RecordingAndPlaying,
            _ => AudioState::Playing,
        };

        Ok(())
    }

    /// Stop audio playback
    pub fn stop_playback(&mut self) {
        info!("Stopping audio playback");

        if !self.is_playing.load(Ordering::Relaxed) {
            return;
        }

        self.is_playing.store(false, Ordering::Relaxed);
        
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::RecordingAndPlaying => AudioState::Recording,
            _ => AudioState::Idle,
        };
    }

    /// Check if audio is currently playing
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Check if audio is currently recording
    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::Relaxed)
    }

    /// Get current audio state
    pub fn get_state(&self) -> AudioState {
        *self.state.lock().unwrap()
    }

    /// List available audio devices
    pub fn list_devices(&self) -> Vec<AudioDeviceInfo> {
        let mut devices = Vec::new();

        #[cfg(not(target_os = "android"))]
        {
            let host = cpal::default_host();

            // Default input device
            if let Some(device) = host.default_input_device() {
                devices.push(AudioDeviceInfo {
                    name: device.name().unwrap_or_else(|_| "Default Input".to_string()),
                    is_input: true,
                    is_default: true,
                });
            }

            // Default output device
            if let Some(device) = host.default_output_device() {
                devices.push(AudioDeviceInfo {
                    name: device.name().unwrap_or_else(|_| "Default Output".to_string()),
                    is_input: false,
                    is_default: true,
                });
            }

            // List all input devices
            if let Ok(input_devices) = host.input_devices() {
                for device in input_devices {
                    if let Ok(name) = device.name() {
                        if !devices.iter().any(|d| d.name == name && d.is_input) {
                            devices.push(AudioDeviceInfo {
                                name,
                                is_input: true,
                                is_default: false,
                            });
                        }
                    }
                }
            }

            // List all output devices
            if let Ok(output_devices) = host.output_devices() {
                for device in output_devices {
                    if let Ok(name) = device.name() {
                        if !devices.iter().any(|d| d.name == name && !d.is_input) {
                            devices.push(AudioDeviceInfo {
                                name,
                                is_input: false,
                                is_default: false,
                            });
                        }
                    }
                }
            }
        }

        #[cfg(target_os = "android")]
        {
            devices.push(AudioDeviceInfo {
                name: "Default Microphone".to_string(),
                is_input: true,
                is_default: true,
            });
            devices.push(AudioDeviceInfo {
                name: "Default Speaker".to_string(),
                is_input: false,
                is_default: true,
            });
        }

        devices
    }

    /// Select input device by name
    pub fn select_input_device(&mut self, device_name: &str) -> Result<(), String> {
        info!("Selecting input device: {}", device_name);
        
        #[cfg(not(target_os = "android"))]
        {
            let host = cpal::default_host();
            let found = host.input_devices()
                .map_err(|e| e.to_string())?
                .any(|d| d.name().map(|n| n == device_name).unwrap_or(false));
            
            if !found {
                return Err(format!("Input device not found: {}", device_name));
            }
        }

        self.selected_input = Some(device_name.to_string());
        Ok(())
    }

    /// Select output device by name
    pub fn select_output_device(&mut self, device_name: &str) -> Result<(), String> {
        info!("Selecting output device: {}", device_name);
        
        #[cfg(not(target_os = "android"))]
        {
            let host = cpal::default_host();
            let found = host.output_devices()
                .map_err(|e| e.to_string())?
                .any(|d| d.name().map(|n| n == device_name).unwrap_or(false));
            
            if !found {
                return Err(format!("Output device not found: {}", device_name));
            }
        }

        self.selected_output = Some(device_name.to_string());
        Ok(())
    }

    /// Get input volume (0-100)
    pub fn get_input_volume(&self) -> u32 {
        self.input_volume.load(Ordering::Relaxed)
    }

    /// Get output volume (0-100)
    pub fn get_output_volume(&self) -> u32 {
        self.output_volume.load(Ordering::Relaxed)
    }

    /// Set input volume (0-100)
    pub fn set_input_volume(&self, volume: u32) {
        let clamped = volume.min(100);
        self.input_volume.store(clamped, Ordering::Relaxed);
        info!("Input volume set to {}%", clamped);
    }

    /// Set output volume (0-100)
    pub fn set_output_volume(&self, volume: u32) {
        let clamped = volume.min(100);
        self.output_volume.store(clamped, Ordering::Relaxed);
        info!("Output volume set to {}%", clamped);
    }

    /// Apply volume to audio buffer
    pub fn apply_volume(&self, buffer: &mut [i16], is_input: bool) {
        let volume = if is_input {
            self.get_input_volume()
        } else {
            self.get_output_volume()
        };

        let gain = volume as f32 / 100.0;
        for sample in buffer.iter_mut() {
            let value = (*sample as f32 * gain).clamp(-32768.0, 32767.0);
            *sample = value as i16;
        }
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio processing utilities

/// Apply gain to audio buffer
pub fn apply_gain(buffer: &mut [i16], gain: f32) {
    for sample in buffer.iter_mut() {
        let value = (*sample as f32 * gain).clamp(-32768.0, 32767.0);
        *sample = value as i16;
    }
}

/// Simple noise gate to reduce background noise
pub fn noise_gate(buffer: &mut [i16], threshold: i16) {
    for sample in buffer.iter_mut() {
        if sample.abs() < threshold {
            *sample = 0;
        }
    }
}

/// Convert f32 audio samples to i16
pub fn f32_to_i16(input: &[f32], output: &mut [i16]) {
    for (i, &sample) in input.iter().enumerate() {
        if i >= output.len() {
            break;
        }
        output[i] = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
    }
}

/// Convert i16 audio samples to f32
pub fn i16_to_f32(input: &[i16], output: &mut [f32]) {
    for (i, &sample) in input.iter().enumerate() {
        if i >= output.len() {
            break;
        }
        output[i] = sample as f32 / 32768.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_engine_init() {
        let engine = AudioEngine::new();
        assert_eq!(engine.get_state(), AudioState::Idle);
    }

    #[test]
    fn test_audio_state_transitions() {
        let mut engine = AudioEngine::new();
        assert_eq!(engine.get_state(), AudioState::Idle);

        engine.start_capture().unwrap();
        assert_eq!(engine.get_state(), AudioState::Recording);

        engine.start_playback().unwrap();
        assert_eq!(engine.get_state(), AudioState::RecordingAndPlaying);

        engine.stop_capture();
        assert_eq!(engine.get_state(), AudioState::Playing);

        engine.stop_playback();
        assert_eq!(engine.get_state(), AudioState::Idle);
    }

    #[test]
    fn test_volume_controls() {
        let engine = AudioEngine::new();
        
        assert_eq!(engine.get_input_volume(), 80);
        assert_eq!(engine.get_output_volume(), 80);
        
        engine.set_input_volume(50);
        engine.set_output_volume(75);
        
        assert_eq!(engine.get_input_volume(), 50);
        assert_eq!(engine.get_output_volume(), 75);
        
        // Test clamping
        engine.set_input_volume(150);
        assert_eq!(engine.get_input_volume(), 100);
    }

    #[test]
    fn test_gain_application() {
        let mut buffer = vec![1000i16, -1000, 2000, -2000];
        apply_gain(&mut buffer, 0.5);
        assert_eq!(buffer, vec![500, -500, 1000, -1000]);
    }

    #[test]
    fn test_noise_gate() {
        let mut buffer = vec![100i16, -50, 200, -150, 10, -5];
        noise_gate(&mut buffer, 100);
        assert_eq!(buffer, vec![100, 0, 200, -150, 0, 0]);
    }
}
