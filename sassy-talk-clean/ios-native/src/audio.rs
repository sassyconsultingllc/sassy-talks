/// Audio Module for iOS
/// 
/// Interfaces with Swift's AVAudioEngine for recording and playback
/// Handles PCM audio frames for transmission

use std::sync::{Arc, Mutex};
use ringbuf::{HeapRb, HeapProducer, HeapConsumer};
use thiserror::Error;

/// Sample rate (48kHz for Opus)
pub const SAMPLE_RATE: u32 = 48000;

/// Frame size (20ms at 48kHz = 960 samples)
pub const FRAME_SIZE: usize = 960;

/// Ring buffer size (1 second)
const BUFFER_SIZE: usize = SAMPLE_RATE as usize;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Buffer overflow")]
    BufferOverflow,
    
    #[error("Buffer underflow")]
    BufferUnderflow,
    
    #[error("Invalid frame size: {0}")]
    InvalidFrameSize(usize),
    
    #[error("Not recording")]
    NotRecording,
    
    #[error("Not playing")]
    NotPlaying,
}

/// Audio frame for transmission
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

impl AudioFrame {
    /// Create new audio frame
    pub fn new(samples: Vec<i16>) -> Self {
        Self {
            samples,
            sample_rate: SAMPLE_RATE,
        }
    }
    
    /// Convert to bytes for transmission
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.samples.len() * 2);
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }
    
    /// Convert from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AudioError> {
        if bytes.len() % 2 != 0 {
            return Err(AudioError::InvalidFrameSize(bytes.len()));
        }
        
        let mut samples = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            samples.push(sample);
        }
        
        Ok(Self::new(samples))
    }
}

/// Audio engine state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioState {
    Idle,
    Recording,
    Playing,
    RecordingAndPlaying,
}

/// Audio engine
/// 
/// Note: Actual audio I/O happens in Swift via AVAudioEngine
/// This manages buffers and state for Rust side
pub struct AudioEngine {
    // State
    state: Arc<Mutex<AudioState>>,
    
    // Input ring buffer (mic → transmission)
    input_producer: Arc<Mutex<HeapProducer<i16>>>,
    input_consumer: Arc<Mutex<HeapConsumer<i16>>>,
    
    // Output ring buffer (reception → speaker)
    output_producer: Arc<Mutex<HeapProducer<i16>>>,
    output_consumer: Arc<Mutex<HeapConsumer<i16>>>,
}

impl AudioEngine {
    /// Create new audio engine
    pub fn new() -> Self {
        // Create ring buffers
        let input_rb = HeapRb::<i16>::new(BUFFER_SIZE);
        let (input_producer, input_consumer) = input_rb.split();
        
        let output_rb = HeapRb::<i16>::new(BUFFER_SIZE);
        let (output_producer, output_consumer) = output_rb.split();
        
        Self {
            state: Arc::new(Mutex::new(AudioState::Idle)),
            input_producer: Arc::new(Mutex::new(input_producer)),
            input_consumer: Arc::new(Mutex::new(input_consumer)),
            output_producer: Arc::new(Mutex::new(output_producer)),
            output_consumer: Arc::new(Mutex::new(output_consumer)),
        }
    }
    
    /// Start recording
    pub fn start_recording(&mut self) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::Idle => AudioState::Recording,
            AudioState::Playing => AudioState::RecordingAndPlaying,
            _ => *state,
        };
        Ok(())
    }
    
    /// Stop recording
    pub fn stop_recording(&mut self) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::Recording => AudioState::Idle,
            AudioState::RecordingAndPlaying => AudioState::Playing,
            _ => *state,
        };
        Ok(())
    }
    
    /// Start playback
    pub fn start_playing(&mut self) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::Idle => AudioState::Playing,
            AudioState::Recording => AudioState::RecordingAndPlaying,
            _ => *state,
        };
        Ok(())
    }
    
    /// Stop playback
    pub fn stop_playing(&mut self) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        *state = match *state {
            AudioState::Playing => AudioState::Idle,
            AudioState::RecordingAndPlaying => AudioState::Recording,
            _ => *state,
        };
        Ok(())
    }
    
    /// Write audio from Swift to input buffer
    /// Called by Swift's AVAudioEngine callback
    pub fn write_input(&self, samples: &[i16]) -> Result<(), AudioError> {
        let mut producer = self.input_producer.lock().unwrap();
        let written = producer.push_slice(samples);
        if written < samples.len() {
            return Err(AudioError::BufferOverflow);
        }
        Ok(())
    }
    
    /// Read audio frame for transmission
    pub fn read_input_frame(&self) -> Result<AudioFrame, AudioError> {
        let mut consumer = self.input_consumer.lock().unwrap();
        
        if consumer.len() < FRAME_SIZE {
            return Err(AudioError::BufferUnderflow);
        }
        
        let mut samples = vec![0i16; FRAME_SIZE];
        let read = consumer.pop_slice(&mut samples);
        
        if read < FRAME_SIZE {
            return Err(AudioError::BufferUnderflow);
        }
        
        Ok(AudioFrame::new(samples))
    }
    
    /// Write received audio frame to output buffer
    pub fn write_output_frame(&self, frame: &AudioFrame) -> Result<(), AudioError> {
        let mut producer = self.output_producer.lock().unwrap();
        let written = producer.push_slice(&frame.samples);
        if written < frame.samples.len() {
            return Err(AudioError::BufferOverflow);
        }
        Ok(())
    }
    
    /// Read audio for Swift playback
    /// Called by Swift's AVAudioEngine callback
    pub fn read_output(&self, buffer: &mut [i16]) -> Result<usize, AudioError> {
        let mut consumer = self.output_consumer.lock().unwrap();
        Ok(consumer.pop_slice(buffer))
    }
    
    /// Get current state
    pub fn state(&self) -> AudioState {
        *self.state.lock().unwrap()
    }
    
    /// Get available input samples
    pub fn input_available(&self) -> usize {
        self.input_consumer.lock().unwrap().len()
    }
    
    /// Get available output space
    pub fn output_available(&self) -> usize {
        self.output_producer.lock().unwrap().vacant_len()
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}
