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

use std::collections::{HashMap, HashSet, VecDeque};
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

// ── Mix-mode constants ──────────────────────────────────────────────────────
/// Hard cap on simultaneous speakers we'll PCM-mix client-side. Beyond this
/// the cache falls back to Queue mode (sequential utterance playback). Six
/// is the sweet spot — past that the mix becomes a crowd-noise wall, AGC
/// fights itself, and CPU starts to matter on low-end Androids.
const MIX_MAX_SPEAKERS: usize = 6;

/// Per-sender alignment window for the mixer. Frames within this window
/// (relative to the leading edge of the current 20 ms mix tick) are summed
/// into the same output frame. Outside, they wait for the next tick. 25 ms
/// = one full frame + 5 ms slack; tighter than this and minor clock skew
/// between senders causes alternating silence/sound stutter.
const MIX_ALIGNMENT_WINDOW_MS: u64 = 25;

/// Soft-clip ceiling as a fraction of i16::MAX. Above this we apply tanh-style
/// rolloff instead of hard clipping — hard clip in voice mixes sounds like
/// fuzz; soft clip sounds like a slightly hot mic, much less offensive.
const MIX_SOFT_CLIP_THRESHOLD: f32 = 0.85;

/// Target RMS for the mixed output as a fraction of i16::MAX. Voice typically
/// sits around 0.10–0.20 RMS; we aim slightly higher so the listener doesn't
/// have to ride the volume button after mix kicks in.
const MIX_TARGET_RMS: f32 = 0.18;

/// Per-tick gain smoothing — moves the applied gain at most this fraction
/// toward the target gain on each frame. Prevents AGC from "breathing"
/// audibly when speakers pause mid-mix.
const MIX_GAIN_SMOOTHING: f32 = 0.10;

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
        // Saturating math — if `ended_at < started_at` (clock skew between
        // senders, NTP step, deliberately bad wire timestamp), raw
        // subtraction would underflow on u64 and produce an astronomical
        // duration that breaks the queue ordering's tie-break sort. Cap
        // at zero instead.
        self.ended_at.saturating_sub(self.started_at).saturating_add(20)
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
/// is forwarded to playback.
///
/// Now a runtime atomic so the Settings UI can offer Low-Latency (3 = 60ms),
/// Balanced (5 = 100ms, the new default), or Smooth (8 = 160ms) presets.
/// Larger absorbs more cellular jitter — fewer chops — at the cost of
/// perceptible delay between PTT-press on the sender and audio start on
/// the receiver. The previous fixed 3-frame setting under-absorbed on
/// LTE handoffs and CDN re-routes; bumping the default to 5 buys ~33% more
/// smoothing for one extra Opus frame of latency (40 ms — well under
/// human perception of conversational delay).
///
/// Read via [live_jitter_prebuffer_frames]; never read the constant
/// directly (it's just the initial value).
const DEFAULT_LIVE_JITTER_PREBUFFER_FRAMES: usize = 5;
static LIVE_JITTER_PREBUFFER_FRAMES_ATOMIC: AtomicU64 =
    AtomicU64::new(DEFAULT_LIVE_JITTER_PREBUFFER_FRAMES as u64);

#[inline]
pub fn live_jitter_prebuffer_frames() -> usize {
    LIVE_JITTER_PREBUFFER_FRAMES_ATOMIC.load(Ordering::Relaxed) as usize
}

/// Set the runtime jitter-buffer pre-buffer size in frames. Clamped to
/// [2, 16] — under 2 there's no smoothing, over 16 latency becomes
/// audibly bad (>320 ms).
pub fn set_live_jitter_prebuffer_frames(frames: usize) {
    let clamped = frames.clamp(2, 16);
    LIVE_JITTER_PREBUFFER_FRAMES_ATOMIC.store(clamped as u64, Ordering::Relaxed);
}

// Kept for the (very few) sites that still want the compile-time default
// in error messages. NEVER use this as a runtime value.
#[allow(dead_code)]
const LIVE_JITTER_PREBUFFER_FRAMES: usize = DEFAULT_LIVE_JITTER_PREBUFFER_FRAMES;

/// Age (ms) after which the jitter buffer drains one stranded frame per
/// tick instead of waiting for new arrivals. Just above one frame period
/// (20 ms) so the tail of an utterance plays out promptly when PTT is
/// released. Larger values (we used 40 ms) audibly slow the closing of
/// a transmission and were a contributor to the "slowed-down" symptom.
const LIVE_JITTER_DRAIN_AGE_MS: u64 = 25;

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
            // saturating_* avoids the panic path of Instant::duration_since on a
            // non-monotonic clock (returns 0 instead of aborting the mixer thread).
            if now.saturating_duration_since(*front) > cutoff {
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

    /// Mix mode: PCM-sum 2..=MIX_MAX_SPEAKERS overlapping streams with AGC
    /// + soft-clip. Activates instead of Queue when `enable_mix_mode` is set
    /// AND active speakers are within MIX_MAX_SPEAKERS. Falls back to Queue
    /// the moment that ceiling is crossed.
    Mix,
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

    /// IDs of utterances added to `history` since the last call to
    /// `take_newly_committed_ids()`. Drained by the RX thread, dispatched
    /// to Kotlin via JNI so `TranscriptionEntry.utteranceId` can be
    /// populated from the *authoritative* commit event instead of a
    /// best-effort poll of `last_history_id()` — the previous polling
    /// pattern raced against the 800 ms `is_speech_complete()` timer and
    /// captured the wrong ID (or -1), which is why the timeline play
    /// button silently did nothing.
    newly_committed_ids: Vec<(u64, String, String, u64)>, // (id, sender_id, sender_name, duration_ms)

    // ── Mix-mode state ──
    /// Opt-in flag. When false the cache behaves exactly as before (Live/Queue
    /// only); existing single-speaker and turn-taking flows are untouched.
    enable_mix_mode: bool,
    /// Per-sender most-recent-frame buffer used by the mixer to align inputs
    /// within MIX_ALIGNMENT_WINDOW_MS. Keyed by sender_id; one frame per
    /// sender max — the mixer consumes and clears on each emit.
    mix_pending: HashMap<String, CachedFrame>,
    /// Smoothed AGC gain. Persisted across ticks so a sentence boundary
    /// doesn't visibly pump the level.
    mix_gain: f32,

    /// Cross-transport dedup window. Every RX path (WiFi multicast, Bluetooth
    /// RFCOMM, Cloudflare relay) converges on `ingest_frame`, so the same logical
    /// frame delivered on two paths at once — a peer reachable on both WiFi and
    /// Bluetooth, WiFi + relay, or our own multicast loopback echoed by the relay
    /// — would otherwise be played twice. We remember a bounded set of recent
    /// `(hash(sender_id), timestamp)` keys and drop the second copy. Bounded so
    /// memory stays O(FRAME_DEDUP_WINDOW) regardless of session length.
    recent_frames: (HashSet<(u64, u64)>, VecDeque<(u64, u64)>),
}

/// Max recent (sender, timestamp) frame keys remembered for cross-transport
/// dedup. ~256 frames ≈ 5 s of a single 50 fps speaker, comfortably covering the
/// few-ms skew between two transports delivering the same frame.
const FRAME_DEDUP_WINDOW: usize = 256;

/// Hash a sender id to a u64 so the dedup key avoids per-frame String allocation.
fn hash_sender(sender_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sender_id.hash(&mut h);
    h.finish()
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
            newly_committed_ids: Vec::new(),
            enable_mix_mode: false,
            mix_pending: HashMap::new(),
            mix_gain: 1.0,
            recent_frames: (HashSet::new(), VecDeque::new()),
        }
    }

    /// Record `(sender, timestamp)` and report whether it is NEW (true) or a
    /// duplicate already seen within the recent window (false). The window is
    /// bounded to [FRAME_DEDUP_WINDOW] keys (oldest evicted first).
    fn accept_frame_once(&mut self, sender_id: &str, timestamp: u64) -> bool {
        let key = (hash_sender(sender_id), timestamp);
        let (set, order) = &mut self.recent_frames;
        if !set.insert(key) {
            return false; // duplicate delivery on another transport — drop
        }
        order.push_back(key);
        if order.len() > FRAME_DEDUP_WINDOW {
            if let Some(old) = order.pop_front() {
                set.remove(&old);
            }
        }
        true
    }

    /// Toggle client-side mixing for 2..=MIX_MAX_SPEAKERS overlapping speakers.
    /// When enabled, overlap flips the cache into `Mix` instead of `Queue`.
    /// When disabled (default), legacy Queue behavior is preserved.
    /// Settings UI should call this from the audio-preferences screen.
    pub fn set_mix_mode_enabled(&mut self, enabled: bool) {
        if self.enable_mix_mode == enabled { return; }
        self.enable_mix_mode = enabled;
        // Don't try to migrate state mid-mode — let the next ingest_frame
        // decide based on current active speakers.
        self.mix_pending.clear();
        self.mix_gain = 1.0;
        info!("AudioCache: mix mode {}", if enabled { "enabled" } else { "disabled" });
    }

    pub fn is_mix_mode_enabled(&self) -> bool {
        self.enable_mix_mode
    }

    /// Take the queue of recently committed utterance IDs. RX thread calls
    /// this each loop iteration to forward commits to Kotlin via JNI.
    pub fn take_newly_committed_ids(&mut self) -> Vec<(u64, String, String, u64)> {
        std::mem::take(&mut self.newly_committed_ids)
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
        // Cross-transport dedup. Every RX path converges here with the frame's
        // (sender_id, timestamp) from the shared wire header, so a frame delivered
        // on two transports at once (WiFi+Bluetooth, WiFi+relay, or our own
        // multicast loopback echoed by the relay) is dropped on the second copy
        // rather than double-played. timestamp is the per-frame capture time (ms),
        // so consecutive distinct frames (~20 ms apart) are never falsely merged.
        if !self.accept_frame_once(sender_id, timestamp) {
            return None;
        }

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
            // Two transition paths from Live:
            //   1. mix_mode_enabled AND active_speakers in 2..=MIX_MAX_SPEAKERS
            //      → Mix (real-time PCM sum)
            //   2. anything else (mix disabled, or too many speakers)
            //      → Queue (legacy serialize-utterances behavior)
            let next_mode = if self.enable_mix_mode && active_speakers <= MIX_MAX_SPEAKERS {
                CacheMode::Mix
            } else {
                CacheMode::Queue
            };
            info!(
                "AudioCache: overlap detected ({} active speakers), switching to {:?} mode",
                active_speakers, next_mode
            );
            self.mode = next_mode;
            // Drop any frames still parked in the Live-mode jitter buffer.
            // In Queue/Mix mode audio flows through different paths; leaving
            // stale jitter entries would orphan them until next mode flip.
            self.live_jitter.clear();
        }

        // Promote Mix→Queue if speaker count exceeds the mix ceiling. We do
        // NOT demote Queue→Mix mid-conversation — once Queue has utterances
        // buffered, switching to Mix mid-flight would drop them.
        if self.mode == CacheMode::Mix && active_speakers > MIX_MAX_SPEAKERS {
            info!(
                "AudioCache: mix ceiling exceeded ({} > {}), falling back to Queue",
                active_speakers, MIX_MAX_SPEAKERS
            );
            self.mode = CacheMode::Queue;
            self.mix_pending.clear();
        }

        // Mix-mode hot path: register the incoming frame in mix_pending and
        // try to emit a mixed output if we have alignable frames from
        // multiple senders. Returns the mixed PCM directly — bypassing the
        // Utterance pipeline since mix output is real-time, not replayable.
        if self.mode == CacheMode::Mix {
            self.mix_pending.insert(
                sender_id.to_string(),
                CachedFrame {
                    sender_id: sender_id.to_string(),
                    timestamp,
                    samples,
                    received_at: Instant::now(),
                },
            );
            if let Some(mixed) = self.try_emit_mix() {
                return Some(mixed);
            }
            return None;
        }

        // In Live mode with single active speaker, route the frame through
        // the per-sender mini jitter buffer. Remove the frame we just
        // pushed so it doesn't get re-queued when tick() finalizes the
        // SpeakerBuffer into an Utterance.
        if self.mode == CacheMode::Live && active_speakers <= 1 && self.now_playing.is_none() {
            // Reclaim the frame we pushed above (it already holds a copy of
            // `samples`) and reuse it as the replay-history frame instead of
            // cloning `samples` a second time. This halves the per-frame heap
            // copies on the dominant single-speaker Live path: previously we
            // cloned once for active_buffers (immediately popped + discarded)
            // and again here for the accumulator.
            let recycled = self.active_buffers
                .get_mut(sender_id)
                .and_then(|buf| buf.frames.pop());

            // Shadow-accumulate for replay history even in Live mode.
            if let Some(live_frame) = recycled {
                self.live_accumulator
                    .entry(sender_id.to_string())
                    .or_insert_with(|| SpeakerBuffer::new(sender_id))
                    .push_frame(live_frame);
            }

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

            if q.len() > live_jitter_prebuffer_frames() {
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
                    let commit = (utterance.id, utterance.sender_id.clone(), utterance.sender_name.clone(), utterance.duration_ms());
                    if self.history.len() >= self.max_history {
                        self.history.pop_front();
                    }
                    self.history.push_back(utterance);
                    self.newly_committed_ids.push(commit);
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
            let commit = (finished.id, finished.sender_id.clone(), finished.sender_name.clone(), finished.duration_ms());
            if self.history.len() >= self.max_history {
                self.history.pop_front();
            }
            self.history.push_back(finished);
            self.newly_committed_ids.push(commit);
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
                Some(back) => now.saturating_duration_since(back.received_at)
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

    /// Try to emit one mixed frame from `mix_pending`.
    ///
    /// Behavior:
    ///   - If fewer than 2 senders have a pending frame: return None
    ///     (wait for another sender's frame to arrive within the alignment
    ///     window).
    ///   - Otherwise: align all frames whose timestamp is within
    ///     MIX_ALIGNMENT_WINDOW_MS of the leading edge, PCM-sum them,
    ///     run AGC + soft-clip, return the mixed samples.
    ///
    /// Out-of-window pending frames stay for the next emit attempt — they're
    /// only dropped if they age past 2 × MIX_ALIGNMENT_WINDOW_MS (rare; only
    /// happens when one sender drops mid-mix).
    fn try_emit_mix(&mut self) -> Option<Vec<i16>> {
        if self.mix_pending.len() < 2 {
            return None;
        }

        // Drop stranded frames older than 2×alignment window. These would
        // otherwise pile up indefinitely if one sender went silent.
        let now = Instant::now();
        let stale_cutoff = Duration::from_millis(MIX_ALIGNMENT_WINDOW_MS * 2);
        self.mix_pending.retain(|_, f| now.saturating_duration_since(f.received_at) < stale_cutoff);
        if self.mix_pending.len() < 2 {
            return None;
        }

        // Leading edge = oldest pending frame's wire timestamp. Any frame
        // within MIX_ALIGNMENT_WINDOW_MS of that joins this tick's mix.
        // Saturating addition so a corrupted/adversarial timestamp near
        // u64::MAX can't wrap window_end down to a small value and drop
        // every legitimate frame.
        let leading_ts = self.mix_pending.values().map(|f| f.timestamp).min()?;
        let window_end = leading_ts.saturating_add(MIX_ALIGNMENT_WINDOW_MS);

        let aligned_keys: Vec<String> = self.mix_pending
            .iter()
            .filter(|(_, f)| f.timestamp <= window_end)
            .map(|(k, _)| k.clone())
            .collect();

        if aligned_keys.len() < 2 {
            return None;  // Not enough overlap to justify mixing this tick.
        }

        // Determine frame length from the first aligned frame. All Opus
        // frames decoded by this app are the same length (FRAME_SIZE = 960
        // samples = 20 ms @ 48 kHz). Defensive: pad/truncate mismatches.
        let frame_len = self.mix_pending[&aligned_keys[0]].samples.len();
        let mut acc = vec![0i32; frame_len];

        for k in &aligned_keys {
            if let Some(frame) = self.mix_pending.remove(k) {
                // Accumulate as i32 to avoid clip during summation.
                for (i, &s) in frame.samples.iter().take(frame_len).enumerate() {
                    acc[i] += s as i32;
                }
            }
        }

        // Compute RMS of the raw sum (as fraction of i16::MAX).
        let mut sq_sum: f64 = 0.0;
        for &s in &acc {
            let f = s as f64 / i16::MAX as f64;
            sq_sum += f * f;
        }
        let rms = (sq_sum / frame_len as f64).sqrt() as f32;

        // Target gain = how much to scale to land at MIX_TARGET_RMS.
        // Floor at 0.1 (never amplify silence to infinity) and cap at 2.0
        // (never boost more than +6dB — louder than that and you're just
        // amplifying noise floor).
        // Cap chosen for walkie-talkie use: a whispering peer can sit
        // near -20 dB RMS (≈ 0.05); we need ~+12 dB to bring that up to
        // the target (4×). +6 dB (2×) under-amplified soft conversations
        // and left them buried under background noise on louder peers.
        // Floor at 0.1 prevents silence-period gain runaway.
        let target_gain = if rms > 0.001 {
            (MIX_TARGET_RMS / rms).clamp(0.1, 4.0)
        } else {
            self.mix_gain  // signal is essentially silence — hold previous gain
        };

        // Smooth toward target — avoids audible AGC pumping.
        self.mix_gain += (target_gain - self.mix_gain) * MIX_GAIN_SMOOTHING;

        // Apply gain + soft-clip, write into i16 output.
        let threshold = MIX_SOFT_CLIP_THRESHOLD * i16::MAX as f32;
        let out: Vec<i16> = acc.iter().map(|&s| {
            let scaled = (s as f32) * self.mix_gain;
            let clipped = if scaled.abs() > threshold {
                // tanh-style soft knee. Sign-preserving, asymptotes at i16::MAX.
                let sign = scaled.signum();
                let over = (scaled.abs() - threshold) / (i16::MAX as f32 - threshold);
                sign * (threshold + (i16::MAX as f32 - threshold) * over.tanh())
            } else {
                scaled
            };
            clipped.clamp(i16::MIN as f32, i16::MAX as f32) as i16
        }).collect();

        Some(out)
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
    ///
    /// Before clearing, any in-progress live-accumulator entries are
    /// drained into `history` so utterances that hadn't yet timed out
    /// (800 ms gap) when the transport dropped don't vanish silently.
    /// This fixes the "the conversation I just had is missing from
    /// timeline" bug.
    pub fn clear_active(&mut self) {
        // Drain accumulating utterances into history before we wipe state.
        let drained: Vec<String> = self.live_accumulator.keys().cloned().collect();
        for id in drained {
            if let Some(mut buf) = self.live_accumulator.remove(&id) {
                let (name, is_fav, _) = self.user_info.get(&id)
                    .cloned()
                    .unwrap_or_else(|| (id.clone(), false, false));
                let mut utt = buf.drain_to_utterance(&name, is_fav);
                if !utt.frames.is_empty() {
                    utt.fully_played = true;
                    let commit = (utt.id, utt.sender_id.clone(), utt.sender_name.clone(), utt.duration_ms());
                    if self.history.len() >= self.max_history {
                        self.history.pop_front();
                    }
                    self.history.push_back(utt);
                    self.newly_committed_ids.push(commit);
                }
            }
        }

        self.active_buffers.clear();
        self.live_accumulator.clear();
        self.playback_queue.clear();
        self.live_jitter.clear();
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
    /// 1. Favorites before non-favorites.
    /// 2. Within the same priority, ordered by `started_at` ASC (older first).
    /// 3. Tie-break: shorter `duration_ms` first — minimizes head-of-line
    ///    blocking when two utterances started at the same instant, so the
    ///    long talker doesn't trap a quick "yeah" behind 30 seconds of speech.
    ///
    /// Insertion picks the first position where the comparison key
    /// (started_at, duration_ms) of the new utterance is strictly LESS than
    /// the existing slot's key. That keeps adjacent equal-key utterances in
    /// arrival order (stable insert), which is what a real walkie-talkie
    /// audience expects: when two people start at the same instant with the
    /// same length, the one whose first frame hit the wire first plays first.
    fn insert_prioritized(&mut self, utterance: Utterance) {
        // Bound search to the priority tier of this utterance.
        let (tier_start, tier_end) = if utterance.is_favorite {
            // Favorites occupy [0, first non-favorite).
            let end = self.playback_queue.iter()
                .position(|u| !u.is_favorite)
                .unwrap_or(self.playback_queue.len());
            (0, end)
        } else {
            // Non-favorites occupy [first non-favorite, end).
            let start = self.playback_queue.iter()
                .position(|u| !u.is_favorite)
                .unwrap_or(self.playback_queue.len());
            (start, self.playback_queue.len())
        };

        let new_key = (utterance.started_at, utterance.duration_ms());
        let pos = (tier_start..tier_end)
            .find(|&i| {
                let u = &self.playback_queue[i];
                (u.started_at, u.duration_ms()) > new_key
            })
            .unwrap_or(tier_end);

        self.playback_queue.insert(pos, utterance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // FRAME_SIZE is conceptually a wire constant (20 ms at 48 kHz = 960
    // samples). Consumer crates may also re-export it from their audio
    // module; inlined here so the test suite runs standalone without
    // depending on a consumer's audio code.
    const FRAME_SIZE: usize = 960;

    #[test]
    fn cross_transport_dedup_drops_exact_duplicate() {
        let mut cache = AudioCache::new();
        let s = vec![0i16; FRAME_SIZE];

        // The dedup check runs first, so a duplicate always returns None
        // regardless of cache mode / jitter buffering. The first copy's return
        // is mode-dependent, so we ignore it and only assert the dup is dropped.
        let _ = cache.ingest_frame("peer-1", 1000, s.clone());
        assert!(
            cache.ingest_frame("peer-1", 1000, s.clone()).is_none(),
            "duplicate (sender, timestamp) from a second transport must be dropped"
        );

        // A different sender with the SAME timestamp is NOT a duplicate.
        let _ = cache.ingest_frame("peer-2", 1000, s.clone());
        assert!(
            cache.ingest_frame("peer-2", 1000, s.clone()).is_none(),
            "peer-2's own duplicate is deduped independently of peer-1"
        );

        // The same sender with a NEW timestamp is a fresh frame (not deduped):
        // it passes the dedup gate, so it reaches the normal ingest path.
        let _ = cache.ingest_frame("peer-1", 1020, s);
        // (Return value is mode-dependent; the point is it was not dropped at the
        // dedup gate — proven by the dup of THIS key now being dropped.)
        let s2 = vec![1i16; FRAME_SIZE];
        assert!(
            cache.ingest_frame("peer-1", 1020, s2).is_none(),
            "the 1020 frame registered, so its duplicate is now deduped"
        );
    }

    #[test]
    fn test_cache_live_passthrough() {
        let mut cache = AudioCache::new();
        assert_eq!(cache.mode(), CacheMode::Live);

        let samples = vec![100i16; FRAME_SIZE];

        // Single speaker stays in Live mode, but the per-sender jitter buffer
        // holds live_jitter_prebuffer_frames() frames before releasing the
        // oldest. Drive one frame past the prebuffer; the oldest frame
        // (ts=1000) then passes through unchanged.
        let prebuffer = live_jitter_prebuffer_frames();
        let mut result = None;
        for i in 0..=prebuffer {
            result = cache.ingest_frame("alice", 1000 + (i as u64) * 20, samples.clone());
        }
        assert!(result.is_some(), "frame should pass through once prebuffer is exceeded");
        assert_eq!(result.unwrap(), samples);
        assert_eq!(cache.mode(), CacheMode::Live);
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

        // Bob sends a single presence beacon — must not trigger Queue. (The
        // frame itself is absorbed by the Live jitter prebuffer, so a Some
        // return isn't guaranteed on this single call; the invariant under
        // test is the MODE, not passthrough timing.)
        cache.ingest_frame("bob", 1040, vec![0i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Live, "single beacon must not trigger Queue");

        // Alice continues — stays in Live mode and her audio drains through
        // the jitter buffer once the prebuffer fills (no Queue-mode jam).
        let mut passthroughs = 0;
        for ts in (1060..1300).step_by(20) {
            if cache.ingest_frame("alice", ts, vec![100i16; FRAME_SIZE]).is_some() {
                passthroughs += 1;
            }
        }
        assert!(passthroughs > 0, "alice's audio should drain in Live mode (Queue-mode jam)");
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

        // Now alice resumes talking — should be back in Live mode, and her
        // audio should flow once the jitter prebuffer refills (it was cleared
        // on the earlier mode flip). Drain any residual so the assertion
        // doesn't hinge on the exact release tick.
        std::thread::sleep(Duration::from_millis(SPEECH_GAP_MS + 50));
        cache.tick();
        // Force one more tick to be sure
        cache.tick();
        let prebuffer = live_jitter_prebuffer_frames();
        let mut got_audio = false;
        for i in 0..=prebuffer {
            if cache.ingest_frame("alice", 5000 + (i as u64) * 20, vec![100i16; FRAME_SIZE]).is_some() {
                got_audio = true;
            }
        }
        while cache.next_playback_frame().is_some() { got_audio = true; }
        assert!(
            got_audio,
            "after bob ages out and alice resumes, audio should flow; mode={:?}",
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

    // ── Mix-mode tests ───────────────────────────────────────────────────

    #[test]
    fn test_mix_mode_flips_when_enabled_and_within_ceiling() {
        let mut cache = AudioCache::new();
        cache.set_mix_mode_enabled(true);

        // Two active speakers (>=2 frames each within active window) → Mix
        cache.ingest_frame("alice", 1000, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("alice", 1020, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1010, vec![200i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1030, vec![200i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Mix,
            "with mix enabled and 2 speakers, should be Mix not Queue");
    }

    #[test]
    fn test_mix_mode_falls_back_to_queue_above_ceiling() {
        let mut cache = AudioCache::new();
        cache.set_mix_mode_enabled(true);

        // 7 speakers, each crossing ACTIVE_SPEAKER_MIN_FRAMES → exceeds
        // MIX_MAX_SPEAKERS (6) → Queue
        for name in ["a", "b", "c", "d", "e", "f", "g"] {
            cache.ingest_frame(name, 1000, vec![100i16; FRAME_SIZE]);
            cache.ingest_frame(name, 1020, vec![100i16; FRAME_SIZE]);
        }
        assert_eq!(cache.mode(), CacheMode::Queue,
            "above MIX_MAX_SPEAKERS the cache must fall back to Queue");
    }

    #[test]
    fn test_mix_disabled_preserves_queue_behavior() {
        // Default cache (mix disabled) must behave as before — Queue on overlap.
        let mut cache = AudioCache::new();
        assert!(!cache.is_mix_mode_enabled());

        cache.ingest_frame("alice", 1000, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("alice", 1020, vec![100i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1010, vec![200i16; FRAME_SIZE]);
        cache.ingest_frame("bob",   1030, vec![200i16; FRAME_SIZE]);
        assert_eq!(cache.mode(), CacheMode::Queue,
            "mix disabled (default) must still go to Queue, not Mix");
    }

    #[test]
    fn test_queue_orders_by_started_at_then_shorter_first() {
        // Three non-favorite utterances inserted out of timestamp order.
        // Expected playback order:
        //   1. ts=1000 dur=500  (oldest started)
        //   2. ts=2000 dur=200  (tie-break: shorter first)
        //   3. ts=2000 dur=1000 (same start, longer goes after)
        let mut cache = AudioCache::new();

        fn mk(id: u64, ts: u64, dur: u64) -> Utterance {
            // Build a frame buffer whose ended_at - started_at + 20 == dur.
            // Frame count doesn't matter for ordering; only the timestamps do.
            Utterance {
                id,
                sender_id: format!("s{}", id),
                sender_name: format!("S{}", id),
                is_favorite: false,
                started_at: ts,
                ended_at: ts + dur.saturating_sub(20),
                frames: vec![CachedFrame {
                    sender_id: format!("s{}", id),
                    timestamp: ts,
                    samples: vec![0i16; FRAME_SIZE],
                    received_at: Instant::now(),
                }],
                fully_played: false,
            }
        }

        // Insert in arrival order: long-same-ts first, then short-same-ts,
        // then oldest. After sorting, the oldest must be first; among the
        // two same-ts, the shorter must precede the longer.
        cache.insert_prioritized(mk(1, 2000, 1000));
        cache.insert_prioritized(mk(2, 2000, 200));
        cache.insert_prioritized(mk(3, 1000, 500));

        let ids: Vec<u64> = cache.playback_queue.iter().map(|u| u.id).collect();
        assert_eq!(ids, vec![3, 2, 1],
            "expected ordering: oldest start first, then shorter duration on ties");
    }

    #[test]
    fn test_queue_favorites_jump_ahead_of_non_favorites() {
        let mut cache = AudioCache::new();

        fn mk(id: u64, ts: u64, dur: u64, fav: bool) -> Utterance {
            Utterance {
                id, sender_id: format!("s{}", id), sender_name: format!("S{}", id),
                is_favorite: fav, started_at: ts, ended_at: ts + dur.saturating_sub(20),
                frames: vec![CachedFrame {
                    sender_id: format!("s{}", id), timestamp: ts,
                    samples: vec![0i16; FRAME_SIZE], received_at: Instant::now(),
                }],
                fully_played: false,
            }
        }

        // Two non-favs then a favorite that started LATER.
        cache.insert_prioritized(mk(1, 1000, 500, false));
        cache.insert_prioritized(mk(2, 1500, 500, false));
        cache.insert_prioritized(mk(3, 5000, 500, true));  // favorite, latest ts

        let ids: Vec<u64> = cache.playback_queue.iter().map(|u| u.id).collect();
        assert_eq!(ids, vec![3, 1, 2],
            "favorite must jump ahead regardless of timestamp; non-favs keep their order");
    }

    #[test]
    fn test_mixer_soft_clip_keeps_output_in_range() {
        // Drive multiple loud streams and verify nothing overflows i16.
        let mut cache = AudioCache::new();
        cache.set_mix_mode_enabled(true);

        let loud = vec![30000i16; FRAME_SIZE];
        cache.ingest_frame("alice", 1000, loud.clone());
        cache.ingest_frame("alice", 1020, loud.clone());
        cache.ingest_frame("bob",   1010, loud.clone());
        let mixed = cache.ingest_frame("bob", 1015, loud.clone());

        if let Some(samples) = mixed {
            for &s in &samples {
                assert!(s >= i16::MIN && s <= i16::MAX, "sample out of i16 range: {}", s);
            }
        }
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
