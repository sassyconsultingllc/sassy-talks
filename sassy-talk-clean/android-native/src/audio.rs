/// Audio Module - Voice Capture and Playback via JNI
/// 
/// Handles microphone recording (PTT press) and speaker playback (receiving)
/// Uses Android AudioRecord/AudioTrack through JNI bridge

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use log::{error, info, warn};

use crate::jni_bridge::{AndroidAudioRecord, AndroidAudioTrack};

/// Audio configuration constants
pub const SAMPLE_RATE: i32 = 48000;  // 48kHz high quality
pub const CHANNEL_CONFIG_MONO: i32 = 16;  // AudioFormat.CHANNEL_IN_MONO
pub const CHANNEL_CONFIG_OUT_MONO: i32 = 4;  // AudioFormat.CHANNEL_OUT_MONO
pub const AUDIO_FORMAT_PCM_16: i32 = 2;  // AudioFormat.ENCODING_PCM_16BIT
pub const FRAME_SIZE: usize = 960;  // 20ms at 48kHz

/// Audio state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioState {
    Idle,
    Recording,
    Playing,
    Error,
}

/// Audio engine for managing recording and playback
pub struct AudioEngine {
    recorder: Arc<Mutex<Option<AndroidAudioRecord>>>,
    player: Arc<Mutex<Option<AndroidAudioTrack>>>,
    recording: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    state: Arc<Mutex<AudioState>>,
}

impl AudioEngine {
    /// Create new audio engine
    pub fn new() -> Result<Self, String> {
        info!("Initializing audio engine");
        
        Ok(Self {
            recorder: Arc::new(Mutex::new(None)),
            player: Arc::new(Mutex::new(None)),
            recording: Arc::new(AtomicBool::new(false)),
            playing: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(AudioState::Idle)),
        })
    }

    /// Initialize audio recorder
    pub fn init_recorder(&self) -> Result<(), String> {
        info!("Initializing audio recorder");
        
        // Get minimum buffer size
        let buffer_size = AndroidAudioRecord::get_min_buffer_size(
            SAMPLE_RATE,
            CHANNEL_CONFIG_MONO,
            AUDIO_FORMAT_PCM_16
        )?;
        
        info!("Recorder buffer size: {} bytes", buffer_size);
        
        // Create recorder
        let recorder = AndroidAudioRecord::new(
            SAMPLE_RATE,
            CHANNEL_CONFIG_MONO,
            AUDIO_FORMAT_PCM_16,
            buffer_size * 2  // Double buffer for safety
        )?;
        
        *self.recorder.lock().unwrap() = Some(recorder);
        info!("✓ Audio recorder initialized");
        
        Ok(())
    }

    /// Initialize audio player
    pub fn init_player(&self) -> Result<(), String> {
        info!("Initializing audio player");
        
        // Calculate buffer size (same as recorder for consistency)
        let buffer_size = AndroidAudioRecord::get_min_buffer_size(
            SAMPLE_RATE,
            CHANNEL_CONFIG_MONO,
            AUDIO_FORMAT_PCM_16
        )?;
        
        info!("Player buffer size: {} bytes", buffer_size);
        
        // Create player
        let player = AndroidAudioTrack::new(
            SAMPLE_RATE,
            CHANNEL_CONFIG_OUT_MONO,
            AUDIO_FORMAT_PCM_16,
            buffer_size * 2
        )?;
        
        *self.player.lock().unwrap() = Some(player);
        info!("✓ Audio player initialized");
        
        Ok(())
    }

    /// Start recording audio
    pub fn start_recording(&self) -> Result<(), String> {
        info!("Starting audio recording");
        
        // Ensure recorder is initialized
        if self.recorder.lock().unwrap().is_none() {
            self.init_recorder()?;
        }
        
        let recorder = self.recorder.lock().unwrap();
        if let Some(rec) = recorder.as_ref() {
            rec.start_recording()?;
            self.recording.store(true, Ordering::Relaxed);
            *self.state.lock().unwrap() = AudioState::Recording;
            info!("✓ Recording started");
            Ok(())
        } else {
            Err("Recorder not initialized".to_string())
        }
    }

    /// Stop recording audio
    pub fn stop_recording(&self) -> Result<(), String> {
        info!("Stopping audio recording");
        
        self.recording.store(false, Ordering::Relaxed);
        
        let recorder = self.recorder.lock().unwrap();
        if let Some(rec) = recorder.as_ref() {
            rec.stop()?;
            *self.state.lock().unwrap() = AudioState::Idle;
            info!("✓ Recording stopped");
            Ok(())
        } else {
            warn!("Recorder not initialized");
            Ok(())
        }
    }

    /// Read recorded audio data
    pub fn read_audio(&self, buffer: &mut [i16]) -> Result<usize, String> {
        let recorder = self.recorder.lock().unwrap();
        if let Some(rec) = recorder.as_ref() {
            rec.read(buffer)
        } else {
            Err("Recorder not initialized".to_string())
        }
    }

    /// Start playing audio
    pub fn start_playing(&self) -> Result<(), String> {
        info!("Starting audio playback");
        
        // Ensure player is initialized
        if self.player.lock().unwrap().is_none() {
            self.init_player()?;
        }
        
        let player = self.player.lock().unwrap();
        if let Some(play) = player.as_ref() {
            play.play()?;
            self.playing.store(true, Ordering::Relaxed);
            *self.state.lock().unwrap() = AudioState::Playing;
            info!("✓ Playback started");
            Ok(())
        } else {
            Err("Player not initialized".to_string())
        }
    }

    /// Stop playing audio
    pub fn stop_playing(&self) -> Result<(), String> {
        info!("Stopping audio playback");
        
        self.playing.store(false, Ordering::Relaxed);
        
        let player = self.player.lock().unwrap();
        if let Some(play) = player.as_ref() {
            play.stop()?;
            *self.state.lock().unwrap() = AudioState::Idle;
            info!("✓ Playback stopped");
            Ok(())
        } else {
            warn!("Player not initialized");
            Ok(())
        }
    }

    /// Write audio data for playback
    pub fn write_audio(&self, buffer: &[i16]) -> Result<usize, String> {
        let player = self.player.lock().unwrap();
        if let Some(play) = player.as_ref() {
            play.write(buffer)
        } else {
            Err("Player not initialized".to_string())
        }
    }

    /// Check if currently recording
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Check if currently playing
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Get current audio state
    pub fn get_state(&self) -> AudioState {
        *self.state.lock().unwrap()
    }

    /// Release audio resources
    pub fn release(&self) -> Result<(), String> {
        info!("Releasing audio resources");
        
        // Stop recording if active
        if self.is_recording() {
            self.stop_recording()?;
        }
        
        // Stop playing if active
        if self.is_playing() {
            self.stop_playing()?;
        }
        
        // Release recorder
        if let Some(rec) = self.recorder.lock().unwrap().as_ref() {
            rec.release()?;
        }
        
        // Release player
        if let Some(play) = self.player.lock().unwrap().as_ref() {
            play.release()?;
        }
        
        *self.recorder.lock().unwrap() = None;
        *self.player.lock().unwrap() = None;
        *self.state.lock().unwrap() = AudioState::Idle;
        
        info!("✓ Audio resources released");
        Ok(())
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

/// Audio frame for transmission
pub struct AudioFrame {
    pub samples: Vec<i16>,
    pub timestamp: u64,
}

impl AudioFrame {
    pub fn new(size: usize) -> Self {
        Self {
            samples: vec![0; size],
            timestamp: 0,
        }
    }

    /// Convert samples to bytes for Bluetooth transmission
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.samples.len() * 2);
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    /// Convert bytes from Bluetooth to samples
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() % 2 != 0 {
            return Err("Invalid audio data: odd number of bytes".to_string());
        }
        
        let mut samples = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        
        Ok(Self {
            samples,
            timestamp: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_frame_conversion() {
        let frame = AudioFrame {
            samples: vec![100, -200, 300, -400],
            timestamp: 0,
        };
        
        let bytes = frame.to_bytes();
        assert_eq!(bytes.len(), 8);
        
        let recovered = AudioFrame::from_bytes(&bytes).unwrap();
        assert_eq!(recovered.samples, frame.samples);
    }

    #[test]
    fn test_audio_engine_creation() {
        // Note: Will fail without Android environment
        let _ = AudioEngine::new();
    }
}
