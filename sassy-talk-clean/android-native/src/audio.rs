/// Audio Module - Voice Capture and Playback via JNI
/// 
/// Handles microphone recording (PTT press) and speaker playback (receiving)
/// Uses Android AudioRecord/AudioTrack through JNI bridge

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use log::{info, warn};

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

/// Audio engine for managing recording and playback.
///
/// `recorder`/`player` are stored as `Arc<...>` inside the mutex so callers can
/// clone the handle out, release the mutex, and then make the (potentially
/// blocking) JNI call without serializing every audio op behind one lock.
/// Holding the mutex across `read()`/`start_recording()`/`stop()` would
/// deadlock the audio TX thread against any control-plane stop.
pub struct AudioEngine {
    recorder: Arc<Mutex<Option<Arc<AndroidAudioRecord>>>>,
    player: Arc<Mutex<Option<Arc<AndroidAudioTrack>>>>,
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

        // Get minimum buffer size. AudioRecord.getMinBufferSize returns
        // ERROR_BAD_VALUE (-2) or ERROR (-1) on failure; treating those as a
        // valid size and feeding `size * 2` into AudioRecord::new produces a
        // negative request that the JNI layer would either reject or — worse —
        // sign-extend into an enormous allocation.
        let buffer_size = match AndroidAudioRecord::get_min_buffer_size(
            SAMPLE_RATE,
            CHANNEL_CONFIG_MONO,
            AUDIO_FORMAT_PCM_16
        ) {
            Ok(size) if size > 0 => size,
            Ok(bad) => {
                *self.state.lock().unwrap() = AudioState::Error;
                return Err(format!(
                    "AudioRecord.getMinBufferSize returned non-positive value {} \
                     (sample_rate={}, channel={}, format={}) — unsupported config",
                    bad, SAMPLE_RATE, CHANNEL_CONFIG_MONO, AUDIO_FORMAT_PCM_16
                ));
            }
            Err(e) => {
                *self.state.lock().unwrap() = AudioState::Error;
                return Err(e);
            }
        };

        // Guard the doubling against i32 overflow on absurd device returns.
        let alloc_size = buffer_size.checked_mul(2).ok_or_else(|| {
            *self.state.lock().unwrap() = AudioState::Error;
            format!("Recorder buffer size {} would overflow i32 when doubled", buffer_size)
        })?;

        info!("Recorder buffer size: {} bytes (doubled to {})", buffer_size, alloc_size);

        // Create recorder
        let recorder = match AndroidAudioRecord::new(
            SAMPLE_RATE,
            CHANNEL_CONFIG_MONO,
            AUDIO_FORMAT_PCM_16,
            alloc_size
        ) {
            Ok(r) => r,
            Err(e) => {
                *self.state.lock().unwrap() = AudioState::Error;
                return Err(e);
            }
        };

        *self.recorder.lock().unwrap() = Some(Arc::new(recorder));
        info!("✓ Audio recorder initialized");

        Ok(())
    }

    /// Initialize audio player
    pub fn init_player(&self) -> Result<(), String> {
        info!("Initializing audio player");

        // Query AudioTrack's own min buffer size — AudioRecord's is not suitable
        // for playback (can under-allocate and cause audible glitches / underruns).
        // Same negative-return guard as init_recorder: AudioTrack returns
        // ERROR(-1) / ERROR_BAD_VALUE(-2) for unsupported configs.
        let buffer_size = match AndroidAudioTrack::get_min_buffer_size(
            SAMPLE_RATE,
            CHANNEL_CONFIG_OUT_MONO,
            AUDIO_FORMAT_PCM_16
        ) {
            Ok(size) if size > 0 => size,
            Ok(bad) => {
                *self.state.lock().unwrap() = AudioState::Error;
                return Err(format!(
                    "AudioTrack.getMinBufferSize returned non-positive value {} \
                     (sample_rate={}, channel={}, format={}) — unsupported config",
                    bad, SAMPLE_RATE, CHANNEL_CONFIG_OUT_MONO, AUDIO_FORMAT_PCM_16
                ));
            }
            Err(e) => {
                *self.state.lock().unwrap() = AudioState::Error;
                return Err(e);
            }
        };

        let alloc_size = buffer_size.checked_mul(2).ok_or_else(|| {
            *self.state.lock().unwrap() = AudioState::Error;
            format!("Player buffer size {} would overflow i32 when doubled", buffer_size)
        })?;

        info!("Player buffer size: {} bytes (doubled to {})", buffer_size, alloc_size);

        // Create player
        let player = match AndroidAudioTrack::new(
            SAMPLE_RATE,
            CHANNEL_CONFIG_OUT_MONO,
            AUDIO_FORMAT_PCM_16,
            alloc_size
        ) {
            Ok(p) => p,
            Err(e) => {
                *self.state.lock().unwrap() = AudioState::Error;
                return Err(e);
            }
        };

        *self.player.lock().unwrap() = Some(Arc::new(player));
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

        // Clone the Arc out of the lock so the (potentially blocking) JNI call
        // does not serialize against read_audio() / stop_recording().
        let rec = match self.recorder.lock().unwrap().as_ref() {
            Some(r) => Arc::clone(r),
            None => return Err("Recorder not initialized".to_string()),
        };

        rec.start_recording()?;
        self.recording.store(true, Ordering::Relaxed);
        *self.state.lock().unwrap() = AudioState::Recording;
        info!("✓ Recording started");
        Ok(())
    }

    /// Stop recording audio
    pub fn stop_recording(&self) -> Result<(), String> {
        info!("Stopping audio recording");

        self.recording.store(false, Ordering::Relaxed);

        let rec = self.recorder.lock().unwrap().as_ref().map(Arc::clone);
        match rec {
            Some(rec) => {
                rec.stop()?;
                *self.state.lock().unwrap() = AudioState::Idle;
                info!("✓ Recording stopped");
                Ok(())
            }
            None => {
                warn!("Recorder not initialized");
                Ok(())
            }
        }
    }

    /// Read recorded audio data
    pub fn read_audio(&self, buffer: &mut [i16]) -> Result<usize, String> {
        // Clone-out-then-call: read() blocks waiting for samples; holding the
        // mutex across that call would block stop_recording() indefinitely.
        let rec = match self.recorder.lock().unwrap().as_ref() {
            Some(r) => Arc::clone(r),
            None => return Err("Recorder not initialized".to_string()),
        };
        rec.read(buffer)
    }

    /// Start playing audio
    pub fn start_playing(&self) -> Result<(), String> {
        info!("Starting audio playback");

        // Ensure player is initialized
        if self.player.lock().unwrap().is_none() {
            self.init_player()?;
        }

        let play = match self.player.lock().unwrap().as_ref() {
            Some(p) => Arc::clone(p),
            None => return Err("Player not initialized".to_string()),
        };

        play.play()?;
        self.playing.store(true, Ordering::Relaxed);
        *self.state.lock().unwrap() = AudioState::Playing;
        info!("✓ Playback started");
        Ok(())
    }

    /// Stop playing audio
    pub fn stop_playing(&self) -> Result<(), String> {
        info!("Stopping audio playback");

        self.playing.store(false, Ordering::Relaxed);

        let play = self.player.lock().unwrap().as_ref().map(Arc::clone);
        match play {
            Some(play) => {
                play.stop()?;
                *self.state.lock().unwrap() = AudioState::Idle;
                info!("✓ Playback stopped");
                Ok(())
            }
            None => {
                warn!("Player not initialized");
                Ok(())
            }
        }
    }

    /// Write audio data for playback
    pub fn write_audio(&self, buffer: &[i16]) -> Result<usize, String> {
        let play = match self.player.lock().unwrap().as_ref() {
            Some(p) => Arc::clone(p),
            None => return Err("Player not initialized".to_string()),
        };
        play.write(buffer)
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

        // Take the handles out of the mutex first so the JNI release() calls
        // run without the lock held — same deadlock concern as the start/stop
        // path above.
        let rec = self.recorder.lock().unwrap().take();
        let play = self.player.lock().unwrap().take();

        if let Some(rec) = rec {
            rec.release()?;
        }
        if let Some(play) = play {
            play.release()?;
        }

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
    /// Format: [timestamp:8][samples:N*2]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + self.samples.len() * 2);
        bytes.extend_from_slice(&self.timestamp.to_le_bytes());
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    /// Convert bytes from Bluetooth to samples
    /// Format: [timestamp:8][samples:N*2]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 8 {
            return Err("Invalid audio data: too short for header".to_string());
        }

        let timestamp = u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5], bytes[6], bytes[7],
        ]);

        let audio_bytes = &bytes[8..];
        if audio_bytes.len() % 2 != 0 {
            return Err("Invalid audio data: odd number of sample bytes".to_string());
        }

        let mut samples = Vec::with_capacity(audio_bytes.len() / 2);
        for chunk in audio_bytes.chunks_exact(2) {
            samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }

        Ok(Self {
            samples,
            timestamp,
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
            timestamp: 12345678,
        };

        let bytes = frame.to_bytes();
        assert_eq!(bytes.len(), 8 + 8); // 8 byte timestamp + 4 samples * 2 bytes

        let recovered = AudioFrame::from_bytes(&bytes).unwrap();
        assert_eq!(recovered.samples, frame.samples);
        assert_eq!(recovered.timestamp, 12345678);
    }

    #[test]
    fn test_audio_engine_creation() {
        // Note: Will fail without Android environment
        let _ = AudioEngine::new();
    }
}
