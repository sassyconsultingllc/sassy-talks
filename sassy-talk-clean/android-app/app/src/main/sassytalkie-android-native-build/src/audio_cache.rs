use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const SPEECH_GAP_MS: u64 = 400;
const MAX_FRAMES_PER_SPEAKER: usize = 500;
const MAX_CACHED_SPEAKERS: usize = 16;
const MAX_QUEUED_UTTERANCES: usize = 32;

#[derive(Clone)]
#[allow(dead_code)]
pub struct CachedFrame { pub sender_id: String, pub timestamp: u64, pub samples: Vec<i16>, received_at: Instant }

pub struct Utterance {
    pub sender_id: String, pub sender_name: String, pub is_favorite: bool,
    pub started_at: u64, pub ended_at: u64, pub frames: Vec<CachedFrame>, pub fully_played: bool,
}
impl Utterance {
    fn duration_ms(&self) -> u64 { if self.frames.is_empty() { 0 } else { self.ended_at - self.started_at + 20 } }
    #[allow(dead_code)]
    fn frame_count(&self) -> usize { self.frames.len() }
}

struct SpeakerBuffer { sender_id: String, frames: Vec<CachedFrame>, last_frame_at: Instant, first_timestamp: u64, last_timestamp: u64 }
impl SpeakerBuffer {
    fn new(id: &str) -> Self { Self { sender_id: id.to_string(), frames: Vec::new(), last_frame_at: Instant::now(), first_timestamp: 0, last_timestamp: 0 } }
    fn push_frame(&mut self, frame: CachedFrame) {
        if self.frames.is_empty() { self.first_timestamp = frame.timestamp; }
        self.last_timestamp = frame.timestamp; self.last_frame_at = Instant::now();
        if self.frames.len() < MAX_FRAMES_PER_SPEAKER { self.frames.push(frame); }
    }
    fn is_speech_complete(&self) -> bool { !self.frames.is_empty() && self.last_frame_at.elapsed() > Duration::from_millis(SPEECH_GAP_MS) }
    fn drain_to_utterance(&mut self, name: &str, is_fav: bool) -> Utterance {
        let frames = std::mem::take(&mut self.frames);
        let s = self.first_timestamp; let e = self.last_timestamp;
        self.first_timestamp = 0; self.last_timestamp = 0;
        Utterance { sender_id: self.sender_id.clone(), sender_name: name.to_string(), is_favorite: is_fav, started_at: s, ended_at: e, frames, fully_played: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CacheMode { Live, Queue, Replay }

pub struct AudioCache {
    active_buffers: HashMap<String, SpeakerBuffer>, playback_queue: VecDeque<Utterance>,
    now_playing: Option<Utterance>, play_cursor: usize, mode: CacheMode,
    user_info: HashMap<String, (String, bool, bool)>,
    history: VecDeque<Utterance>, max_history: usize,
}

impl AudioCache {
    pub fn new() -> Self {
        Self { active_buffers: HashMap::new(), playback_queue: VecDeque::new(), now_playing: None,
            play_cursor: 0, mode: CacheMode::Live, user_info: HashMap::new(), history: VecDeque::new(), max_history: 50 }
    }
    pub fn update_user_info(&mut self, id: &str, name: &str, is_fav: bool, is_muted: bool) {
        self.user_info.insert(id.to_string(), (name.to_string(), is_fav, is_muted));
    }
    pub fn ingest_frame(&mut self, sender_id: &str, timestamp: u64, samples: Vec<i16>) -> Option<Vec<i16>> {
        if let Some((_, _, true)) = self.user_info.get(sender_id) { return None; }
        let frame = CachedFrame { sender_id: sender_id.to_string(), timestamp, samples: samples.clone(), received_at: Instant::now() };
        if !self.active_buffers.contains_key(sender_id) {
            if self.active_buffers.len() >= MAX_CACHED_SPEAKERS { return None; }
            self.active_buffers.insert(sender_id.to_string(), SpeakerBuffer::new(sender_id));
        }
        self.active_buffers.get_mut(sender_id).unwrap().push_frame(frame);
        let active = self.active_buffers.len();
        if active > 1 && self.mode == CacheMode::Live { self.mode = CacheMode::Queue; }
        if self.mode == CacheMode::Live && active <= 1 && self.now_playing.is_none() { return Some(samples); }
        None
    }
    pub fn tick(&mut self) {
        let completed: Vec<String> = self.active_buffers.iter().filter(|(_, b)| b.is_speech_complete()).map(|(id, _)| id.clone()).collect();
        for id in completed {
            if let Some(mut buf) = self.active_buffers.remove(&id) {
                let (name, is_fav, is_muted) = self.user_info.get(&id).cloned().unwrap_or_else(|| (id.clone(), false, false));
                if is_muted { continue; }
                let utt = buf.drain_to_utterance(&name, is_fav);
                if utt.frames.is_empty() { continue; }
                self.insert_prioritized(utt);
            }
        }
        if self.mode == CacheMode::Queue && self.playback_queue.is_empty() && self.now_playing.is_none() && self.active_buffers.is_empty() { self.mode = CacheMode::Live; }
        while self.playback_queue.len() > MAX_QUEUED_UTTERANCES { self.playback_queue.pop_back(); }
    }
    pub fn next_playback_frame(&mut self) -> Option<(String, Vec<i16>)> {
        if let Some(ref utt) = self.now_playing {
            if self.play_cursor < utt.frames.len() {
                let f = &utt.frames[self.play_cursor]; self.play_cursor += 1;
                return Some((f.sender_id.clone(), f.samples.clone()));
            }
            let mut fin = self.now_playing.take().unwrap(); fin.fully_played = true;
            if self.history.len() >= self.max_history { self.history.pop_front(); }
            self.history.push_back(fin);
        }
        if let Some(next) = self.playback_queue.pop_front() {
            let first = if !next.frames.is_empty() { Some((next.frames[0].sender_id.clone(), next.frames[0].samples.clone())) } else { None };
            self.now_playing = Some(next); self.play_cursor = 1;
            return first;
        }
        None
    }
    #[allow(dead_code)]
    pub fn mode(&self) -> CacheMode { self.mode }
    pub fn set_mode(&mut self, mode: CacheMode) { self.mode = mode; }
    pub fn skip_current(&mut self) { if self.now_playing.take().is_some() { self.play_cursor = 0; } }
    pub fn replay_from_history(&mut self, index: usize) -> bool {
        if index >= self.history.len() { return false; }
        let orig = &self.history[index];
        let replay = Utterance { sender_id: orig.sender_id.clone(), sender_name: orig.sender_name.clone(), is_favorite: orig.is_favorite,
            started_at: orig.started_at, ended_at: orig.ended_at, frames: orig.frames.clone(), fully_played: false };
        self.mode = CacheMode::Replay; self.now_playing = Some(replay); self.play_cursor = 0; true
    }
    pub fn clear(&mut self) {
        self.active_buffers.clear(); self.playback_queue.clear(); self.now_playing = None; self.play_cursor = 0; self.mode = CacheMode::Live;
    }
    pub fn status_json(&self) -> String {
        let current_speaker = self.now_playing.as_ref().map(|u| u.sender_id.clone());
        let current_name = self.now_playing.as_ref().map(|u| u.sender_name.clone());
        let speakers: Vec<String> = self.playback_queue.iter().map(|u| u.sender_name.clone()).collect();
        let qd: u64 = self.playback_queue.iter().map(|u| u.duration_ms()).sum();
        serde_json::json!({"mode":format!("{:?}",self.mode),"queued_utterances":self.playback_queue.len(),"queued_duration_ms":qd,"current_speaker":current_speaker,"current_speaker_name":current_name,"speakers_in_queue":speakers,"history_count":self.history.len()}).to_string()
    }
    fn insert_prioritized(&mut self, utterance: Utterance) {
        if utterance.is_favorite {
            let insert_at = self.playback_queue.iter().position(|u| !u.is_favorite).unwrap_or(self.playback_queue.len());
            let final_pos = self.playback_queue.iter().take(insert_at).rposition(|u| u.started_at <= utterance.started_at).map(|p| p + 1).unwrap_or(0);
            self.playback_queue.insert(final_pos, utterance);
        } else {
            self.playback_queue.push_back(utterance);
        }
    }
}
