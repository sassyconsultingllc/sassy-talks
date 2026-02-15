/// Audio Cache - Multi-Speaker Store/Replay System (Dane.com-style)
///
/// Problem: When multiple people talk at once on a walkie-talkie channel,
/// their audio overlaps and you miss messages. Traditional radios just
/// stomp one speaker over another.
///
/// Solution: Cache incoming audio per-speaker, queue it, and replay
/// sequentially so every person is heard in full, even if they spoke
/// simultaneously. Think of it like a voicemail-style catch-up buffer.
///
/// How it works:
/// 1. RX thread deposits frames into per-speaker ring buffers
/// 2. The mixer drains speakers in priority order:
///    - Favorites first, then others (Wyze-style)
///    - Within a tier, FIFO by speech-start timestamp
/// 3. While one speaker is playing, new arrivals queue up
/// 4. When current speaker finishes, next queued speaker auto-plays
/// 5. "Catch-up" indicator shows how many speakers are queued
///
/// Wire format needed: [channel:1][sender_id:16][timestamp:8][samples:N*2]
/// The sender_id comes from UserRegistry::derive_user_id()

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use log::{info, warn};

use crate::audio::FRAME_SIZE;

/// How long silence before we consider a speaker "done talking"
const SPEECH_GAP_MS: u64 = 400;

/// Maximum cached frames per speaker (prevents memory bloat)
/// At 20ms/frame, 500 frames = 10 seconds of audio per speaker
const MAX_FRAMES_PER_SPEAKER: usize = 500;

/// Maximum number of speakers we'll cache simultaneously
const MAX_CACHED_SPEAKERS: usize = 16;

/// Maximum total queued utterances before we start dropping oldest
const MAX_QUEUED_UTTERANCES: usize = 32;

/// A single audio frame with sender metadata
#[derive(Clone)]
pub struct CachedFrame {
    pub sender_id: String,
    pub timestamp: u64,
    pub samples: Vec<i16>,
    pub received_at: Instant,
}

/// A complete utterance (contiguous speech) from one speaker
pub struct Utterance {
    pub sender_id: String,
    pub sender_name: String,
    pub is_favorite: bool,
    pub started_at: u64,       // first frame timestamp
    pub ended_at: u64,         // last frame timestamp
    pub frames: Vec<CachedFrame>,
    pub fully_played: bool,
}

impl Utterance {
    fn duration_ms(&self) -> u64 {
        if self.frames.is_empty() { return 0; }
        self.ended_at - self.started_at + 20 // +20 for last frame duration
    }

    fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

/// Per-speaker accumulator: collects frames until speech gap detected
struct SpeakerBuffer {
    sender_id: String,
    frames: Vec<CachedFrame>,
    last_frame_at: Instant,
    first_timestamp: u64,
    last_timestamp: u64,
}

impl SpeakerBuffer {
    fn new(sender_id: &str) -> Self {
        Self {
            sender_id: sender_id.to_string(),
            frames: Vec::new(),
            last_frame_at: Instant::now(),
            first_timestamp: 0,
            last_timestamp: 0,
        }
    }

    fn push_frame(&mut self, frame: CachedFrame) {
        if self.frames.is_empty() {
            self.first_timestamp = frame.timestamp;
        }
        self.last_timestamp = frame.timestamp;
        self.last_frame_at = Instant::now();

        if self.frames.len() < MAX_FRAMES_PER_SPEAKER {
            self.frames.push(frame);
        } else {
            warn!("AudioCache: speaker {} buffer full, dropping frame", self.sender_id);
        }
    }

    /// Returns true if enough silence has passed to finalize this utterance
    fn is_speech_complete(&self) -> bool {
        if self.frames.is_empty() {
            return false;
        }
        self.last_frame_at.elapsed() > Duration::from_millis(SPEECH_GAP_MS)
    }

    /// Drain frames into an Utterance
    fn drain_to_utterance(&mut self, sender_name: &str, is_favorite: bool) -> Utterance {
        let frames = std::mem::take(&mut self.frames);
        let started = self.first_timestamp;
        let ended = self.last_timestamp;

        self.first_timestamp = 0;
        self.last_timestamp = 0;

        Utterance {
            sender_id: self.sender_id.clone(),
            sender_name: sender_name.to_string(),
            is_favorite,
            started_at: started,
            ended_at: ended,
            frames,
            fully_played: false,
        }
    }
}

/// Cache mode determines how audio is handled
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CacheMode {
    /// Live passthrough (no caching, traditional walkie-talkie)
    /// When only one person talks at a time, behaves normally
    Live,

    /// Queue mode: cache all incoming, play sequentially
    /// Activates automatically when overlap is detected
    Queue,

    /// Replay mode: user manually scrubbing through cached audio
    Replay,
}

/// Status info for the UI "catch-up" indicator
#[derive(Debug, Clone)]
pub struct CacheStatus {
    pub mode: CacheMode,
    pub queued_utterances: usize,
    pub queued_duration_ms: u64,
    pub current_speaker: Option<String>,
    pub current_speaker_name: Option<String>,
    pub speakers_in_queue: Vec<String>,
}

/// The main audio cache / mixer
pub struct AudioCache {
    /// Per-speaker frame accumulator (actively receiving)
    active_buffers: HashMap<String, SpeakerBuffer>,

    /// Finalized utterances waiting to be played, priority ordered
    playback_queue: VecDeque<Utterance>,

    /// Currently playing utterance
    now_playing: Option<Utterance>,
    /// Frame index within now_playing
    play_cursor: usize,

    /// Current operating mode
    mode: CacheMode,

    /// Lookup: sender_id → (name, is_favorite, is_muted)
    /// Refreshed from UserRegistry periodically
    user_info: HashMap<String, (String, bool, bool)>,

    /// History of played utterances (for replay scrubbing)
    history: VecDeque<Utterance>,
    /// Max history entries
    max_history: usize,
}

impl AudioCache {
    pub fn new() -> Self {
        Self {
            active_buffers: HashMap::new(),
            playback_queue: VecDeque::new(),
            now_playing: None,
            play_cursor: 0,
            mode: CacheMode::Live,
            user_info: HashMap::new(),
            history: VecDeque::new(),
            max_history: 50,
        }
    }

    /// Update user info from UserRegistry (call periodically or on change)
    pub fn update_user_info(&mut self, sender_id: &str, name: &str, is_favorite: bool, is_muted: bool) {
        self.user_info.insert(
            sender_id.to_string(),
            (name.to_string(), is_favorite, is_muted),
        );
    }

    /// Ingest a received audio frame from the RX thread
    ///
    /// Returns Some(samples) if frame should be played immediately (Live mode),
    /// or None if frame was cached for later playback (Queue mode).
    pub fn ingest_frame(&mut self, sender_id: &str, timestamp: u64, samples: Vec<i16>) -> Option<Vec<i16>> {
        // Check mute status — drop silently
        if let Some((_, _, is_muted)) = self.user_info.get(sender_id) {
            if *is_muted {
                return None;
            }
        }

        let frame = CachedFrame {
            sender_id: sender_id.to_string(),
            timestamp,
            samples: samples.clone(),
            received_at: Instant::now(),
        };

        // Get or create speaker buffer
        if !self.active_buffers.contains_key(sender_id) {
            if self.active_buffers.len() >= MAX_CACHED_SPEAKERS {
                warn!("AudioCache: max speakers reached, dropping new speaker {}", sender_id);
                return None;
            }
            self.active_buffers.insert(
                sender_id.to_string(),
                SpeakerBuffer::new(sender_id),
            );
        }
        self.active_buffers.get_mut(sender_id).unwrap().push_frame(frame);

        // Detect overlap: if >1 speaker has active buffers, switch to Queue mode
        let active_speakers = self.active_buffers.len();
        if active_speakers > 1 && self.mode == CacheMode::Live {
            info!("AudioCache: overlap detected ({} speakers), switching to Queue mode", active_speakers);
            self.mode = CacheMode::Queue;
        }

        // In Live mode with single speaker, pass through immediately
        if self.mode == CacheMode::Live && active_speakers <= 1 && self.now_playing.is_none() {
            return Some(samples);
        }

        // In Queue mode, frames are buffered — played via next_playback_frame()
        None
    }

    /// Called periodically by the RX/playback thread to check for completed utterances
    /// and move them to the playback queue
    pub fn tick(&mut self) {
        // Check each active buffer for speech completion
        let completed_ids: Vec<String> = self.active_buffers.iter()
            .filter(|(_, buf)| buf.is_speech_complete())
            .map(|(id, _)| id.clone())
            .collect();

        for id in completed_ids {
            if let Some(mut buffer) = self.active_buffers.remove(&id) {
                let (name, is_fav, is_muted) = self.user_info.get(&id)
                    .cloned()
                    .unwrap_or_else(|| (id.clone(), false, false));

                if is_muted {
                    continue; // Don't queue muted speakers
                }

                let utterance = buffer.drain_to_utterance(&name, is_fav);
                if utterance.frames.is_empty() {
                    continue;
                }

                info!("AudioCache: utterance complete from {} ({} frames, {}ms)",
                    name, utterance.frame_count(), utterance.duration_ms());

                // Insert in priority order: favorites first, then by timestamp
                self.insert_prioritized(utterance);
            }
        }

        // If we're in Queue mode but queue is empty and no active buffers,
        // switch back to Live
        if self.mode == CacheMode::Queue
            && self.playback_queue.is_empty()
            && self.now_playing.is_none()
            && self.active_buffers.is_empty()
        {
            info!("AudioCache: queue drained, switching back to Live mode");
            self.mode = CacheMode::Live;
        }

        // Enforce queue size limit
        while self.playback_queue.len() > MAX_QUEUED_UTTERANCES {
            let dropped = self.playback_queue.pop_back();
            if let Some(u) = dropped {
                warn!("AudioCache: dropping oldest utterance from {} (queue full)", u.sender_name);
            }
        }
    }

    /// Get the next frame to play from the queue
    ///
    /// Returns (sender_id, samples) or None if nothing to play
    pub fn next_playback_frame(&mut self) -> Option<(String, Vec<i16>)> {
        // If currently playing an utterance, advance cursor
        if let Some(ref utterance) = self.now_playing {
            if self.play_cursor < utterance.frames.len() {
                let frame = &utterance.frames[self.play_cursor];
                self.play_cursor += 1;
                return Some((frame.sender_id.clone(), frame.samples.clone()));
            }

            // Current utterance finished
            let mut finished = self.now_playing.take().unwrap();
            finished.fully_played = true;
            info!("AudioCache: finished playing utterance from {}", finished.sender_name);

            // Move to history
            if self.history.len() >= self.max_history {
                self.history.pop_front();
            }
            self.history.push_back(finished);
        }

        // Advance to next utterance in queue
        if let Some(next) = self.playback_queue.pop_front() {
            info!("AudioCache: now playing from {} ({} frames)",
                next.sender_name, next.frame_count());

            let first_frame = if !next.frames.is_empty() {
                Some((next.frames[0].sender_id.clone(), next.frames[0].samples.clone()))
            } else {
                None
            };

            self.now_playing = Some(next);
            self.play_cursor = 1; // Already consumed frame 0

            return first_frame;
        }

        None
    }

    /// Get current cache status for UI
    pub fn status(&self) -> CacheStatus {
        let queued_duration: u64 = self.playback_queue.iter()
            .map(|u| u.duration_ms())
            .sum();

        let current_speaker = self.now_playing.as_ref().map(|u| u.sender_id.clone());
        let current_speaker_name = self.now_playing.as_ref().map(|u| u.sender_name.clone());

        let speakers_in_queue: Vec<String> = self.playback_queue.iter()
            .map(|u| u.sender_name.clone())
            .collect();

        CacheStatus {
            mode: self.mode,
            queued_utterances: self.playback_queue.len(),
            queued_duration_ms: queued_duration,
            current_speaker,
            current_speaker_name,
            speakers_in_queue,
        }
    }

    /// Get current mode
    pub fn mode(&self) -> CacheMode {
        self.mode
    }

    /// Force switch to a specific mode
    pub fn set_mode(&mut self, mode: CacheMode) {
        info!("AudioCache: mode forced to {:?}", mode);
        self.mode = mode;
    }

    /// Skip the current utterance and move to next
    pub fn skip_current(&mut self) {
        if let Some(skipped) = self.now_playing.take() {
            info!("AudioCache: skipped utterance from {}", skipped.sender_name);
            self.play_cursor = 0;
            // Don't add to history since it wasn't fully played
        }
    }

    /// Get the replay history for scrubbing
    pub fn history(&self) -> &VecDeque<Utterance> {
        &self.history
    }

    /// Replay a specific utterance from history by index
    pub fn replay_from_history(&mut self, index: usize) -> bool {
        if index >= self.history.len() {
            return false;
        }

        // Clone the utterance frames for replay
        let original = &self.history[index];
        let replay = Utterance {
            sender_id: original.sender_id.clone(),
            sender_name: original.sender_name.clone(),
            is_favorite: original.is_favorite,
            started_at: original.started_at,
            ended_at: original.ended_at,
            frames: original.frames.clone(),
            fully_played: false,
        };

        self.mode = CacheMode::Replay;
        self.now_playing = Some(replay);
        self.play_cursor = 0;
        true
    }

    /// Clear all cached audio
    pub fn clear(&mut self) {
        self.active_buffers.clear();
        self.playback_queue.clear();
        self.now_playing = None;
        self.play_cursor = 0;
        self.mode = CacheMode::Live;
        info!("AudioCache: cleared all caches");
    }

    /// Serialize cache status to JSON (for JNI bridge)
    pub fn status_json(&self) -> String {
        let status = self.status();
        serde_json::json!({
            "mode": format!("{:?}", status.mode),
            "queued_utterances": status.queued_utterances,
            "queued_duration_ms": status.queued_duration_ms,
            "current_speaker": status.current_speaker,
            "current_speaker_name": status.current_speaker_name,
            "speakers_in_queue": status.speakers_in_queue,
            "history_count": self.history.len(),
        }).to_string()
    }

    // ── Internal ──

    /// Insert utterance into queue with priority ordering:
    /// 1. Favorites before non-favorites
    /// 2. Within same priority, ordered by speech start timestamp (FIFO)
    fn insert_prioritized(&mut self, utterance: Utterance) {
        if utterance.is_favorite {
            // Find insertion point: after last favorite, before first non-favorite
            let insert_at = self.playback_queue.iter()
                .position(|u| !u.is_favorite)
                .unwrap_or(self.playback_queue.len());

            // Within favorites, maintain timestamp order
            let final_pos = self.playback_queue.iter()
                .take(insert_at)
                .rposition(|u| u.started_at <= utterance.started_at)
                .map(|p| p + 1)
                .unwrap_or(0);

            self.playback_queue.insert(final_pos, utterance);
        } else {
            // Non-favorites go at the end, ordered by timestamp
            self.playback_queue.push_back(utterance);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_live_passthrough() {
        let mut cache = AudioCache::new();
        assert_eq!(cache.mode(), CacheMode::Live);

        let samples = vec![100i16; FRAME_SIZE];
        let result = cache.ingest_frame("alice", 1000, samples.clone());

        // Single speaker in Live mode → passthrough
        assert!(result.is_some());
        assert_eq!(result.unwrap(), samples);
    }

    #[test]
    fn test_cache_overlap_triggers_queue_mode() {
        let mut cache = AudioCache::new();

        // Speaker 1
        cache.ingest_frame("alice", 1000, vec![100i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Live);

        // Speaker 2 arrives while speaker 1 still active → Queue
        cache.ingest_frame("bob", 1001, vec![200i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Queue);
    }

    #[test]
    fn test_muted_speaker_dropped() {
        let mut cache = AudioCache::new();
        cache.update_user_info("bob", "Bob", false, true); // muted

        let result = cache.ingest_frame("bob", 1000, vec![100i16; FRAME_SIZE]);
        assert!(result.is_none()); // Dropped silently
    }

    #[test]
    fn test_favorite_priority_ordering() {
        let mut cache = AudioCache::new();
        cache.update_user_info("alice", "Alice", false, false); // regular
        cache.update_user_info("bob", "Bob", true, false);       // favorite

        // Create utterances manually
        let u_alice = Utterance {
            sender_id: "alice".into(),
            sender_name: "Alice".into(),
            is_favorite: false,
            started_at: 1000,
            ended_at: 1100,
            frames: vec![CachedFrame {
                sender_id: "alice".into(),
                timestamp: 1000,
                samples: vec![100i16; FRAME_SIZE],
                received_at: Instant::now(),
            }],
            fully_played: false,
        };

        let u_bob = Utterance {
            sender_id: "bob".into(),
            sender_name: "Bob".into(),
            is_favorite: true,
            started_at: 1050, // Started after Alice
            ended_at: 1150,
            frames: vec![CachedFrame {
                sender_id: "bob".into(),
                timestamp: 1050,
                samples: vec![200i16; FRAME_SIZE],
                received_at: Instant::now(),
            }],
            fully_played: false,
        };

        // Insert non-fav first, then fav
        cache.insert_prioritized(u_alice);
        cache.insert_prioritized(u_bob);

        // Bob (favorite) should be first despite arriving later
        assert_eq!(cache.playback_queue[0].sender_id, "bob");
        assert_eq!(cache.playback_queue[1].sender_id, "alice");
    }

    #[test]
    fn test_playback_drains_queue() {
        let mut cache = AudioCache::new();
        cache.mode = CacheMode::Queue;

        let frames_a: Vec<CachedFrame> = (0..3).map(|i| CachedFrame {
            sender_id: "alice".into(),
            timestamp: 1000 + i * 20,
            samples: vec![100i16; FRAME_SIZE],
            received_at: Instant::now(),
        }).collect();

        let frames_b: Vec<CachedFrame> = (0..2).map(|i| CachedFrame {
            sender_id: "bob".into(),
            timestamp: 2000 + i * 20,
            samples: vec![200i16; FRAME_SIZE],
            received_at: Instant::now(),
        }).collect();

        cache.playback_queue.push_back(Utterance {
            sender_id: "alice".into(),
            sender_name: "Alice".into(),
            is_favorite: false,
            started_at: 1000,
            ended_at: 1040,
            frames: frames_a,
            fully_played: false,
        });

        cache.playback_queue.push_back(Utterance {
            sender_id: "bob".into(),
            sender_name: "Bob".into(),
            is_favorite: false,
            started_at: 2000,
            ended_at: 2020,
            frames: frames_b,
            fully_played: false,
        });

        // Play through all of Alice's frames
        let mut played_alice = 0;
        while let Some((id, _)) = cache.next_playback_frame() {
            if id == "alice" { played_alice += 1; } else { break; }
        }
        // We get 3 alice frames, then the first call after that gives bob
        // Actually: first call starts alice utterance (frame 0), then 1, 2
        // Then next call finishes alice, starts bob (frame 0)
        // So we need to collect all
        assert!(played_alice >= 3);

        // Continue getting bob's frames
        let mut played_bob = 1; // We already got one bob frame from the break above
        while let Some((id, _)) = cache.next_playback_frame() {
            if id == "bob" { played_bob += 1; }
        }
        assert_eq!(played_bob, 2);

        // Queue empty now
        assert!(cache.next_playback_frame().is_none());
        assert_eq!(cache.history.len(), 2); // Both moved to history
    }

    #[test]
    fn test_skip_current() {
        let mut cache = AudioCache::new();
        cache.mode = CacheMode::Queue;

        let frames: Vec<CachedFrame> = (0..10).map(|i| CachedFrame {
            sender_id: "alice".into(),
            timestamp: 1000 + i * 20,
            samples: vec![100i16; FRAME_SIZE],
            received_at: Instant::now(),
        }).collect();

        cache.playback_queue.push_back(Utterance {
            sender_id: "alice".into(),
            sender_name: "Alice".into(),
            is_favorite: false,
            started_at: 1000,
            ended_at: 1180,
            frames,
            fully_played: false,
        });

        // Start playing
        cache.next_playback_frame();
        // Skip after 1 frame
        cache.skip_current();
        assert!(cache.now_playing.is_none());
        // History should NOT contain skipped utterance
        assert_eq!(cache.history.len(), 0);
    }

    #[test]
    fn test_status_json() {
        let cache = AudioCache::new();
        let json = cache.status_json();
        assert!(json.contains("\"mode\":\"Live\""));
        assert!(json.contains("\"queued_utterances\":0"));
    }

    #[test]
    fn test_clear_resets_everything() {
        let mut cache = AudioCache::new();
        cache.mode = CacheMode::Queue;
        cache.active_buffers.insert("test".into(), SpeakerBuffer::new("test"));
        cache.clear();

        assert_eq!(cache.mode(), CacheMode::Live);
        assert!(cache.active_buffers.is_empty());
        assert!(cache.playback_queue.is_empty());
        assert!(cache.now_playing.is_none());
    }

    #[test]
    fn test_max_speakers_limit() {
        let mut cache = AudioCache::new();

        // Fill up to MAX_CACHED_SPEAKERS
        for i in 0..MAX_CACHED_SPEAKERS {
            let id = format!("speaker_{}", i);
            cache.ingest_frame(&id, 1000, vec![1i16; FRAME_SIZE]);
        }
        assert_eq!(cache.active_buffers.len(), MAX_CACHED_SPEAKERS);

        // One more should be dropped
        let result = cache.ingest_frame("overflow_speaker", 1000, vec![1i16; FRAME_SIZE]);
        assert!(result.is_none());
        assert_eq!(cache.active_buffers.len(), MAX_CACHED_SPEAKERS);
    }
}
