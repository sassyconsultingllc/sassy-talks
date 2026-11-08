// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-XQV26S2RW56I
use ringbuf::{HeapConsumer, HeapProducer, HeapRb};
/// Audio Module for iOS
///
/// Interfaces with Swift's AVAudioEngine for recording and playback
/// Handles PCM audio frames for transmission
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Sample rate (48kHz for Opus)
pub const SAMPLE_RATE: u32 = 48000;

/// Frame size (20ms at 48kHz = 960 samples)
pub const FRAME_SIZE: usize = 960;

/// Ring buffer size (1 second)
const BUFFER_SIZE: usize = SAMPLE_RATE as usize;

/// After an underrun, hold playout until this much audio is queued (60 ms).
/// Relay frames arrive with 20-100 ms jitter; playing each frame the instant it
/// lands made every late frame a click of silence.
pub const PLAYOUT_PREBUFFER_SAMPLES: usize = FRAME_SIZE * 3;

/// Playout latency ceiling (300 ms). A burst (reconnect, relay catch-up, a
/// Wi-Fi stall releasing) used to sit in the 1 s ring and delay everything
/// after it for the rest of the transmission. Past this, the oldest queued
/// audio is dropped so the listener stays near real time.
pub const PLAYOUT_MAX_SAMPLES: usize = FRAME_SIZE * 15;

/// The speaker side of the output ring, safe to call from the Core Audio render
/// thread: it never blocks (a contended lock yields silence for one callback)
/// and never allocates. Holds only the consumer end, so it does not contend
/// with the global FFI state lock or the RX writer's producer lock.
#[derive(Clone)]
pub struct PlayoutHandle {
    consumer: Arc<Mutex<HeapConsumer<i16>>>,
    primed: Arc<AtomicBool>,
}

impl PlayoutHandle {
    /// Fill `out` from the ring. Returns samples written; the caller zero-fills
    /// the remainder.
    pub fn read(&self, out: &mut [i16]) -> usize {
        let Ok(mut consumer) = self.consumer.try_lock() else {
            return 0;
        };
        if !self.primed.load(Ordering::Acquire) {
            if consumer.len() < PLAYOUT_PREBUFFER_SAMPLES {
                return 0;
            }
            self.primed.store(true, Ordering::Release);
        }
        let n = consumer.pop_slice(out);
        if n < out.len() {
            // Ran dry: re-prime before resuming so the next late frame does not
            // produce another click.
            self.primed.store(false, Ordering::Release);
        }
        n
    }
}

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
    output_primed: Arc<AtomicBool>,
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
            output_primed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Handle for the render callback (see [`PlayoutHandle`]).
    pub fn playout_handle(&self) -> PlayoutHandle {
        PlayoutHandle {
            consumer: Arc::clone(&self.output_consumer),
            primed: Arc::clone(&self.output_primed),
        }
    }

    /// Drop any mic audio still queued from the previous press. Without this the
    /// tail of the last transmission (up to 1 s the TX thread never consumed)
    /// went out at the start of the next one.
    pub fn clear_input(&self) {
        self.input_consumer.lock().unwrap().clear();
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

    /// Write received audio frame to output buffer, dropping the oldest queued
    /// audio first if this frame would push latency past [`PLAYOUT_MAX_SAMPLES`].
    pub fn write_output_frame(&self, frame: &AudioFrame) -> Result<(), AudioError> {
        let mut producer = self.output_producer.lock().unwrap();
        let queued = producer.len();
        if queued + frame.samples.len() > PLAYOUT_MAX_SAMPLES {
            let excess = queued + frame.samples.len() - PLAYOUT_MAX_SAMPLES;
            // Lock order is always producer then consumer; the render thread
            // only ever try_locks the consumer, so this cannot deadlock it.
            self.output_consumer.lock().unwrap().skip(excess);
        }
        let written = producer.push_slice(&frame.samples);
        if written < frame.samples.len() {
            return Err(AudioError::BufferOverflow);
        }
        Ok(())
    }

    /// Read audio for Swift playback (prebuffered, non-blocking).
    pub fn read_output(&self, buffer: &mut [i16]) -> Result<usize, AudioError> {
        Ok(self.playout_handle().read(buffer))
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
        // ringbuf 0.3 Producer: free_len() = unoccupied slots (0.4 renamed it vacant_len).
        self.output_producer.lock().unwrap().free_len()
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(v: i16) -> AudioFrame {
        AudioFrame::new(vec![v; FRAME_SIZE])
    }

    #[test]
    fn playout_waits_for_the_prebuffer_then_plays_in_order() {
        let engine = AudioEngine::new();
        let play = engine.playout_handle();
        let mut out = vec![0i16; FRAME_SIZE];

        engine.write_output_frame(&frame(1)).unwrap();
        engine.write_output_frame(&frame(2)).unwrap();
        assert_eq!(
            play.read(&mut out),
            0,
            "two frames is under the 60 ms prebuffer"
        );

        engine.write_output_frame(&frame(3)).unwrap();
        assert_eq!(play.read(&mut out), FRAME_SIZE);
        assert!(out.iter().all(|&s| s == 1));
        assert_eq!(play.read(&mut out), FRAME_SIZE);
        assert!(out.iter().all(|&s| s == 2));
    }

    #[test]
    fn underrun_reprimes_instead_of_clicking_frame_by_frame() {
        let engine = AudioEngine::new();
        let play = engine.playout_handle();
        let mut out = vec![0i16; FRAME_SIZE];
        for v in 1..=3 {
            engine.write_output_frame(&frame(v)).unwrap();
        }
        for _ in 0..3 {
            assert_eq!(play.read(&mut out), FRAME_SIZE);
        }
        assert_eq!(play.read(&mut out), 0, "ran dry");
        // One late frame alone does not restart playout.
        engine.write_output_frame(&frame(9)).unwrap();
        assert_eq!(play.read(&mut out), 0);
    }

    #[test]
    fn a_burst_is_capped_to_the_latency_ceiling_keeping_the_newest_audio() {
        let engine = AudioEngine::new();
        let play = engine.playout_handle();
        for v in 0..40 {
            engine.write_output_frame(&frame(v)).unwrap();
        }
        let mut out = vec![0i16; FRAME_SIZE];
        let mut frames = Vec::new();
        while play.read(&mut out) == FRAME_SIZE {
            frames.push(out[0]);
        }
        let cap = PLAYOUT_MAX_SAMPLES / FRAME_SIZE;
        assert_eq!(frames.len(), cap);
        assert_eq!(*frames.last().unwrap(), 39, "newest audio survives");
        assert_eq!(frames[0], 40 - cap as i16);
    }

    #[test]
    fn clear_input_drops_the_previous_press_tail() {
        let engine = AudioEngine::new();
        engine.write_input(&vec![7i16; FRAME_SIZE * 2]).unwrap();
        engine.clear_input();
        assert!(
            engine.read_input_frame().is_err(),
            "nothing stale left to transmit"
        );
    }
}
