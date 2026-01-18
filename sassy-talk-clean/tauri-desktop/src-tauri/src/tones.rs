/// Tones Module - Audio Feedback Tones
/// 
/// Connection success: 3-tone ascending (XP login style)
/// Message delivered: 2-tone low→high (450→480 Hz)
/// Failed/Error: 2-tone mono (330, 330 Hz)
/// 
/// Copyright 2025 Sassy Consulting LLC. All rights reserved.

use std::f32::consts::PI;
use std::sync::Arc;
use tokio::sync::Mutex;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

/// Sample rate for tone generation
const SAMPLE_RATE: u32 = 48000;

/// Tone types
#[derive(Debug, Clone, Copy)]
pub enum ToneType {
    /// 3-tone ascending chime (connection success)
    ConnectionSuccess,
    /// 2-tone low→high (message delivered)
    MessageDelivered,
    /// 2-tone mono (error/failed)
    Failed,
    /// Single beep for roger (end of transmission)
    RogerBeep,
}

/// Tone specification
#[derive(Debug, Clone)]
struct ToneSpec {
    frequency: f32,
    duration_ms: u32,
    amplitude: f32,
}

/// Generate samples for a single tone with envelope
fn generate_tone(spec: &ToneSpec, sample_rate: u32) -> Vec<f32> {
    let num_samples = (sample_rate as f32 * spec.duration_ms as f32 / 1000.0) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    // Attack/decay envelope parameters
    let attack_samples = (sample_rate as f32 * 0.01) as usize; // 10ms attack
    let decay_samples = (sample_rate as f32 * 0.05) as usize;  // 50ms decay
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        
        // Generate sine wave
        let sample = (2.0 * PI * spec.frequency * t).sin();
        
        // Apply envelope
        let envelope = if i < attack_samples {
            // Attack phase - fade in
            i as f32 / attack_samples as f32
        } else if i > num_samples - decay_samples {
            // Decay phase - fade out
            (num_samples - i) as f32 / decay_samples as f32
        } else {
            1.0
        };
        
        samples.push(sample * spec.amplitude * envelope);
    }
    
    samples
}

/// Generate silence gap
fn generate_silence(duration_ms: u32, sample_rate: u32) -> Vec<f32> {
    let num_samples = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
    vec![0.0; num_samples]
}

/// Build the complete tone sequence for a given type
pub fn build_tone_sequence(tone_type: ToneType) -> Vec<f32> {
    match tone_type {
        ToneType::ConnectionSuccess => {
            // Windows XP login style: 3-tone ascending chime
            // E5 (659Hz) → G5 (784Hz) → C6 (1047Hz)
            // With slight overlap/reverb feel
            let mut sequence = Vec::new();
            
            // First tone: E5
            sequence.extend(generate_tone(&ToneSpec {
                frequency: 659.0,
                duration_ms: 150,
                amplitude: 0.4,
            }, SAMPLE_RATE));
            
            // Short gap
            sequence.extend(generate_silence(30, SAMPLE_RATE));
            
            // Second tone: G5
            sequence.extend(generate_tone(&ToneSpec {
                frequency: 784.0,
                duration_ms: 150,
                amplitude: 0.45,
            }, SAMPLE_RATE));
            
            // Short gap
            sequence.extend(generate_silence(30, SAMPLE_RATE));
            
            // Third tone: C6 (higher, slightly longer for emphasis)
            sequence.extend(generate_tone(&ToneSpec {
                frequency: 1047.0,
                duration_ms: 250,
                amplitude: 0.5,
            }, SAMPLE_RATE));
            
            sequence
        }
        
        ToneType::MessageDelivered => {
            // 2-tone low→high: 450Hz → 480Hz
            let mut sequence = Vec::new();
            
            // First tone: 450Hz
            sequence.extend(generate_tone(&ToneSpec {
                frequency: 450.0,
                duration_ms: 100,
                amplitude: 0.35,
            }, SAMPLE_RATE));
            
            // Tiny gap
            sequence.extend(generate_silence(20, SAMPLE_RATE));
            
            // Second tone: 480Hz (slightly higher)
            sequence.extend(generate_tone(&ToneSpec {
                frequency: 480.0,
                duration_ms: 120,
                amplitude: 0.4,
            }, SAMPLE_RATE));
            
            sequence
        }
        
        ToneType::Failed => {
            // 2-tone mono: 330Hz, 330Hz (Windows error style)
            let mut sequence = Vec::new();
            
            // First tone: 330Hz
            sequence.extend(generate_tone(&ToneSpec {
                frequency: 330.0,
                duration_ms: 150,
                amplitude: 0.4,
            }, SAMPLE_RATE));
            
            // Gap between beeps
            sequence.extend(generate_silence(80, SAMPLE_RATE));
            
            // Second tone: 330Hz (same pitch)
            sequence.extend(generate_tone(&ToneSpec {
                frequency: 330.0,
                duration_ms: 150,
                amplitude: 0.4,
            }, SAMPLE_RATE));
            
            sequence
        }
        
        ToneType::RogerBeep => {
            // Classic roger beep: short high tone
            generate_tone(&ToneSpec {
                frequency: 1200.0,
                duration_ms: 80,
                amplitude: 0.3,
            }, SAMPLE_RATE)
        }
    }
}


/// Tone player that outputs audio through CPAL
pub struct TonePlayer {
    samples: Arc<Mutex<Option<Vec<f32>>>>,
    sample_index: Arc<Mutex<usize>>,
}

impl TonePlayer {
    /// Create a new tone player
    pub fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(None)),
            sample_index: Arc::new(Mutex::new(0)),
        }
    }
    
    /// Play a tone asynchronously
    pub async fn play(&self, tone_type: ToneType) -> Result<(), ToneError> {
        let samples = build_tone_sequence(tone_type);
        
        // Store samples for playback
        {
            let mut stored = self.samples.lock().await;
            *stored = Some(samples);
        }
        {
            let mut idx = self.sample_index.lock().await;
            *idx = 0;
        }
        
        // Get default output device
        let host = cpal::default_host();
        let device = host.default_output_device()
            .ok_or(ToneError::NoOutputDevice)?;
        
        let config = StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };
        
        let samples_clone = Arc::clone(&self.samples);
        let index_clone = Arc::clone(&self.sample_index);
        
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let tx_clone = Arc::clone(&tx);
        
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let samples_guard = samples_clone.blocking_lock();
                let mut index_guard = index_clone.blocking_lock();
                
                if let Some(ref samples) = *samples_guard {
                    for sample in data.iter_mut() {
                        if *index_guard < samples.len() {
                            *sample = samples[*index_guard];
                            *index_guard += 1;
                        } else {
                            *sample = 0.0;
                        }
                    }
                    
                    // Signal completion
                    if *index_guard >= samples.len() {
                        if let Some(tx) = tx_clone.blocking_lock().take() {
                            let _ = tx.send(());
                        }
                    }
                } else {
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                }
            },
            move |err| {
                eprintln!("Tone playback error: {}", err);
            },
            None,
        ).map_err(|e| ToneError::StreamError(e.to_string()))?;
        
        stream.play().map_err(|e| ToneError::PlayError(e.to_string()))?;
        
        // Wait for playback to complete or timeout
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            rx
        ).await;
        
        // Cleanup
        drop(stream);
        
        {
            let mut stored = self.samples.lock().await;
            *stored = None;
        }
        
        Ok(())
    }
    
    /// Play tone synchronously (blocking)
    pub fn play_sync(&self, tone_type: ToneType) -> Result<(), ToneError> {
        let samples = build_tone_sequence(tone_type);
        
        let host = cpal::default_host();
        let device = host.default_output_device()
            .ok_or(ToneError::NoOutputDevice)?;
        
        let config = StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };
        
        let samples = Arc::new(std::sync::Mutex::new(samples));
        let index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        
        let samples_clone = Arc::clone(&samples);
        let index_clone = Arc::clone(&index);
        let done_clone = Arc::clone(&done);
        
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let samples = samples_clone.lock().unwrap();
                
                for sample in data.iter_mut() {
                    let i = index_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i < samples.len() {
                        *sample = samples[i];
                    } else {
                        *sample = 0.0;
                        done_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
            move |err| {
                eprintln!("Tone playback error: {}", err);
            },
            None,
        ).map_err(|e| ToneError::StreamError(e.to_string()))?;
        
        stream.play().map_err(|e| ToneError::PlayError(e.to_string()))?;
        
        // Wait for completion
        while !done.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        
        // Small delay to let final samples play
        std::thread::sleep(std::time::Duration::from_millis(50));
        
        Ok(())
    }
}

impl Default for TonePlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Tone errors
#[derive(Debug, thiserror::Error)]
pub enum ToneError {
    #[error("No output device available")]
    NoOutputDevice,
    
    #[error("Stream error: {0}")]
    StreamError(String),
    
    #[error("Playback error: {0}")]
    PlayError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tone_generation() {
        let success = build_tone_sequence(ToneType::ConnectionSuccess);
        assert!(!success.is_empty());
        println!("ConnectionSuccess: {} samples ({:.2}s)", 
            success.len(), 
            success.len() as f32 / SAMPLE_RATE as f32);
        
        let delivered = build_tone_sequence(ToneType::MessageDelivered);
        assert!(!delivered.is_empty());
        println!("MessageDelivered: {} samples ({:.2}s)", 
            delivered.len(), 
            delivered.len() as f32 / SAMPLE_RATE as f32);
        
        let failed = build_tone_sequence(ToneType::Failed);
        assert!(!failed.is_empty());
        println!("Failed: {} samples ({:.2}s)", 
            failed.len(), 
            failed.len() as f32 / SAMPLE_RATE as f32);
        
        let roger = build_tone_sequence(ToneType::RogerBeep);
        assert!(!roger.is_empty());
        println!("RogerBeep: {} samples ({:.2}s)", 
            roger.len(), 
            roger.len() as f32 / SAMPLE_RATE as f32);
    }

    #[test]
    fn test_amplitude_range() {
        for tone_type in [
            ToneType::ConnectionSuccess,
            ToneType::MessageDelivered,
            ToneType::Failed,
            ToneType::RogerBeep,
        ] {
            let samples = build_tone_sequence(tone_type);
            for sample in &samples {
                assert!(*sample >= -1.0 && *sample <= 1.0, 
                    "Sample out of range: {}", sample);
            }
        }
    }
}
