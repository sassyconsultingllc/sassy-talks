/// On-device speech-to-text using whisper.cpp (tiny model).
///
/// The WhisperEngine wraps the C library and provides a simple
/// `transcribe(pcm_16k_f32) -> String` interface. Audio must be
/// 16 kHz mono float; use `downsample_48k_to_16k` to convert from
/// the codec's native 48 kHz i16 format.

use crate::whisper_ffi as ffi;
use log::{error, info};
use std::ffi::{CStr, CString};
use std::sync::Mutex;

/// Maximum result buffer size for a single transcription (8 KB).
const MAX_RESULT_BYTES: usize = 8192;

/// CPU threads for inference. 2 is conservative for weak phones;
/// high-end devices could use 4.
const INFERENCE_THREADS: i32 = 2;

/// Whisper engine singleton (loaded once, used for all transcriptions).
pub struct WhisperEngine {
    ctx: *mut std::os::raw::c_void,
}

// Safety: whisper_context is only accessed behind a Mutex.
unsafe impl Send for WhisperEngine {}

impl WhisperEngine {
    /// Load a whisper model from the given file path.
    pub fn load(model_path: &str) -> Result<Self, String> {
        let c_path = CString::new(model_path)
            .map_err(|_| "Invalid model path (contains NUL)".to_string())?;

        let ctx = unsafe { ffi::whisper_wrapper_init(c_path.as_ptr()) };
        if ctx.is_null() {
            return Err(format!("Failed to load whisper model: {}", model_path));
        }

        info!("Whisper model loaded: {}", model_path);
        Ok(Self { ctx })
    }

    /// Run inference on 16 kHz mono f32 PCM samples.
    ///
    /// Returns the transcribed text (may be empty for silence/noise).
    pub fn transcribe(&self, samples_16k: &[f32]) -> String {
        if samples_16k.is_empty() || self.ctx.is_null() {
            return String::new();
        }

        let lang = CString::new("en").unwrap();
        let mut result_buf = vec![0u8; MAX_RESULT_BYTES];

        let n_segments = unsafe {
            ffi::whisper_wrapper_transcribe(
                self.ctx,
                samples_16k.as_ptr(),
                samples_16k.len() as i32,
                lang.as_ptr(),
                INFERENCE_THREADS,
                result_buf.as_mut_ptr() as *mut std::os::raw::c_char,
                MAX_RESULT_BYTES as i32,
            )
        };

        if n_segments < 0 {
            error!("Whisper inference failed");
            return String::new();
        }

        // Convert C string result to Rust String
        let c_str = unsafe {
            CStr::from_ptr(result_buf.as_ptr() as *const std::os::raw::c_char)
        };
        let text = c_str.to_string_lossy().trim().to_string();

        if !text.is_empty() {
            info!("Whisper: {} segments, text: {}", n_segments, &text[..text.len().min(80)]);
        }

        text
    }
}

impl Drop for WhisperEngine {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { ffi::whisper_wrapper_free(self.ctx) };
            info!("Whisper model unloaded");
        }
    }
}

/// Downsample 48 kHz i16 PCM to 16 kHz f32 (simple 3:1 decimation).
///
/// Whisper expects 16 kHz mono float samples in [-1.0, 1.0].
/// A 3:1 ratio is exact (48000/16000 = 3). For better quality a
/// low-pass filter could be added, but for voice the simple approach
/// is sufficient and very fast on weak CPUs.
pub fn downsample_48k_to_16k(pcm_48k: &[i16]) -> Vec<f32> {
    let out_len = pcm_48k.len() / 3;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        // Average 3 consecutive samples then normalize i16 → f32
        let s0 = pcm_48k[i * 3] as f32;
        let s1 = pcm_48k[i * 3 + 1] as f32;
        let s2 = pcm_48k[i * 3 + 2] as f32;
        out.push((s0 + s1 + s2) / (3.0 * 32768.0));
    }
    out
}

/// Global whisper engine (lazy-init, behind Mutex).
static WHISPER_ENGINE: once_cell::sync::Lazy<Mutex<Option<WhisperEngine>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// Initialize the global whisper engine with the given model path.
/// Returns true on success.
pub fn init_global(model_path: &str) -> bool {
    match WhisperEngine::load(model_path) {
        Ok(engine) => {
            let mut guard = WHISPER_ENGINE.lock().unwrap();
            *guard = Some(engine);
            true
        }
        Err(e) => {
            error!("Whisper init failed: {}", e);
            false
        }
    }
}

/// Transcribe 16 kHz f32 PCM using the global engine.
/// Returns empty string if engine not loaded or inference fails.
pub fn transcribe_global(samples_16k: &[f32]) -> String {
    let guard = WHISPER_ENGINE.lock().unwrap();
    match guard.as_ref() {
        Some(engine) => engine.transcribe(samples_16k),
        None => {
            error!("Whisper engine not initialized");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downsample_48k_to_16k() {
        // 960 samples at 48kHz (20ms frame) → 320 samples at 16kHz
        let input: Vec<i16> = (0..960).map(|i| (i % 100) as i16).collect();
        let output = downsample_48k_to_16k(&input);
        assert_eq!(output.len(), 320);
        // Verify normalization range
        for &s in &output {
            assert!(s >= -1.0 && s <= 1.0);
        }
    }

    #[test]
    fn test_downsample_silence() {
        let silence = vec![0i16; 960];
        let output = downsample_48k_to_16k(&silence);
        assert_eq!(output.len(), 320);
        assert!(output.iter().all(|&s| s == 0.0));
    }
}
