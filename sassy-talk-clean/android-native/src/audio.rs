/// Audio Module - Voice Capture and Playback via JNI
/// 
/// Handles microphone recording (PTT press) and speaker playback (receiving)
/// Uses Android AudioRecord/AudioTrack through JNI bridge

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use log::{info, warn};

use crate::jni_bridge::{AndroidAudioRecord, AndroidAudioTrack};
use crate::audio_effects::AppliedEffects;

/// Audio configuration constants
pub const SAMPLE_RATE: i32 = 48000;  // 48kHz high quality
pub const CHANNEL_CONFIG_MONO: i32 = 16;  // AudioFormat.CHANNEL_IN_MONO
pub const CHANNEL_CONFIG_OUT_MONO: i32 = 4;  // AudioFormat.CHANNEL_OUT_MONO
pub const AUDIO_FORMAT_PCM_16: i32 = 2;  // AudioFormat.ENCODING_PCM_16BIT
pub const FRAME_SIZE: usize = 960;  // 20ms at 48kHz

/// Hard-floor recorder buffer multiplier when no quirks profile applies.
/// Per-device profiles (`device_quirks::Profile.record_buffer_multiplier`)
/// override this. 4× is the safe default; Moto bumps to 6× because the
/// HAL stalls for tens of ms under load.
const RECORDER_BUFFER_FALLBACK: i32 = 4;

/// Hard-floor player buffer multiplier. Same rationale as the recorder.
const PLAYER_BUFFER_FALLBACK: i32 = 4;

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
    /// Hardware effects (AEC/NS/AGC) attached to the recorder. Held in
    /// place so they stay alive for the recorder's lifetime; dropped on
    /// `release` so the driver tears them down.
    effects: Arc<Mutex<Option<AppliedEffects>>>,
    /// Wall-clock timestamp (ms since UNIX_EPOCH) of the last `write_audio`
    /// that touched the AudioTrack. Used by the replay-thread's
    /// `wait_for_playback_idle` loop to detect "live audio is hot, back off".
    /// Zero means "never written".
    last_write_at_ms: Arc<AtomicU64>,

    /// Monotonic write counter — incremented by EVERY successful `write_audio`.
    /// The replay thread uses this (NOT the wall-clock timestamp) to detect
    /// RX intrusion mid-loop: it records its own post-write seq, then before
    /// each next frame re-reads — if the value differs by more than the
    /// expected 1 (its own bump), an RX writer slipped in.
    ///
    /// Why counter not timestamp: two writes within the same millisecond
    /// (common at 25 fps on fast hardware) collapse to the same wall-clock
    /// value, defeating the comparison. A counter is granular per-call.
    /// Wraparound is irrelevant — at 50 fps × 2 producers, u64 wraps in
    /// ~6 billion years.
    write_seq: Arc<AtomicU64>,

    /// Single-owner playback session lock.
    ///
    /// Held for the duration of one continuous playback (one queue utterance
    /// or one replay) so the AudioTrack only ever has ONE producer at a
    /// time. Callers acquire via [acquire_playback]:
    ///   - RX `try_acquire` per frame; on contention, drops the frame.
    ///     Acceptable: short replay interrupts live; the cellular jitter
    ///     buffer absorbs sub-200 ms gaps, longer ones are perceived as a
    ///     deliberate user-initiated scrub.
    ///   - Replay `acquire` and HOLD for its entire frame loop. While held,
    ///     RX defers; once released, RX resumes immediately.
    /// Combined with [last_write_at_ms] this gives the replay thread a way
    /// to wait for live audio to QUIET DOWN (poll the timestamp) BEFORE it
    /// acquires the lock — no thrashing.
    playback_lock: Arc<Mutex<()>>,
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
            effects: Arc::new(Mutex::new(None)),
            last_write_at_ms: Arc::new(AtomicU64::new(0)),
            write_seq: Arc::new(AtomicU64::new(0)),
            playback_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Cheap clone of the monotonic write-sequence counter. Replay uses
    /// this to detect RX intrusion at per-call granularity — see
    /// [write_seq] field doc.
    pub fn write_seq_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.write_seq)
    }

    /// Cheap clone of the last-write atomic so callers can poll the gate
    /// without contending the engine mutex. Used by the replay thread to
    /// detect "live audio is currently flowing — wait it out".
    pub fn write_clock(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.last_write_at_ms)
    }

    /// Cheap clone of the single-owner session lock. Replay acquires this
    /// for its entire frame loop; RX try-acquires per frame and drops on
    /// contention. See [playback_lock] doc for full semantics.
    pub fn playback_lock_handle(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.playback_lock)
    }

    /// Milliseconds since the last `write_audio` succeeded. Returns
    /// `u64::MAX` if nothing has ever been written.
    pub fn last_write_idle_ms(&self) -> u64 {
        playback_idle_ms(&self.last_write_at_ms)
    }

    /// Initialize audio recorder.
    ///
    /// Picks the recording source from the active `DeviceQuirks` profile's
    /// `source_fallback_chain` (defaults to `VOICE_RECOGNITION → MIC → DEFAULT`).
    /// `VOICE_RECOGNITION` bypasses the OEM modem-side processing that mangles
    /// voice on many devices; some OEMs reject it, so we fall back. Buffer
    /// size is `getMinBufferSize × profile.record_buffer_multiplier` (4× by
    /// default, 6× on Moto).
    pub fn init_recorder(&self) -> Result<(), String> {
        info!("Initializing audio recorder");

        let quirks = crate::device_quirks::current();
        let multiplier = quirks.record_buffer_multiplier.max(RECORDER_BUFFER_FALLBACK);

        // getMinBufferSize() — negative returns mean unsupported config.
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

        let alloc_size = buffer_size.checked_mul(multiplier).ok_or_else(|| {
            *self.state.lock().unwrap() = AudioState::Error;
            format!("Recorder buffer size {} would overflow i32 when scaled by {}", buffer_size, multiplier)
        })?;

        info!(
            "Recorder buffer size: {} bytes (x{} = {}); sources to try: {:?}",
            buffer_size, multiplier, alloc_size, quirks.source_fallback_chain
        );

        // Try sources in order until one yields STATE_INITIALIZED.
        let mut last_err = String::from("no sources to try");
        let mut recorder_opt: Option<AndroidAudioRecord> = None;
        let mut chosen_source: i32 = -1;
        for src in &quirks.source_fallback_chain {
            match AndroidAudioRecord::new_with_source(*src, SAMPLE_RATE, CHANNEL_CONFIG_MONO, AUDIO_FORMAT_PCM_16, alloc_size) {
                Ok(r) => {
                    chosen_source = *src;
                    recorder_opt = Some(r);
                    break;
                }
                Err(e) => {
                    warn!("AudioRecord source={} rejected: {}", src, e);
                    last_err = e;
                }
            }
        }
        let recorder = match recorder_opt {
            Some(r) => r,
            None => {
                *self.state.lock().unwrap() = AudioState::Error;
                return Err(format!("All AudioRecord sources rejected — last error: {}", last_err));
            }
        };

        // Attach hardware effects to this session per the quirks profile
        // (AEC/NS off by default — Android OEM versions mangle voice).
        let session_id = recorder.audio_session_id().unwrap_or(-1);
        info!(
            "AudioRecord initialized: source={} sessionId={}",
            chosen_source, session_id
        );
        if session_id > 0 {
            let applied = crate::audio_effects::apply(session_id, &quirks.effects);
            *self.effects.lock().unwrap() = Some(applied);
        }

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

        let quirks = crate::device_quirks::current();
        let multiplier = quirks.player_buffer_multiplier.max(PLAYER_BUFFER_FALLBACK);
        let alloc_size = buffer_size.checked_mul(multiplier).ok_or_else(|| {
            *self.state.lock().unwrap() = AudioState::Error;
            format!("Player buffer size {} would overflow i32 when scaled by {}", buffer_size, multiplier)
        })?;

        info!("Player buffer size: {} bytes (x{} = {})", buffer_size, multiplier, alloc_size);

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

    /// Start playing audio. Also engages `AudioManager.MODE_IN_COMMUNICATION`
    /// + force speakerphone if the active `DeviceQuirks` profile flags
    /// `output_force_comm_mode` — bypasses OEM media post-processing for
    /// devices that mangle voice (Moto, Xiaomi).
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

        // Engage comm-mode routing if the active quirk profile wants it.
        // Failure here is non-fatal — playback works either way; we just
        // log so the field complaint matches a missing context.
        let quirks = crate::device_quirks::current();
        if quirks.output_force_comm_mode {
            if let Err(e) = crate::audio_routing::engage_comm_mode(true) {
                warn!("audio_routing engage failed (continuing anyway): {}", e);
            }
        }
        info!("✓ Playback started");
        Ok(())
    }

    /// Stop playing audio. Releases comm-mode routing if it was engaged.
    pub fn stop_playing(&self) -> Result<(), String> {
        info!("Stopping audio playback");

        self.playing.store(false, Ordering::Relaxed);

        let play = self.player.lock().unwrap().as_ref().map(Arc::clone);
        match play {
            Some(play) => {
                play.stop()?;
                *self.state.lock().unwrap() = AudioState::Idle;
                // Restore the system audio mode + speakerphone state we
                // saved when we engaged. Safe to call even if we didn't
                // engage; it's a no-op when inactive.
                crate::audio_routing::release();
                info!("✓ Playback stopped");
                Ok(())
            }
            None => {
                warn!("Player not initialized");
                Ok(())
            }
        }
    }

    /// Write audio data for playback.
    ///
    /// On success, bumps BOTH:
    ///   - `last_write_at_ms` (wall-clock) — for the "is RX hot?" idle
    ///     check used at replay startup.
    ///   - `write_seq` (monotonic counter) — for replay's per-call
    ///     intrusion detection. Counter is granular per-call so two writes
    ///     in the same ms are still distinguishable.
    pub fn write_audio(&self, buffer: &[i16]) -> Result<usize, String> {
        let play = match self.player.lock().unwrap().as_ref() {
            Some(p) => Arc::clone(p),
            None => return Err("Player not initialized".to_string()),
        };
        let result = play.write(buffer);
        if result.is_ok() {
            self.last_write_at_ms.store(now_ms(), Ordering::Relaxed);
            self.write_seq.fetch_add(1, Ordering::Relaxed);
        }
        result
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
        // Drop the AppliedEffects so AEC/NS/AGC are torn down before the
        // recorder they're attached to is released — otherwise some HALs
        // log warnings about effect handles outliving their parent.
        let _ = self.effects.lock().unwrap().take();

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

// ── Playback gate helpers ─────────────────────────────────────────────────
// Free-standing so the replay thread can poll a cheap `Arc<AtomicU64>`
// snapshot without holding the engine mutex (which the live RX thread is
// frequently inside). The clock is monotonic-ish wall-clock ms since epoch;
// the exact value doesn't matter, only the delta.

#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Milliseconds since the playback clock was last bumped. `u64::MAX` if
/// the clock has never been written (i.e. AudioEngine just initialized).
pub fn playback_idle_ms(clock: &Arc<AtomicU64>) -> u64 {
    let last = clock.load(Ordering::Relaxed);
    if last == 0 { return u64::MAX; }
    now_ms().saturating_sub(last)
}

/// Block (sleep-poll) until the playback clock has been idle for at least
/// `min_idle_ms`, OR until `max_wait_ms` total has elapsed. Returns true if
/// the gate was acquired (idle long enough), false if we timed out and the
/// caller should bail. Caller is expected to call this BEFORE each batch of
/// frames it's about to write — RX activity in the meantime will reset the
/// idle window and force another wait.
pub fn wait_for_playback_idle(
    clock: &Arc<AtomicU64>,
    min_idle_ms: u64,
    max_wait_ms: u64,
) -> bool {
    use std::thread::sleep;
    use std::time::Duration;
    const POLL_INTERVAL_MS: u64 = 25;
    let mut waited: u64 = 0;
    loop {
        if playback_idle_ms(clock) >= min_idle_ms {
            return true;
        }
        if waited >= max_wait_ms {
            return false;
        }
        sleep(Duration::from_millis(POLL_INTERVAL_MS));
        waited += POLL_INTERVAL_MS;
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn playback_idle_returns_max_until_first_write() {
        let clock = Arc::new(AtomicU64::new(0));
        assert_eq!(playback_idle_ms(&clock), u64::MAX);
    }

    #[test]
    fn playback_idle_grows_after_write_stops() {
        let clock = Arc::new(AtomicU64::new(now_ms()));
        sleep(Duration::from_millis(80));
        let idle = playback_idle_ms(&clock);
        // Allow some slop for scheduler — the test is "did time pass at all?"
        assert!(idle >= 50, "expected idle >= 50, got {}", idle);
        assert!(idle < 5000, "expected idle < 5000, got {}", idle);
    }

    #[test]
    fn wait_for_playback_idle_succeeds_when_already_cold() {
        let clock = Arc::new(AtomicU64::new(0));  // never written = u64::MAX idle
        assert!(wait_for_playback_idle(&clock, 100, 500));
    }

    #[test]
    fn wait_for_playback_idle_succeeds_after_quiescence() {
        let clock = Arc::new(AtomicU64::new(now_ms()));
        // Don't bump the clock — let it go cold. The waiter should return true
        // once idle has crossed min_idle_ms.
        let acquired = wait_for_playback_idle(&clock, 100, 1000);
        assert!(acquired, "expected acquisition after quiescence");
    }

    #[test]
    fn wait_for_playback_idle_times_out_when_clock_keeps_bumping() {
        let clock = Arc::new(AtomicU64::new(now_ms()));
        let clock_for_thread = Arc::clone(&clock);
        // Continuously bump the clock so idle never crosses 100 ms.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let bumper = std::thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed) {
                clock_for_thread.store(now_ms(), Ordering::Relaxed);
                sleep(Duration::from_millis(10));
            }
        });
        let acquired = wait_for_playback_idle(&clock, 100, 300);
        stop.store(true, Ordering::Relaxed);
        let _ = bumper.join();
        assert!(!acquired, "expected timeout when clock keeps bumping");
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
