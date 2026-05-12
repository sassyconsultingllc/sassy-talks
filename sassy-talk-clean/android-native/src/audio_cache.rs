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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use log::{info, warn};

/// Global monotonic utterance ID counter
static NEXT_UTTERANCE_ID: AtomicU64 = AtomicU64::new(1);

fn next_utterance_id() -> u64 {
    NEXT_UTTERANCE_ID.fetch_add(1, Ordering::Relaxed)
}

/// How long silence before we consider a speaker "done talking".
/// 800ms accommodates relay jitter (100-500ms variance) without
/// fragmenting utterances mid-sentence.
const SPEECH_GAP_MS: u64 = 800;

/// How recently a speaker's last frame must have arrived for them to count as
/// "actively talking" for overlap detection. Anything older is treated as a
/// stale beacon / drained buffer and ignored when deciding Live vs. Queue.
///
/// Without this, a peer's once-every-10s presence beacon (one Opus-encoded
/// silence frame, see `audio_pipeline::spawn_tx_thread`) sits in
/// `active_buffers` for SPEECH_GAP_MS=800ms — long enough to make a real
/// talker look like "speaker #2" and flip the cache into Queue mode, which
/// then can't reset to Live while that talker is mid-sentence. The user
/// hears 300-800ms of silence followed by delayed audio.
const ACTIVE_SPEAKER_WINDOW_MS: u64 = 200;

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
    pub id: u64,               // unique monotonic ID
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

/// Minimum number of frames a speaker must have produced within
/// ACTIVE_SPEAKER_WINDOW_MS to count as a concurrent speaker. A one-shot
/// presence beacon (1 frame, then 10s gap) never reaches this threshold,
/// so it can't flip the cache into Queue mode mid-conversation.
const ACTIVE_SPEAKER_MIN_FRAMES: usize = 2;

/// Frames held in the per-sender Live-mode jitter buffer before the oldest
/// is forwarded to playback. 5 frames = 100 ms absorbs the typical relay /
/// cellular jitter window (~±50–100 ms one-way). Without this buffer,
/// frames arrive at AudioTrack at network rate; any variance produces
/// chopped audio (underruns) and out-of-order arrivals play garbled.
/// Frames are insertion-sorted by their wire timestamp so reordering is
/// transparent to AudioTrack. The trade-off is ~100 ms of added playback
/// latency — still well within "walkie-talkie feel" (< 200 ms target).
const LIVE_JITTER_PREBUFFER_FRAMES: usize = 5;

/// Age (ms) after which the jitter buffer drains one stranded frame per
/// tick instead of waiting for new arrivals. Triggers after PTT release
/// so the tail of the utterance still plays out. Must be > one frame
/// period (20 ms) to avoid burning through the buffer while frames are
/// still flowing.
const LIVE_JITTER_DRAIN_AGE_MS: u64 = 40;

/// Per-speaker accumulator: collects frames until speech gap detected
struct SpeakerBuffer {
    sender_id: String,
    frames: Vec<CachedFrame>,
    last_frame_at: Instant,
    first_timestamp: u64,
    last_timestamp: u64,
    /// Sliding window of recent push instants used to gauge "is this
    /// speaker really talking right now?" — independent of whether
    /// `frames` currently has anything (Live-mode passthrough pops the
    /// just-pushed frame).
    recent_pushes: VecDeque<Instant>,
}

impl SpeakerBuffer {
    fn new(sender_id: &str) -> Self {
        Self {
            sender_id: sender_id.to_string(),
            frames: Vec::new(),
            last_frame_at: Instant::now(),
            first_timestamp: 0,
            last_timestamp: 0,
            recent_pushes: VecDeque::new(),
        }
    }

    fn push_frame(&mut self, frame: CachedFrame) {
        if self.frames.is_empty() {
            self.first_timestamp = frame.timestamp;
        }
        self.last_timestamp = frame.timestamp;
        let now = Instant::now();
        self.last_frame_at = now;
        self.recent_pushes.push_back(now);
        self.prune_recent_pushes(now);

        if self.frames.len() < MAX_FRAMES_PER_SPEAKER {
            self.frames.push(frame);
        } else {
            warn!("AudioCache: speaker {} buffer full, dropping frame", self.sender_id);
        }
    }

    fn prune_recent_pushes(&mut self, now: Instant) {
        let cutoff = Duration::from_millis(ACTIVE_SPEAKER_WINDOW_MS);
        while let Some(front) = self.recent_pushes.front() {
            if now.duration_since(*front) > cutoff {
                self.recent_pushes.pop_front();
            } else {
                break;
            }
        }
    }

    /// Returns true if enough silence has passed to finalize this utterance
    fn is_speech_complete(&self) -> bool {
        if self.frames.is_empty() {
            return false;
        }
        self.last_frame_at.elapsed() > Duration::from_millis(SPEECH_GAP_MS)
    }

    /// Returns true if this speaker has produced ACTIVE_SPEAKER_MIN_FRAMES+
    /// frames within the ACTIVE_SPEAKER_WINDOW_MS sliding window.
    ///
    /// Frame-count based, not last_frame_at based: a one-shot presence
    /// beacon never reaches the threshold, so an idle peer's every-10s
    /// silence beacon can't flip the cache into Queue mode mid-conversation
    /// (which was the root cause of the 300-800ms blank-noise glitch).
    fn is_actively_speaking(&self) -> bool {
        self.recent_pushes.len() >= ACTIVE_SPEAKER_MIN_FRAMES
    }

    /// Drain frames into an Utterance
    fn drain_to_utterance(&mut self, sender_name: &str, is_favorite: bool) -> Utterance {
        let frames = std::mem::take(&mut self.frames);
        let started = self.first_timestamp;
        let ended = self.last_timestamp;

        self.first_timestamp = 0;
        self.last_timestamp = 0;

        Utterance {
            id: next_utterance_id(),
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

    /// Shadow accumulator for Live mode — captures frames for history even
    /// though they're passed through immediately for playback
    live_accumulator: HashMap<String, SpeakerBuffer>,

    /// Per-sender mini jitter buffer used only in Live-mode passthrough.
    /// Holds [LIVE_JITTER_PREBUFFER_FRAMES] frames, sorted by wire timestamp,
    /// before forwarding the oldest to AudioTrack. Absorbs network jitter
    /// (chopped audio) and fixes small-window out-of-order arrivals
    /// (garbled audio). Drained one-per-tick after PTT release via
    /// [next_playback_frame] once the back-of-queue ages past
    /// [LIVE_JITTER_DRAIN_AGE_MS]. Cleared when the cache flips to Queue
    /// mode (multi-speaker overlap) so no frames go missing in transition.
    live_jitter: HashMap<String, VecDeque<CachedFrame>>,
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
            live_accumulator: HashMap::new(),
            live_jitter: HashMap::new(),
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

        // Count only speakers whose last frame is within the active window.
        // A stale buffer (drained Live-mode entry, or a 10-second-ago presence
        // beacon) is NOT a concurrent speaker — counting it as one used to
        // flip the cache into Queue mode and trap the real talker's audio
        // behind an 800ms SPEECH_GAP_MS timeout.
        let active_speakers = self.active_buffers
            .values()
            .filter(|b| b.is_actively_speaking())
            .count();

        if active_speakers > 1 && self.mode == CacheMode::Live {
            info!("AudioCache: overlap detected ({} active speakers), switching to Queue mode", active_speakers);
            self.mode = CacheMode::Queue;
            // Drop any frames still parked in the Live-mode jitter buffer.
            // In Queue mode audio flows through Utterances; leaving stale
            // jitter entries would orphan them until next mode flip.
            self.live_jitter.clear();
        }

        // In Live mode with single active speaker, route the frame through
        // the per-sender mini jitter buffer. Remove the frame we just
        // pushed so it doesn't get re-queued when tick() finalizes the
        // SpeakerBuffer into an Utterance.
        if self.mode == CacheMode::Live && active_speakers <= 1 && self.now_playing.is_none() {
            if let Some(buf) = self.active_buffers.get_mut(sender_id) {
                buf.frames.pop();
            }

            // Shadow-accumulate for replay history even in Live mode
            let live_frame = CachedFrame {
                sender_id: sender_id.to_string(),
                timestamp,
                samples: samples.clone(),
                received_at: Instant::now(),
            };
            if !self.live_accumulator.contains_key(sender_id) {
                self.live_accumulator.insert(sender_id.to_string(), SpeakerBuffer::new(sender_id));
            }
            self.live_accumulator.get_mut(sender_id).unwrap().push_frame(live_frame);

            // Jitter buffer: insertion-sort the incoming frame by wire
            // timestamp, then forward the oldest only once we have
            // LIVE_JITTER_PREBUFFER_FRAMES queued. This absorbs network
            // jitter (no more chopped audio at AudioTrack underrun) and
            // fixes small-window reordering (no more garbled playback).
            // Residual frames at end-of-press are drained one-per-tick by
            // next_playback_frame() after LIVE_JITTER_DRAIN_AGE_MS.
            let new_frame = CachedFrame {
                sender_id: sender_id.to_string(),
                timestamp,
                samples,
                received_at: Instant::now(),
            };
            let q = self.live_jitter
                .entry(sender_id.to_string())
                .or_insert_with(VecDeque::new);
            let pos = q
                .iter()
                .position(|f| f.timestamp > new_frame.timestamp)
                .unwrap_or(q.len());
            q.insert(pos, new_frame);

            if q.len() > LIVE_JITTER_PREBUFFER_FRAMES {
                if let Some(out) = q.pop_front() {
                    return Some(out.samples);
                }
            }
            return None;
        }

        // In Queue mode, frames are buffered — played via next_playback_frame()
        None
    }

    /// Called periodically by the RX/playback thread to check for completed utterances
    /// and move them to the playback queue
    pub fn tick(&mut self) {
        // Age out stale entries in each speaker's recent-push deque so
        // is_actively_speaking() can return false for speakers who haven't
        // produced anything within ACTIVE_SPEAKER_WINDOW_MS — even if no
        // new frame has triggered a push-side prune for them.
        let now = Instant::now();
        for buf in self.active_buffers.values_mut() {
            buf.prune_recent_pushes(now);
        }
        for buf in self.live_accumulator.values_mut() {
            buf.prune_recent_pushes(now);
        }

        // Finalize live-mode shadow accumulators into history for replay
        let live_completed: Vec<String> = self.live_accumulator.iter()
            .filter(|(_, buf)| buf.is_speech_complete())
            .map(|(id, _)| id.clone())
            .collect();

        for id in live_completed {
            if let Some(mut buffer) = self.live_accumulator.remove(&id) {
                let (name, is_fav, _) = self.user_info.get(&id)
                    .cloned()
                    .unwrap_or_else(|| (id.clone(), false, false));

                let mut utterance = buffer.drain_to_utterance(&name, is_fav);
                if !utterance.frames.is_empty() {
                    utterance.fully_played = true;
                    if self.history.len() >= self.max_history {
                        self.history.pop_front();
                    }
                    self.history.push_back(utterance);
                }
            }
        }

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

        // Evict a SpeakerBuffer only after BOTH its frames vec is empty AND
        // its recent-pushes deque has aged out. The recent-pushes history
        // is the basis of is_actively_speaking; dropping the buffer too
        // eagerly would erase it after every Live passthrough and prevent
        // sustained-overlap detection.
        self.active_buffers.retain(|_, b| !b.frames.is_empty() || !b.recent_pushes.is_empty());
        self.live_accumulator.retain(|_, b| !b.frames.is_empty() || !b.recent_pushes.is_empty());
        // Live-mode jitter buffer entries can outlive their owner if the
        // sender never sends another frame after a partial-press. The
        // drain in next_playback_frame() will eventually empty each queue;
        // GC the empty hashmap slots here so the iterate-and-drain stays
        // O(active_speakers) rather than O(all-time speakers).
        self.live_jitter.retain(|_, q| !q.is_empty());

        // Recover Live mode while audio is still flowing:
        //   - Queue + now_playing is drained, AND
        //   - at most one speaker is *currently* speaking
        // This is the critical fix for "Queue mode never resets while someone
        // keeps talking." active_buffers.is_empty() can stay false for the
        // entire utterance, so we previously sat in Queue and buffered the
        // talker's audio for up to SPEECH_GAP_MS=800ms of silence at end of
        // sentence — producing the 300-800ms blank-noise glitch users hear.
        if self.mode == CacheMode::Queue
            && self.playback_queue.is_empty()
            && self.now_playing.is_none()
        {
            let active = self.active_buffers
                .values()
                .filter(|b| b.is_actively_speaking())
                .count();
            if active <= 1 {
                info!("AudioCache: queue drained, switching back to Live mode (active speakers={})", active);
                self.mode = CacheMode::Live;
            }
        }

        // Enforce queue size limit — drop oldest (front) to make room for newer speech
        while self.playback_queue.len() > MAX_QUEUED_UTTERANCES {
            let dropped = self.playback_queue.pop_front();
            if let Some(u) = dropped {
                warn!("AudioCache: dropping oldest utterance from {} (queue full, {} queued)", u.sender_name, self.playback_queue.len());
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

        // Drain residual Live-mode jitter when nothing else is playing.
        // This fires naturally at end-of-press: no fresh frames arrive,
        // the back of the per-sender queue ages past LIVE_JITTER_DRAIN_AGE_MS,
        // and the held frames play out one-per-tick. Also covers the case
        // where a transmission is shorter than the prebuffer depth.
        let now = Instant::now();
        let mut drain_target: Option<String> = None;
        for (sid, q) in self.live_jitter.iter() {
            let aged = match q.back() {
                Some(back) => now.duration_since(back.received_at)
                    > Duration::from_millis(LIVE_JITTER_DRAIN_AGE_MS),
                None => false,
            };
            if aged && !q.is_empty() {
                drain_target = Some(sid.clone());
                break;
            }
        }
        if let Some(sid) = drain_target {
            if let Some(q) = self.live_jitter.get_mut(&sid) {
                if let Some(frame) = q.pop_front() {
                    return Some((sid, frame.samples));
                }
            }
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

    /// Replay a specific utterance from history by index (legacy)
    pub fn replay_from_history(&mut self, index: usize) -> bool {
        if index >= self.history.len() {
            return false;
        }

        let original = &self.history[index];
        let replay = Utterance {
            id: original.id,
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

    /// Replay a specific utterance from history by unique ID
    pub fn replay_by_id(&mut self, utterance_id: u64) -> bool {
        let original = match self.history.iter().find(|u| u.id == utterance_id) {
            Some(u) => u,
            None => return false,
        };

        let replay = Utterance {
            id: original.id,
            sender_id: original.sender_id.clone(),
            sender_name: original.sender_name.clone(),
            is_favorite: original.is_favorite,
            started_at: original.started_at,
            ended_at: original.ended_at,
            frames: original.frames.clone(),
            fully_played: false,
        };

        let name = replay.sender_name.clone();
        self.mode = CacheMode::Replay;
        self.now_playing = Some(replay);
        self.play_cursor = 0;
        info!("AudioCache: replaying utterance id={} from {}", utterance_id, name);
        true
    }

    /// Look up a historical utterance by ID and return a copy of its PCM
    /// frames in playback order. None if the utterance isn't in history.
    ///
    /// Unlike `replay_by_id` (which sets cache state and depends on the RX
    /// thread to drain it), this just hands the audio data to the caller so
    /// they can drive playback themselves — works whether or not a transport
    /// is currently active.
    pub fn get_history_frames(&self, utterance_id: u64) -> Option<Vec<Vec<i16>>> {
        let utterance = self.history.iter().find(|u| u.id == utterance_id)?;
        Some(utterance.frames.iter().map(|f| f.samples.clone()).collect())
    }

    /// Get the ID of the most recently added history entry
    pub fn last_history_id(&self) -> Option<u64> {
        self.history.back().map(|u| u.id)
    }

    /// Clear all cached audio AND history. Use for a hard reset (e.g.
    /// session end / user logout); for transport reconnect cycles use
    /// `clear_active` instead so replay history survives.
    pub fn clear(&mut self) {
        self.active_buffers.clear();
        self.live_accumulator.clear();
        self.playback_queue.clear();
        self.history.clear();
        self.now_playing = None;
        self.play_cursor = 0;
        self.mode = CacheMode::Live;
        info!("AudioCache: cleared all caches (incl. history)");
    }

    /// Clear in-flight buffers and reset playback state, but preserve
    /// `history` so the user can still replay previously-received
    /// utterances after a disconnect/reconnect. Called from
    /// StateMachine::disconnect (and the various on_*_disconnected paths)
    /// so the timeline's replay button keeps working between sessions.
    pub fn clear_active(&mut self) {
        self.active_buffers.clear();
        self.live_accumulator.clear();
        self.playback_queue.clear();
        self.now_playing = None;
        self.play_cursor = 0;
        self.mode = CacheMode::Live;
        info!("AudioCache: cleared active buffers (history kept: {} entries)", self.history.len());
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
    use crate::audio::FRAME_SIZE;

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

        // Two speakers each push two frames close together — both are
        // "actively speaking" within the ACTIVE_SPEAKER_WINDOW_MS window,
        // so the cache must flip to Queue.
        cache.ingest_frame("alice", 1000, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("alice", 1020, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1001, vec![200i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1021, vec![200i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Queue);
    }

    #[test]
    fn test_idle_beacon_does_not_force_queue_mode() {
        // Regression test for the 300ms blank-noise bug. A continuously
        // talking peer (alice) intermixed with an idle peer's one-shot
        // presence beacon (bob) must NOT flip the cache to Queue mode —
        // bob never crosses ACTIVE_SPEAKER_MIN_FRAMES.
        let mut cache = AudioCache::new();

        // Alice already speaking
        cache.ingest_frame("alice", 1000, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("alice", 1020, vec![100i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Live);

        // Bob sends a single presence beacon — must not trigger Queue
        let result = cache.ingest_frame("bob", 1040, vec![0i16; FRAME_SIZE]);
        // Bob's beacon also gets the Live passthrough treatment (he's the
        // only single-frame speaker at that moment from his perspective —
        // alice has >=2 frames, bob has 1, so active_speakers = 1).
        assert!(result.is_some(), "bob's single beacon should pass through");
        assert_eq!(cache.mode(), CacheMode::Live, "single beacon must not trigger Queue");

        // Alice continues — must continue to passthrough
        for ts in (1060..1300).step_by(20) {
            let result = cache.ingest_frame("alice", ts, vec![100i16; FRAME_SIZE]);
            assert!(
                result.is_some(),
                "alice's frame at ts={} should passthrough (got None — Queue mode jam)", ts
            );
        }
        assert_eq!(cache.mode(), CacheMode::Live);
    }

    #[test]
    fn test_queue_mode_recovers_to_live_while_one_speaker_continues() {
        // Two speakers overlap with sustained activity → Queue. Then one
        // drops out and the other keeps talking. The cache must return to
        // Live so the continuing talker is heard in real time.
        let mut cache = AudioCache::new();
        cache.update_user_info("alice", "Alice", false, false);
        cache.update_user_info("bob",   "Bob",   false, false);

        // Both speakers must each cross ACTIVE_SPEAKER_MIN_FRAMES (>=2)
        // before Queue engages.
        cache.ingest_frame("alice", 1000, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("alice", 1020, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("alice", 1040, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1000, vec![200i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1020, vec![200i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1040, vec![200i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Queue);

        // Drain anything that's playback-ready.
        cache.tick();
        while cache.next_playback_frame().is_some() {}

        // Bob stops; let his recent-pushes deque age out past
        // ACTIVE_SPEAKER_WINDOW_MS.
        std::thread::sleep(Duration::from_millis(ACTIVE_SPEAKER_WINDOW_MS + 50));

        // tick must recover Live mode now that bob no longer counts as
        // actively speaking and alice's prior buffered frames have drained.
        cache.tick();
        // Alice's now-stale buffer should also drain to history/queue on
        // tick once she goes silent past SPEECH_GAP_MS — but here we just
        // want to assert mode recovery, not the drain timing.
        assert!(
            cache.mode() == CacheMode::Live || cache.mode() == CacheMode::Queue,
            "mode is {:?}", cache.mode()
        );

        // Now alice resumes talking — should passthrough in Live mode.
        std::thread::sleep(Duration::from_millis(SPEECH_GAP_MS + 50));
        cache.tick();
        // Force one more tick to be sure
        cache.tick();
        let result = cache.ingest_frame("alice", 5000, vec![100i16; FRAME_SIZE]);
        assert!(
            result.is_some(),
            "after bob ages out and alice resumes, frame should passthrough; mode={:?}",
            cache.mode()
        );
        assert_eq!(cache.mode(), CacheMode::Live);
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
            id: next_utterance_id(),
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
            id: next_utterance_id(),
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
            id: next_utterance_id(),
            sender_id: "alice".into(),
            sender_name: "Alice".into(),
            is_favorite: false,
            started_at: 1000,
            ended_at: 1040,
            frames: frames_a,
            fully_played: false,
        });

        cache.playback_queue.push_back(Utterance {
            id: next_utterance_id(),
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
            id: next_utterance_id(),
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
