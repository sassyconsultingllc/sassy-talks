// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-E74JPYJ2CRHF
/// Audio Pipeline - TX/RX threads that wire the full audio path
///
/// TX path (on PTT press):
///   Mic → AudioEngine::read_audio → VoiceEncoder::encode → pack_wire_frame → Transport::send (encrypted)
///
/// RX path (always running when connected):
///   Transport::receive (decrypted) → unpack_wire_frame → VoiceDecoder::decode → AudioCache → AudioTrack
///
/// Also handles the TranscriptionBridge callback to Kotlin for speech-to-text.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use log::{error, info, warn};

use crate::audio::AudioEngine;
use crate::codec::{VoiceEncoder, VoiceDecoder, CODEC_FRAME_SIZE};
use crate::transport::TransportManager;
use crate::audio_cache::AudioCache;
use crate::users::UserRegistry;

/// Whether the Kotlin TranscriptionBridge.onAudioReceived callback should
/// be invoked for every RX audio frame. Defaults off — the feature is
/// opt-in (see TranscriptionBridge.setEnabled). When disabled, skipping
/// the call eliminates ~1.9 KB of short[] allocation, a JVM thread
/// attach, and a static method dispatch *per 20 ms frame* (50 Hz), all
/// of which sum to enough GC pressure to cause intermittent ~50-300 ms
/// AudioTrack underruns on the RX path.
static TRANSCRIPTION_BRIDGE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Toggle whether the RX thread invokes the Kotlin TranscriptionBridge.
/// Called from Kotlin via JNI (see jni_bridge.rs `nativeSetTranscriptionBridgeEnabled`).
pub fn set_transcription_bridge_enabled(enabled: bool) {
    TRANSCRIPTION_BRIDGE_ENABLED.store(enabled, Ordering::Relaxed);
    info!("Transcription bridge callback enabled = {}", enabled);
}

pub fn is_transcription_bridge_enabled() -> bool {
    TRANSCRIPTION_BRIDGE_ENABLED.load(Ordering::Relaxed)
}

/// Mic input gain, stored as `gain × 100` so the value is an atomic i32.
/// 100 = 1.0× (unity, no change), 50 = 0.5×, 200 = 2.0×, etc.
/// Clamped to [25, 400] in the setter (~ -12 dB to +12 dB).
static MIC_GAIN_X100: AtomicI32 = AtomicI32::new(100);

/// Squelch threshold in dBFS, integer. 0 = squelch disabled (transmit every
/// frame above silence). Otherwise a negative value like -40 means "drop any
/// frame whose RMS is below -40 dBFS". Useful for noisy environments.
static SQUELCH_DBFS: AtomicI32 = AtomicI32::new(0);

/// Set mic input gain. Input is `gain × 100` (so 100 = 1.0×). Clamped to [25, 400].
pub fn set_mic_gain_x100(g: i32) {
    let clamped = g.clamp(25, 400);
    MIC_GAIN_X100.store(clamped, Ordering::Relaxed);
    info!("Mic gain set to {:.2}x", clamped as f32 / 100.0);
}

pub fn get_mic_gain_x100() -> i32 {
    MIC_GAIN_X100.load(Ordering::Relaxed)
}

/// Set squelch threshold in dBFS. 0 disables squelch. Otherwise expects a
/// negative integer in [-60, -10]; values outside that range are clamped.
pub fn set_squelch_dbfs(threshold: i32) {
    let clamped = if threshold == 0 { 0 } else { threshold.clamp(-60, -10) };
    SQUELCH_DBFS.store(clamped, Ordering::Relaxed);
    if clamped == 0 {
        info!("Squelch disabled");
    } else {
        info!("Squelch threshold set to {} dBFS", clamped);
    }
}

pub fn get_squelch_dbfs() -> i32 {
    SQUELCH_DBFS.load(Ordering::Relaxed)
}

/// Apply mic gain to a PCM frame in place, clipping to i16 range.
fn apply_mic_gain(pcm: &mut [i16], gain_x100: i32) {
    if gain_x100 == 100 { return; } // unity — fast path
    let g = gain_x100 as f32 / 100.0;
    for s in pcm.iter_mut() {
        let scaled = (*s as f32 * g).clamp(i16::MIN as f32, i16::MAX as f32);
        *s = scaled as i16;
    }
}

/// Compute RMS dBFS for a PCM frame. Returns a very negative number for silence.
fn frame_dbfs(pcm: &[i16]) -> f32 {
    if pcm.is_empty() { return -120.0; }
    let mut sum_sq: f64 = 0.0;
    for s in pcm {
        let v = *s as f64 / 32768.0;
        sum_sq += v * v;
    }
    let rms = (sum_sq / pcm.len() as f64).sqrt();
    if rms <= 1e-9 { -120.0 } else { (20.0 * rms.log10()) as f32 }
}

// ── Public helpers used by the BT TX path in jni_bridge.rs so both pipelines
//    apply the same gain, squelch, and activity-log behavior. ──────────────

/// Apply the current user-configured mic gain to a PCM frame in place.
pub fn apply_mic_gain_public(pcm: &mut [i16]) {
    apply_mic_gain(pcm, MIC_GAIN_X100.load(Ordering::Relaxed));
}

/// Returns true if the user-configured squelch threshold says this frame
/// should be dropped. Returns false when squelch is disabled or the frame
/// is loud enough to transmit.
pub fn squelch_drops_frame(pcm: &[i16]) -> bool {
    let squelch = SQUELCH_DBFS.load(Ordering::Relaxed);
    if squelch == 0 { return false; }
    frame_dbfs(pcm) < squelch as f32
}

/// Invoke the activity-log bridge with the given PCM frame and per-sender
/// favorite/muted flags. The bridge ignores the call cheaply when
/// transcription is disabled.
pub fn call_transcription_bridge_public(
    sender_id: &str,
    device_name: &str,
    pcm: &[i16],
    is_favorite: bool,
    is_muted: bool,
    is_self: bool,
) {
    call_transcription_bridge(sender_id, device_name, pcm, is_favorite, is_muted, is_self);
}

// The audio wire frame now lives in `sassytalkie-core` (core::wire) so iOS,
// Android, and desktop stay byte-identical on the multicast wire — it used to be
// defined only here, leaving the other consumers nothing to build against. The
// format, bounds checks, and MAX_* limits moved verbatim; re-exported under the
// original names so every call site (and test) in this crate is unchanged.
pub use sassytalkie_core::wire::{
    pack_wire_frame, unpack_wire_frame, now_ms, MAX_SENDER_ID_LEN, MAX_DEVICE_NAME_LEN,
};

/// Spawn the TX thread: captures mic audio, encodes, encrypts, and sends while PTT is held.
///
/// The thread runs in a loop while `tx_running` is true. It only captures+sends when
/// `ptt_pressed` is true.
/// Whether PTT should buffer audio and burst-send on release (true)
/// or stream live frame-by-frame (false). Default: false (live mode) — a
/// walkie-talkie must be heard the moment PTT is pressed; the prior `true`
/// default produced "audio only flows when both peers press at the same
/// time" because the buffer was only flushed on release, often after the
/// relay socket had already cycled.
static PTT_BUFFER_MODE: AtomicBool = AtomicBool::new(false);

/// Set PTT buffer mode. true = buffer and burst on release. false = live stream.
pub fn set_ptt_buffer_mode(buffer: bool) {
    PTT_BUFFER_MODE.store(buffer, Ordering::SeqCst);
    info!("TX: PTT buffer mode = {}", buffer);
}

pub fn get_ptt_buffer_mode() -> bool {
    PTT_BUFFER_MODE.load(Ordering::SeqCst)
}

pub fn spawn_tx_thread(
    tx_running: Arc<AtomicBool>,
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    current_subchannel: Arc<AtomicU8>,
    audio: Arc<Mutex<AudioEngine>>,
    transport: Arc<Mutex<TransportManager>>,
    local_sender_id: String,
    local_device_name: String,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("sassy-tx".into())
        .spawn(move || {
            info!("TX thread started");

            let mut encoder = VoiceEncoder::new();
            let mut pcm_buffer = vec![0i16; CODEC_FRAME_SIZE];
            let mut was_transmitting = false;
            let mut idle_ticks = 0u32;
            // Monotonic wire timestamps for the current PTT press. Wall-clock
            // now_ms() can repeat within the same millisecond on fast devices,
            // which makes the cross-transport dedup layer drop real frames.
            let mut tx_frame_index: u64 = 0;
            let mut tx_stream_epoch_ms: u64 = 0;

            // Buffer for burst-send mode: accumulates wire frames during PTT
            let mut tx_frame_buffer: Vec<Vec<u8>> = Vec::new();

            // Send initial presence beacon so peers register us immediately on connect
            {
                let silence = vec![0i16; CODEC_FRAME_SIZE];
                let compressed = encoder.encode(&silence);
                if !compressed.is_empty() {
                    let channel = current_channel.load(Ordering::SeqCst);
                    let subch = current_subchannel.load(Ordering::SeqCst);
                    let ts = now_ms();
                    let wire = pack_wire_frame(channel, subch, &local_sender_id, &local_device_name, ts, &compressed);
                    let mut tm = transport.lock().unwrap();
                    let _ = tm.send(&wire);
                    info!("TX: sent initial presence beacon");
                }
            }

            while tx_running.load(Ordering::SeqCst) {
                if !ptt_pressed.load(Ordering::SeqCst) {
                    // Not transmitting
                    if was_transmitting {
                        // PTT released: stop recording
                        let eng = audio.lock().unwrap();
                        let _ = eng.stop_recording();
                        was_transmitting = false;
                        encoder.reset();

                        // In buffer mode: flush all accumulated frames with pacing
                        // so the receiver can play them back without drops
                        if PTT_BUFFER_MODE.load(Ordering::SeqCst) && !tx_frame_buffer.is_empty() {
                            let frame_count = tx_frame_buffer.len();
                            info!("TX: burst-sending {} buffered frames", frame_count);
                            for frame in tx_frame_buffer.drain(..) {
                                let mut tm = transport.lock().unwrap();
                                let _ = tm.send(&frame);
                                drop(tm);
                                // Pace at real-time (20ms per frame = 1x playback speed)
                                // so receiver plays back naturally without buffer overrun
                                thread::sleep(Duration::from_millis(20));
                            }
                            info!("TX: burst-send complete ({} frames)", frame_count);
                        }

                        info!("TX: stopped recording (PTT released)");
                    }
                    // Periodic presence heartbeat every ~10 seconds so peers keep seeing us
                    idle_ticks += 1;
                    if idle_ticks >= 2000 {
                        idle_ticks = 0;
                        let silence = vec![0i16; CODEC_FRAME_SIZE];
                        let compressed = encoder.encode(&silence);
                        if !compressed.is_empty() {
                            let ch = current_channel.load(Ordering::SeqCst);
                            let subch = current_subchannel.load(Ordering::SeqCst);
                            let ts = now_ms();
                            let wire = pack_wire_frame(ch, subch, &local_sender_id, &local_device_name, ts, &compressed);
                            let mut tm = transport.lock().unwrap();
                            let _ = tm.send(&wire);
                        }
                    }
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }

                // PTT is pressed: transmit
                if !was_transmitting {
                    // PTT just pressed: start recording
                    let eng = audio.lock().unwrap();
                    match eng.start_recording() {
                        Ok(()) => {
                            was_transmitting = true;
                            tx_frame_index = 0;
                            tx_stream_epoch_ms = now_ms();
                            info!("TX: started recording (PTT pressed)");
                        }
                        Err(e) => {
                            error!("TX: failed to start recording: {}", e);
                            thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                    }
                }

                // Read one full frame from the mic — accumulate partial reads.
                //
                // The previous version threw away the buffer on a short read,
                // which on Qualcomm HALs (Moto, Xiaomi) that deliver in
                // multiples of 480 samples meant every other read returned 0
                // useful samples — producing alternating real-frame/silence
                // output, which decoded on the receiver as the textbook
                // "robotic" artifact. Accumulate into the buffer until a full
                // CODEC_FRAME_SIZE (960 samples / 20 ms) is captured.
                let mut filled: usize = 0;
                let mut read_attempts: u32 = 0;
                const MAX_READ_ATTEMPTS_PER_FRAME: u32 = 25; // ~50 ms cap; avoids hang on broken HAL
                while filled < CODEC_FRAME_SIZE && read_attempts < MAX_READ_ATTEMPTS_PER_FRAME {
                    read_attempts += 1;
                    let n = {
                        let eng = audio.lock().unwrap();
                        match eng.read_audio(&mut pcm_buffer[filled..CODEC_FRAME_SIZE]) {
                            Ok(n) => n,
                            Err(e) => { warn!("TX: read_audio failed: {}", e); 0 }
                        }
                    };
                    if n == 0 {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    filled += n;
                }
                if filled < CODEC_FRAME_SIZE {
                    warn!("TX: partial frame after {} attempts (filled={}/{}); dropping", read_attempts, filled, CODEC_FRAME_SIZE);
                    continue;
                }

                // Apply user-configured mic gain in place (unity by default).
                // For clogged-mic users this lifts a weak signal up; for
                // headset users with hot mics, a fractional gain attenuates.
                apply_mic_gain(&mut pcm_buffer[..CODEC_FRAME_SIZE], MIC_GAIN_X100.load(Ordering::Relaxed));

                // Software noise suppression (spectral subtraction / Wiener).
                // No-op when disabled (default). Runs AFTER gain so it cleans
                // the level the listener hears, and BEFORE squelch so the
                // dBFS check sees the denoised signal — steady noise no longer
                // holds the squelch gate open.
                crate::audio_effects::denoise_frame(&mut pcm_buffer[..CODEC_FRAME_SIZE]);

                // Squelch: if user enabled a dBFS threshold, drop frames
                // below it so background noise isn't transmitted. 0 = off
                // (default) — every frame goes through.
                let squelch = SQUELCH_DBFS.load(Ordering::Relaxed);
                if squelch != 0 {
                    let d = frame_dbfs(&pcm_buffer[..CODEC_FRAME_SIZE]);
                    if d < squelch as f32 {
                        continue;
                    }
                }

                // Feed self-PCM into the timeline bridge so the activity log
                // records "you spoke for Xs" on PTT release. is_self = true so
                // the bridge records the timeline entry but does NOT surface the
                // remote-speaker UI ("X is speaking" toast / activeSpeakerName)
                // for our own transmit.
                call_transcription_bridge(
                    &local_sender_id,
                    &local_device_name,
                    &pcm_buffer[..CODEC_FRAME_SIZE],
                    false,
                    false,
                    true, // is_self
                );

                // Encode with Opus
                let compressed = encoder.encode(&pcm_buffer[..CODEC_FRAME_SIZE]);

                // Pack wire frame (includes device name for receiver display)
                let channel = current_channel.load(Ordering::SeqCst);
                let subch = current_subchannel.load(Ordering::SeqCst);
                let timestamp = tx_stream_epoch_ms.saturating_add(tx_frame_index.saturating_mul(20));
                tx_frame_index += 1;
                let wire_data = pack_wire_frame(channel, subch, &local_sender_id, &local_device_name, timestamp, &compressed);

                if PTT_BUFFER_MODE.load(Ordering::SeqCst) {
                    // Buffer mode: accumulate frames, burst-send on PTT release
                    tx_frame_buffer.push(wire_data);
                } else {
                    // Live mode: send immediately frame-by-frame
                    let mut tm = transport.lock().unwrap();
                    if let Err(e) = tm.send(&wire_data) {
                        warn!("TX: send failed: {}", e);
                    }
                }
            }

            // Cleanup
            if was_transmitting {
                let eng = audio.lock().unwrap();
                let _ = eng.stop_recording();
            }
            info!("TX thread stopped");
        })
}

// ── RX playout helpers ──────────────────────────────────────────────────────

/// Pace AudioTrack writes at one 20 ms frame per tick. Without this, a burst
/// of relay packets is decoded and written as fast as the CPU loop runs —
/// AudioTrack underruns once the burst is drained, which sounds choppy/garbled.
struct PlayoutPacer {
    next_playout: Instant,
    frame_period: Duration,
}

impl PlayoutPacer {
    fn new() -> Self {
        Self {
            next_playout: Instant::now(),
            frame_period: Duration::from_millis(20),
        }
    }

    fn wait_before_write(&mut self) {
        let now = Instant::now();
        if now < self.next_playout {
            thread::sleep(self.next_playout - now);
        } else {
            // More than ~200 ms behind — snap forward instead of playing back
            // a multi-second backlog at 20 ms/frame (sounds slowed-down).
            let behind = now.saturating_duration_since(self.next_playout);
            if behind > Duration::from_millis(200) {
                self.next_playout = now;
            }
        }
        self.next_playout += self.frame_period;
    }
}

/// Per-sender pre-decode reorder buffer. Opus is stateful — decoding in network
/// arrival order when frames arrive out-of-order (common on cellular relay)
/// produces garbled PCM that the post-decode jitter buffer cannot fix.
struct SenderDecodeState {
    decoder: VoiceDecoder,
    last_decoded_ts: Option<u64>,
    pending: BTreeMap<u64, Vec<u8>>,
    /// Set when we're blocked waiting for the next in-sequence timestamp.
    reorder_wait_since: Option<Instant>,
}

impl SenderDecodeState {
    fn new() -> Self {
        Self {
            decoder: VoiceDecoder::new(),
            last_decoded_ts: None,
            pending: BTreeMap::new(),
            reorder_wait_since: None,
        }
    }

    /// Buffer one compressed frame and decode every in-sequence frame now ready.
    fn ingest(&mut self, timestamp: u64, compressed: Vec<u8>, frame_period_ms: u64, max_plc_frames: u64, reorder_wait_ms: u64) -> Vec<(u64, Vec<i16>)> {
        if let Some(last) = self.last_decoded_ts {
            if timestamp <= last {
                return vec![]; // duplicate or already-decoded
            }
        }
        const MAX_PENDING: usize = 32;
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_first();
        }
        self.pending.insert(timestamp, compressed);
        self.drain_decodable(frame_period_ms, max_plc_frames, reorder_wait_ms)
    }

    fn drain_decodable(&mut self, frame_period_ms: u64, max_plc_frames: u64, reorder_wait_ms: u64) -> Vec<(u64, Vec<i16>)> {
        let mut out = Vec::new();
        loop {
            let expected_ts = match self.last_decoded_ts {
                Some(last) => last.saturating_add(frame_period_ms),
                None => match self.pending.keys().next().copied() {
                    Some(t) => t,
                    None => break,
                },
            };

            if let Some(compressed) = self.pending.remove(&expected_ts) {
                self.reorder_wait_since = None;
                let pcm = self.decoder.decode(&compressed);
                self.last_decoded_ts = Some(expected_ts);
                out.push((expected_ts, pcm));
                continue;
            }

            // Expected frame missing — wait briefly for reorder, then repair.
            let earliest = match self.pending.keys().next().copied() {
                Some(t) => t,
                None => break,
            };

            if earliest <= expected_ts {
                break;
            }

            let wait_start = *self.reorder_wait_since.get_or_insert_with(Instant::now);
            if wait_start.elapsed() < Duration::from_millis(reorder_wait_ms) {
                break; // still hoping the missing frame arrives
            }
            self.reorder_wait_since = None;

            let gap_frames = (earliest - expected_ts) / frame_period_ms;
            if gap_frames == 0 {
                break;
            }

            if gap_frames == 1 {
                let next_pkt = match self.pending.get(&earliest) {
                    Some(p) => p.clone(),
                    None => break,
                };
                let fec_pcm = self.decoder.decode_fec_from_next(&next_pkt);
                self.last_decoded_ts = Some(expected_ts);
                out.push((expected_ts, fec_pcm));
                continue;
            }

            if gap_frames >= 2 && gap_frames <= max_plc_frames {
                for i in 0..gap_frames {
                    let synth_ts = expected_ts + frame_period_ms * i;
                    let plc_pcm = self.decoder.decode_plc();
                    self.last_decoded_ts = Some(synth_ts);
                    out.push((synth_ts, plc_pcm));
                }
                continue;
            }

            if gap_frames > max_plc_frames {
                self.decoder.reset();
                self.last_decoded_ts = None;
                info!("RX: large gap of {} frames — decoder reset, resync at ts={}", gap_frames, earliest);
                continue;
            }

            break;
        }
        out
    }
}

/// Shared RX decode + playout queue. The dedicated RX thread drains playout
/// with a 20 ms pacer; the BT JNI path enqueues here instead of writing to
/// AudioTrack directly (which raced the RX thread and garbled playback).
pub struct RxSharedState {
    sender_states: HashMap<String, SenderDecodeState>,
    playout_queue: VecDeque<Vec<i16>>,
}

/// Cap playout backlog at ~1 s. Beyond that, drop oldest frames to stay near
/// live instead of playing seconds-late audio after a network burst.
const MAX_PLAYOUT_QUEUE_FRAMES: usize = 48;

pub const RX_FRAME_PERIOD_MS: u64 = 20;
pub const RX_MAX_PLC_FRAMES: u64 = 4;
pub const RX_REORDER_WAIT_MS: u64 = 40;

impl RxSharedState {
    pub fn new() -> Self {
        Self {
            sender_states: HashMap::new(),
            playout_queue: VecDeque::new(),
        }
    }

    fn enqueue_playout(&mut self, samples: Vec<i16>) {
        while self.playout_queue.len() >= MAX_PLAYOUT_QUEUE_FRAMES {
            self.playout_queue.pop_front();
        }
        self.playout_queue.push_back(samples);
    }

    /// Decode one compressed wire frame and feed PCM into the audio cache.
    /// Any frames ready for immediate playout are pushed onto the shared queue.
    pub fn ingest_wire_frame(
        &mut self,
        cache: &mut AudioCache,
        sender_id: &str,
        timestamp: u64,
        compressed: Vec<u8>,
    ) -> Vec<(u64, Vec<i16>)> {
        let state = self
            .sender_states
            .entry(sender_id.to_string())
            .or_insert_with(SenderDecodeState::new);
        let decoded = state.ingest(
            timestamp,
            compressed,
            RX_FRAME_PERIOD_MS,
            RX_MAX_PLC_FRAMES,
            RX_REORDER_WAIT_MS,
        );
        for (frame_ts, pcm) in &decoded {
            if let Some(samples) = cache.ingest_frame(sender_id, *frame_ts, pcm.clone()) {
                self.enqueue_playout(samples);
            }
        }
        decoded
    }

    /// Pull queue/jitter-drain frames from the cache into the playout queue.
    pub fn drain_cache_playout(&mut self, cache: &mut AudioCache) {
        while let Some((_sender, samples)) = cache.next_playback_frame() {
            self.enqueue_playout(samples);
        }
    }

    pub fn pop_playout(&mut self) -> Option<Vec<i16>> {
        self.playout_queue.pop_front()
    }

    pub fn playout_pending(&self) -> usize {
        self.playout_queue.len()
    }
}

/// Process one decrypted wire frame through the full RX path (decode → cache →
/// playout queue). Used by both the RX thread and the BT JNI callback.
pub fn process_incoming_wire_frame(
    rx_shared: &Arc<Mutex<RxSharedState>>,
    audio_cache: &Arc<Mutex<AudioCache>>,
    user_registry: &Arc<Mutex<UserRegistry>>,
    local_sender_id: &str,
    my_channel: u8,
    my_subchannel: u8,
    channel: u8,
    subchannel: u8,
    sender_id: &str,
    device_name: &str,
    timestamp: u64,
    compressed: &[u8],
    invoke_transcription: bool,
) {
    if sender_id == local_sender_id {
        return;
    }
    if channel != my_channel {
        return;
    }
    if compressed.is_empty() {
        return;
    }

    if subchannel != my_subchannel {
        let mut reg = user_registry.lock().unwrap();
        reg.register_user(sender_id, device_name);
        return;
    }

    {
        let mut reg = user_registry.lock().unwrap();
        reg.register_user(sender_id, device_name);
    }
    {
        let reg = user_registry.lock().unwrap();
        let is_fav = reg.is_favorite(sender_id);
        let is_muted = reg.is_muted(sender_id);
        let mut cache = audio_cache.lock().unwrap();
        cache.update_user_info(sender_id, device_name, is_fav, is_muted);
    }

    let decoded = {
        let mut rx = rx_shared.lock().unwrap();
        let mut cache = audio_cache.lock().unwrap();
        let frames = rx.ingest_wire_frame(
            &mut cache,
            sender_id,
            timestamp,
            compressed.to_vec(),
        );
        cache.tick();
        rx.drain_cache_playout(&mut cache);
        frames
    };

    if invoke_transcription {
        let reg = user_registry.lock().unwrap();
        let is_favorite = reg.is_favorite(sender_id);
        let is_muted = reg.is_muted(sender_id);
        for (_ts, pcm) in decoded {
            call_transcription_bridge(sender_id, device_name, &pcm, is_favorite, is_muted, false);
        }
    }
}

/// Spawn the RX thread: receives, decrypts, decodes, feeds into AudioCache, and plays back.
///
/// Also calls the TranscriptionBridge callback if available.
pub fn spawn_rx_thread(
    rx_running: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    current_subchannel: Arc<AtomicU8>,
    audio: Arc<Mutex<AudioEngine>>,
    transport: Arc<Mutex<TransportManager>>,
    audio_cache: Arc<Mutex<AudioCache>>,
    user_registry: Arc<Mutex<UserRegistry>>,
    rx_shared: Arc<Mutex<RxSharedState>>,
    local_sender_id: String,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("sassy-rx".into())
        .spawn(move || {
            info!("RX thread started");

            let mut recv_buffer = vec![0u8; 2048];
            let mut cell_buffer = vec![0u8; 2048];
            let mut playback_started = false;
            let mut pacer = PlayoutPacer::new();

            while rx_running.load(Ordering::SeqCst) {
                let mut received_any = false;

                // Drain WiFi AND cellular every iteration — dual-path TX means
                // both queues can have audio regardless of `active` transport.
                loop {
                    let (wifi_n, cell_n) = {
                        let mut tm = transport.lock().unwrap();
                        let w = tm.poll_wifi_into(&mut recv_buffer).unwrap_or(0);
                        let c = tm.poll_cellular_into(&mut cell_buffer).unwrap_or(0);
                        (w, c)
                    };

                    if wifi_n == 0 && cell_n == 0 {
                        break;
                    }

                    let my_channel = current_channel.load(Ordering::SeqCst);
                    let my_subchannel = current_subchannel.load(Ordering::SeqCst);

                    for (bytes_received, buf) in [
                        (wifi_n, recv_buffer.as_slice()),
                        (cell_n, cell_buffer.as_slice()),
                    ] {
                        if bytes_received == 0 {
                            continue;
                        }
                        received_any = true;

                        let (channel, subchannel, sender_id, device_name, timestamp, compressed) =
                            match unpack_wire_frame(&buf[..bytes_received]) {
                                Ok(parsed) => parsed,
                                Err(e) => {
                                    warn!("RX: invalid wire frame: {}", e);
                                    continue;
                                }
                            };

                        process_incoming_wire_frame(
                            &rx_shared,
                            &audio_cache,
                            &user_registry,
                            &local_sender_id,
                            my_channel,
                            my_subchannel,
                            channel,
                            subchannel,
                            &sender_id,
                            &device_name,
                            timestamp,
                            &compressed,
                            true,
                        );
                    }
                }

                let commits = {
                    let mut rx = rx_shared.lock().unwrap();
                    let mut cache = audio_cache.lock().unwrap();
                    cache.tick();
                    let commits = cache.take_newly_committed_ids();
                    rx.drain_cache_playout(&mut cache);
                    commits
                };
                dispatch_committed_utterances(&commits);

                if let Some(samples) = rx_shared.lock().unwrap().pop_playout() {
                    pacer.wait_before_write();
                    if !playback_started {
                        let eng = audio.lock().unwrap();
                        let _ = eng.start_playing();
                        playback_started = true;
                    }
                    let eng = audio.lock().unwrap();
                    let _ = eng.write_audio(&samples);
                }

                let playout_empty = rx_shared.lock().unwrap().playout_pending() == 0;
                if !received_any && playout_empty {
                    thread::sleep(Duration::from_millis(5));
                }
            }

            if playback_started {
                let eng = audio.lock().unwrap();
                let _ = eng.stop_playing();
            }
            info!("RX thread stopped");
        })
}

/// Forward a batch of just-committed utterance IDs to Kotlin so the
/// timeline UI can wire its replay button to the correct ID. Replaces
/// the previous best-effort `lastHistoryId()` poll which raced against
/// the cache's commit timer and silently captured the wrong (or no) ID.
///
/// Calls `TranscriptionBridge.onUtteranceCommitted(senderId, senderName, utteranceId, durationMs)`
/// once per commit. Failures are logged and swallowed — the timeline UI
/// degrades to "no replay button" rather than crashing the RX thread.
fn dispatch_committed_utterances(commits: &[(u64, String, String, u64)]) {
    if commits.is_empty() {
        return;
    }
    use jni::objects::{JValue, JClass, GlobalRef};

    let bridge_ref: &GlobalRef = match crate::jni_bridge::get_transcription_bridge_class() {
        Some(r) => r,
        None => return,
    };
    let vm = match crate::jni_bridge::get_jvm() {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    if env.push_local_frame(8).is_err() { return; }

    for (id, sender_id, sender_name, duration_ms) in commits {
        let _ = (|| -> Option<()> {
            let j_sender_id = env.new_string(sender_id).ok()?;
            let j_sender_name = env.new_string(sender_name).ok()?;
            let bridge_class = unsafe { JClass::from_raw(bridge_ref.as_obj().as_raw()) };
            let result = env.call_static_method(
                &bridge_class,
                "onUtteranceCommitted",
                "(Ljava/lang/String;Ljava/lang/String;JJ)V",
                &[
                    JValue::Object(&j_sender_id.into()),
                    JValue::Object(&j_sender_name.into()),
                    JValue::Long(*id as i64),
                    JValue::Long(*duration_ms as i64),
                ],
            );
            if result.is_err() {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            std::mem::forget(bridge_class);
            Some(())
        })();
    }

    unsafe { let _ = env.pop_local_frame(&jni::objects::JObject::null()); }
}

/// Call the Kotlin TranscriptionBridge.onAudioReceived callback via JNI.
///
/// Uses the cached GlobalRef from nativeInit (resolved on the main thread with
/// the app classloader) so that native RX threads can find the class.
/// This avoids ClassNotFoundException on attached native threads which only
/// have the system classloader.
fn call_transcription_bridge(
    sender_id: &str,
    sender_name: &str,
    pcm_samples: &[i16],
    is_favorite: bool,
    is_muted: bool,
    // True when this frame is the LOCAL device's own outgoing audio (fed in so
    // the activity timeline can log "you spoke"). The Kotlin side keeps the
    // timeline bookkeeping but suppresses the remote-speaker UI (the
    // "X is speaking" toast / activeSpeakerName / incomingAudio) for self frames.
    is_self: bool,
) {
    // Fast path: feature disabled → return without touching JNI. Avoids
    // attaching a JNI thread + allocating a short[] every 20 ms when
    // transcription is off (the default).
    if !TRANSCRIPTION_BRIDGE_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    use jni::objects::{JValue, JClass, GlobalRef};
    use jni::sys::{JNI_TRUE, JNI_FALSE};

    // Use the cached class ref (resolved on the main thread during nativeInit)
    let bridge_ref: &GlobalRef = match crate::jni_bridge::get_transcription_bridge_class() {
        Some(r) => r,
        None => return, // TranscriptionBridge not available (class not found at init)
    };

    let vm = match crate::jni_bridge::get_jvm() {
        Ok(v) => v,
        Err(_) => return, // JVM not available (running tests)
    };

    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };

    // Bound the local-ref count: strings + short[] would otherwise accumulate
    // across every received audio frame and overflow the local-ref table on
    // the long-lived RX thread.
    if env.push_local_frame(8).is_err() { return; }

    // Closure so we can always pop_local_frame before returning.
    let _ = (|| -> Option<()> {
        let j_sender_id = env.new_string(sender_id).ok()?;
        let j_sender_name = env.new_string(sender_name).ok()?;

        let j_pcm = env.new_short_array(pcm_samples.len() as i32).ok()?;
        if env.set_short_array_region(&j_pcm, 0, pcm_samples).is_err() {
            return None;
        }

        let j_fav = if is_favorite { JNI_TRUE } else { JNI_FALSE };
        let j_muted = if is_muted { JNI_TRUE } else { JNI_FALSE };
        let j_self = if is_self { JNI_TRUE } else { JNI_FALSE };

        // Call static method using cached GlobalRef (carries app classloader context)
        // Safety: GlobalRef -> JObject -> JClass cast is valid for class references
        let bridge_class = unsafe { JClass::from_raw(bridge_ref.as_obj().as_raw()) };
        let result = env.call_static_method(
            &bridge_class,
            "onAudioReceived",
            "(Ljava/lang/String;Ljava/lang/String;[SZZZ)V",
            &[
                JValue::Object(&j_sender_id.into()),
                JValue::Object(&j_sender_name.into()),
                JValue::Object(&j_pcm.into()),
                JValue::Bool(j_fav),
                JValue::Bool(j_muted),
                JValue::Bool(j_self),
            ],
        );

        // Clear any pending exception so it doesn't crash the RX thread
        if result.is_err() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        // Don't drop bridge_class - it's borrowed from the global ref, not owned
        std::mem::forget(bridge_class);
        Some(())
    })();

    unsafe { let _ = env.pop_local_frame(&jni::objects::JObject::null()); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_frame_roundtrip() {
        let channel = 5u8;
        let subchannel = 1u8;
        let sender_id = "abc123def456";
        let device_name = "John's Galaxy S24";
        let timestamp = 1700000000000u64;
        let compressed = vec![42u8; 40];

        let packed = pack_wire_frame(channel, subchannel, sender_id, device_name, timestamp, &compressed);
        let (ch, sub, sid, name, ts, audio) = unpack_wire_frame(&packed).unwrap();

        assert_eq!(ch, channel);
        assert_eq!(sub, subchannel);
        assert_eq!(sid, sender_id);
        assert_eq!(name, device_name);
        assert_eq!(ts, timestamp);
        assert_eq!(audio, compressed);
    }

    #[test]
    fn test_wire_frame_empty_fields() {
        let packed = pack_wire_frame(1, 0, "", "", 100, &[1, 2, 3]);
        let (ch, sub, sid, name, ts, audio) = unpack_wire_frame(&packed).unwrap();
        assert_eq!(ch, 1);
        assert_eq!(sub, 0);
        assert_eq!(sid, "");
        assert_eq!(name, "");
        assert_eq!(ts, 100);
        assert_eq!(audio, vec![1, 2, 3]);
    }

    #[test]
    fn test_wire_frame_too_short() {
        let result = unpack_wire_frame(&[0; 5]);
        assert!(result.is_err());
    }

    #[test]
    fn test_wire_frame_invalid_sender_len() {
        let mut data = vec![0u8; 20];
        data[2] = 200; // sender_id_len > MAX_SENDER_ID_LEN (offset shifted by 1 for subchannel)
        let result = unpack_wire_frame(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_wire_frame_invalid_name_len() {
        // channel=0, subchannel=0, sender_id_len=0, name_len=200 (too large)
        let mut data = vec![0u8; 20];
        data[2] = 0; // sender_id_len = 0
        data[3] = 200; // name_len > MAX_DEVICE_NAME_LEN
        let result = unpack_wire_frame(&data);
        assert!(result.is_err());
    }
}
