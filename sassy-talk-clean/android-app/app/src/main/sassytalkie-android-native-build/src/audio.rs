use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::jni_bridge::{AndroidAudioRecord, AndroidAudioTrack};

pub const SAMPLE_RATE: i32 = 48000;
pub const CHANNEL_CONFIG_MONO: i32 = 16;
pub const CHANNEL_CONFIG_OUT_MONO: i32 = 4;
pub const AUDIO_FORMAT_PCM_16: i32 = 2;
#[allow(dead_code)]
pub const FRAME_SIZE: usize = 960;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioState { Idle, Recording, Playing, Error }

pub struct AudioEngine {
    recorder: Arc<Mutex<Option<AndroidAudioRecord>>>, player: Arc<Mutex<Option<AndroidAudioTrack>>>,
    recording: Arc<AtomicBool>, playing: Arc<AtomicBool>, state: Arc<Mutex<AudioState>>,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        Ok(Self { recorder: Arc::new(Mutex::new(None)), player: Arc::new(Mutex::new(None)),
            recording: Arc::new(AtomicBool::new(false)), playing: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(AudioState::Idle)) })
    }
    pub fn init_recorder(&self) -> Result<(), String> {
        let bs = AndroidAudioRecord::get_min_buffer_size(SAMPLE_RATE, CHANNEL_CONFIG_MONO, AUDIO_FORMAT_PCM_16)?;
        let rec = AndroidAudioRecord::new(SAMPLE_RATE, CHANNEL_CONFIG_MONO, AUDIO_FORMAT_PCM_16, bs * 2)?;
        *self.recorder.lock().unwrap() = Some(rec); Ok(())
    }
    pub fn init_player(&self) -> Result<(), String> {
        let bs = AndroidAudioRecord::get_min_buffer_size(SAMPLE_RATE, CHANNEL_CONFIG_MONO, AUDIO_FORMAT_PCM_16)?;
        let p = AndroidAudioTrack::new(SAMPLE_RATE, CHANNEL_CONFIG_OUT_MONO, AUDIO_FORMAT_PCM_16, bs * 2)?;
        *self.player.lock().unwrap() = Some(p); Ok(())
    }
    pub fn start_recording(&self) -> Result<(), String> {
        if self.recorder.lock().unwrap().is_none() { self.init_recorder()?; }
        let r = self.recorder.lock().unwrap();
        if let Some(rec) = r.as_ref() { rec.start_recording()?; self.recording.store(true, Ordering::Relaxed); *self.state.lock().unwrap() = AudioState::Recording; Ok(()) }
        else { Err("Recorder not initialized".into()) }
    }
    pub fn stop_recording(&self) -> Result<(), String> {
        self.recording.store(false, Ordering::Relaxed);
        let r = self.recorder.lock().unwrap();
        if let Some(rec) = r.as_ref() { rec.stop()?; *self.state.lock().unwrap() = AudioState::Idle; }
        Ok(())
    }
    pub fn read_audio(&self, buffer: &mut [i16]) -> Result<usize, String> {
        self.recorder.lock().unwrap().as_ref().ok_or("No recorder")?.read(buffer)
    }
    pub fn start_playing(&self) -> Result<(), String> {
        if self.player.lock().unwrap().is_none() { self.init_player()?; }
        let p = self.player.lock().unwrap();
        if let Some(play) = p.as_ref() { play.play()?; self.playing.store(true, Ordering::Relaxed); *self.state.lock().unwrap() = AudioState::Playing; Ok(()) }
        else { Err("Player not initialized".into()) }
    }
    pub fn stop_playing(&self) -> Result<(), String> {
        self.playing.store(false, Ordering::Relaxed);
        let p = self.player.lock().unwrap();
        if let Some(play) = p.as_ref() { play.stop()?; *self.state.lock().unwrap() = AudioState::Idle; }
        Ok(())
    }
    pub fn write_audio(&self, buffer: &[i16]) -> Result<usize, String> {
        self.player.lock().unwrap().as_ref().ok_or("No player")?.write(buffer)
    }
    pub fn is_recording(&self) -> bool { self.recording.load(Ordering::Relaxed) }
    pub fn is_playing(&self) -> bool { self.playing.load(Ordering::Relaxed) }
    #[allow(dead_code)]
    pub fn get_state(&self) -> AudioState { *self.state.lock().unwrap() }
    pub fn release(&self) -> Result<(), String> {
        if self.is_recording() { self.stop_recording()?; }
        if self.is_playing() { self.stop_playing()?; }
        if let Some(r) = self.recorder.lock().unwrap().as_ref() { r.release()?; }
        if let Some(p) = self.player.lock().unwrap().as_ref() { p.release()?; }
        *self.recorder.lock().unwrap() = None; *self.player.lock().unwrap() = None;
        *self.state.lock().unwrap() = AudioState::Idle; Ok(())
    }
}
impl Drop for AudioEngine { fn drop(&mut self) { let _ = self.release(); } }

#[allow(dead_code)]
pub struct AudioFrame { pub samples: Vec<i16>, pub timestamp: u64 }
#[allow(dead_code)]
impl AudioFrame {
    pub fn new(size: usize) -> Self { Self { samples: vec![0; size], timestamp: 0 } }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(8 + self.samples.len() * 2);
        b.extend_from_slice(&self.timestamp.to_le_bytes());
        for s in &self.samples { b.extend_from_slice(&s.to_le_bytes()); }
        b
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 8 { return Err("Too short".into()); }
        let timestamp = u64::from_le_bytes([bytes[0],bytes[1],bytes[2],bytes[3],bytes[4],bytes[5],bytes[6],bytes[7]]);
        let audio = &bytes[8..];
        if audio.len() % 2 != 0 { return Err("Odd sample bytes".into()); }
        let samples: Vec<i16> = audio.chunks_exact(2).map(|c| i16::from_le_bytes([c[0],c[1]])).collect();
        Ok(Self { samples, timestamp })
    }
}
