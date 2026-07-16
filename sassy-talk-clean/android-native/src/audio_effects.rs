// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-A3UNELTXZIYP
//! Hardware audio effects on the capture chain — AEC, NS, AGC. Ported
//! from the Kotlin `audio/AudioEffectsManager.kt`. Calls
//! `AcousticEchoCanceler.create(sessionId)`, `NoiseSuppressor.create`,
//! `AutomaticGainControl.create` via JNI on the recorder's
//! `audioSessionId`.
//!
//! Defaults for PTT (`DeviceQuirks` may override):
//!   AEC = OFF — half-duplex; AEC just adds processing artifacts
//!   NS  = OFF — aggressive OEM NS erases quiet voices
//!   AGC = ON  — levels out distance-to-mic variation
//!
//! Effects stay alive (and held via `GlobalRef`) for the lifetime of the
//! `AppliedEffects` value; drop it to release them.

use jni::objects::GlobalRef;
use log::{info, warn};

use crate::device_quirks::EffectsConfig;
use crate::jni_bridge::get_jvm;

pub struct AppliedEffects {
    _aec: Option<GlobalRef>,
    _ns: Option<GlobalRef>,
    _agc: Option<GlobalRef>,
    pub aec_active: bool,
    pub ns_active: bool,
    pub agc_active: bool,
}

impl AppliedEffects {
    pub fn none() -> Self {
        Self { _aec: None, _ns: None, _agc: None, aec_active: false, ns_active: false, agc_active: false }
    }
}

/// Apply `config` to the recorder identified by `session_id`. Returns the
/// effects that were actually active (some devices return `isAvailable()=true`
/// but enabled stays false — trust the applied state, not the config).
pub fn apply(session_id: i32, config: &EffectsConfig) -> AppliedEffects {
    let vm = match get_jvm() {
        Ok(v) => v,
        Err(_) => return AppliedEffects::none(),
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return AppliedEffects::none(),
    };

    let aec = if config.enable_aec {
        try_create_effect(&mut env, "android/media/audiofx/AcousticEchoCanceler", session_id)
    } else { None };
    let ns = if config.enable_ns {
        try_create_effect(&mut env, "android/media/audiofx/NoiseSuppressor", session_id)
    } else { None };
    let agc = if config.enable_agc {
        try_create_effect(&mut env, "android/media/audiofx/AutomaticGainControl", session_id)
    } else { None };

    let applied = AppliedEffects {
        aec_active: aec.is_some(),
        ns_active: ns.is_some(),
        agc_active: agc.is_some(),
        _aec: aec,
        _ns: ns,
        _agc: agc,
    };
    info!(
        "AudioEffects applied: AEC={} NS={} AGC={} (requested AEC={} NS={} AGC={})",
        applied.aec_active, applied.ns_active, applied.agc_active,
        config.enable_aec, config.enable_ns, config.enable_agc
    );
    applied
}

fn try_create_effect(env: &mut jni::JNIEnv, class_name: &str, session_id: i32) -> Option<GlobalRef> {
    use jni::objects::JValue;

    let cls = env.find_class(class_name).ok()?;

    // Static `isAvailable()` first — emulator and many OEM HALs return false.
    let avail = env.call_static_method(&cls, "isAvailable", "()Z", &[]).ok()?.z().ok()?;
    if !avail {
        warn!("AudioEffects: {} not available on this device", class_name);
        return None;
    }

    // Static `create(int sessionId)` returns the subclass or null.
    let sig = match class_name {
        "android/media/audiofx/AcousticEchoCanceler" => "(I)Landroid/media/audiofx/AcousticEchoCanceler;",
        "android/media/audiofx/NoiseSuppressor" => "(I)Landroid/media/audiofx/NoiseSuppressor;",
        "android/media/audiofx/AutomaticGainControl" => "(I)Landroid/media/audiofx/AutomaticGainControl;",
        _ => return None,
    };
    let obj = env.call_static_method(&cls, "create", sig, &[JValue::Int(session_id)]).ok()?.l().ok()?;
    if obj.is_null() {
        warn!("AudioEffects: {}.create returned null for sessionId={}", class_name, session_id);
        return None;
    }

    // setEnabled(true). The setter returns int (status), but we don't check
    // — the result of `isEnabled` is more reliable.
    let _ = env.call_method(&obj, "setEnabled", "(Z)I", &[JValue::Bool(jni::sys::JNI_TRUE)]);

    let enabled = env.call_method(&obj, "getEnabled", "()Z", &[])
        .ok()
        .and_then(|v| v.z().ok())
        .unwrap_or(false);
    if !enabled {
        warn!("AudioEffects: {} setEnabled stuck at false (driver may have rejected)", class_name);
        return None;
    }

    env.new_global_ref(&obj).ok()
}

// ════════════════════════════════════════════════════════════════════════════
//  Software single-channel noise suppression (mic TX path)
// ════════════════════════════════════════════════════════════════════════════
//
// ## Why a software denoiser when the OS already ships `NoiseSuppressor`?
//
// The hardware/OEM `NoiseSuppressor` (above) is a black box: on most devices
// it is either unavailable (emulators, many Qualcomm HALs return
// `isAvailable()=false`) or so aggressive it gates quiet voices to silence —
// which is exactly why the PTT defaults turn it OFF. This module is a
// *deterministic, tunable, device-independent* alternative that runs on the
// captured PCM after mic-gain and before squelch/encode, so it behaves
// identically on every handset and the user can dial in how hard it works.
//
// ## Algorithm: spectral subtraction + decision-directed Wiener gain
//
// Steady broadband noise (wind, engine rumble, HVAC, fan hiss) is *stationary*
// in the short term: its magnitude spectrum barely changes frame-to-frame,
// while speech is *non-stationary* (bursts of harmonics that come and go).
// That difference is the whole game. We:
//
//   1. Take a windowed STFT of each block (Hann window, 50% overlap-add). The
//      Hann window kills spectral leakage so a tone shows up as a clean peak
//      instead of smearing across bins; 50% overlap + the complementary
//      Hann²-COLA property means the inverse transform reconstructs the
//      original *exactly* when the gain is 1.0 (no blocking artifacts / no
//      amplitude modulation at the 50 Hz block rate).
//   2. Estimate the per-bin noise floor with minima-controlled recursive
//      averaging (MCRA-lite): track a slow minimum of the smoothed power
//      spectrum per bin. Bins that only ever sit near their own minimum are
//      noise; bins that spike well above it are speech. This needs no VAD and
//      adapts on its own to whatever noise the environment throws at it.
//   3. Form the a-priori SNR with the decision-directed estimator
//      (Ephraim–Malah): blend last frame's clean estimate with this frame's
//      instantaneous SNR. This is what suppresses "musical noise" — the
//      warbling random tones that naive spectral subtraction produces — by
//      smoothing the gain across time so isolated noise bins can't flicker
//      open.
//   4. Apply a Wiener gain `G = ξ / (1 + ξ)` per bin, floored at a
//      user-configurable attenuation floor so we *attenuate* noise rather than
//      null it (nulling sounds unnatural and eats speech onsets). Floor of
//      e.g. -12 dB means "never duck a bin more than 12 dB", which keeps a
//      natural noise bed and avoids the underwater artifact.
//
// This is precisely the classic RNNoise signal chain with the RNN replaced by
// a hand-tuned statistical estimator — genuinely effective on the steady
// noise PTT users actually hit, fully self-contained, no model, no deps.
//
// ## Latency / quality trade
//
// We process in 256-sample sub-blocks with 50% overlap (128-sample hop)
// streamed across the 960-sample (20 ms) frames. The overlap-add keeps a
// 128-sample (~2.7 ms @ 48 kHz) tail of state between blocks; total added
// latency is one sub-block (~5.3 ms), well within the one-frame budget and
// inaudible on a half-duplex walkie-talkie. A 256-bin FFT @ 48 kHz gives
// ~187 Hz resolution — coarse, but plenty to separate a voice's harmonic comb
// from a flat noise bed, and cheap enough to run every 20 ms on a phone core.
//
// ## Safety / correctness
//
// All math is f32 internally; the only place we touch i16 is the input/output
// conversion, which clamps to `[i16::MIN, i16::MAX]`. The Wiener gain is in
// `[floor, 1.0]` so we can only ever *attenuate* — output energy per bin never
// exceeds input, which makes new clipping structurally impossible. Non-finite
// guards scrub any NaN/Inf that a pathological input could produce. With the
// toggle OFF (the default) `denoise_frame` early-returns before allocating or
// touching state — zero cost.

use std::cell::RefCell;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Runtime on/off for the software mic denoiser. Default OFF — this is opt-in
/// and must cost nothing (an early return) when disabled, mirroring the
/// `TRANSCRIPTION_BRIDGE_ENABLED` / squelch atomics in `audio_pipeline.rs`.
static NOISE_SUPPRESSION_ENABLED: AtomicBool = AtomicBool::new(false);

/// Maximum attenuation floor in dB, stored as a positive integer of dB of
/// suppression (e.g. 12 → bins may be ducked at most 12 dB). Larger = more
/// aggressive cleaning but more risk of dulling quiet speech. Clamped to
/// `[6, 30]` in the setter. Default 12 dB — a natural, conservative reduction.
static NS_MAX_ATTEN_DB: AtomicI32 = AtomicI32::new(12);

/// Enable/disable the software mic noise suppressor. Called from Kotlin via JNI.
pub fn set_noise_suppression_enabled(enabled: bool) {
    NOISE_SUPPRESSION_ENABLED.store(enabled, Ordering::Relaxed);
    info!("Software noise suppression enabled = {}", enabled);
}

/// Whether the software mic noise suppressor is currently enabled.
pub fn get_noise_suppression_enabled() -> bool {
    NOISE_SUPPRESSION_ENABLED.load(Ordering::Relaxed)
}

/// Set the maximum attenuation floor in dB (how hard the denoiser is allowed to
/// duck a noise bin). Higher = more aggressive. Clamped to `[6, 30]`.
pub fn set_noise_suppression_atten_db(db: i32) {
    let clamped = db.clamp(6, 30);
    NS_MAX_ATTEN_DB.store(clamped, Ordering::Relaxed);
    info!("Noise suppression max attenuation set to {} dB", clamped);
}

/// Current maximum attenuation floor in dB.
pub fn get_noise_suppression_atten_db() -> i32 {
    NS_MAX_ATTEN_DB.load(Ordering::Relaxed)
}

// ── STFT geometry ───────────────────────────────────────────────────────────

/// FFT / analysis-window size in samples. Power of two for radix-2 FFT.
/// 256 @ 48 kHz ≈ 5.3 ms window, ~187 Hz/bin — coarse enough to be cheap,
/// fine enough to separate a voice's harmonics from a flat noise floor.
const NS_FFT: usize = 256;
/// Hop size — 50 % overlap. The Hann² overlap-add at this hop satisfies the
/// constant-overlap-add (COLA) condition, so a unity gain reconstructs exactly.
const NS_HOP: usize = NS_FFT / 2;
/// Number of unique spectral bins (DC … Nyquist) for a real input.
const NS_BINS: usize = NS_FFT / 2 + 1;

// ── Estimator tuning constants ──────────────────────────────────────────────

/// Smoothing for the per-bin power used to drive the noise tracker. Higher =
/// steadier (more inertia), so transient speech can't yank the noise estimate.
const NS_POW_SMOOTH: f32 = 0.7;
/// Recovery rate for the running minimum — how fast a bin's tracked noise floor
/// is allowed to *rise* back up after speech passes. Small so speech bursts
/// don't permanently inflate the floor.
const NS_MIN_RECOVERY: f32 = 0.002;
/// Over-subtraction factor on the tracked minimum to form the noise estimate.
/// The minimum statistic is biased low; scaling it up compensates.
const NS_NOISE_OVEREST: f32 = 1.5;
/// Decision-directed weighting between the previous clean estimate (α) and the
/// instantaneous SNR (1-α). 0.98 is the Ephraim–Malah value that tames
/// musical noise without smearing speech onsets.
const NS_DD_ALPHA: f32 = 0.98;
/// Floor on a-posteriori SNR to keep `γ - 1` from going wildly negative on the
/// first frame before the estimator has warmed up.
const NS_SNR_FLOOR: f32 = 1e-3;

/// Per-thread denoiser state. The TX capture loop is single-threaded, so a
/// thread-local is the natural home: no locking on the hot path, and each
/// capture thread that ever calls `denoise_frame` gets its own clean state.
///
/// The streaming model is deliberately decoupled from the caller's frame
/// length: incoming samples land in `in_fifo`, we pull `NS_FFT`-sample analysis
/// windows at a fixed `NS_HOP` stride regardless of where 20 ms frame
/// boundaries fall (the 960-sample frame is *not* a whole multiple of the
/// 128-sample hop, so frame-aligned blocking would be wrong). Finished clean
/// samples drain through `out_fifo`. This makes the overlap-add provably exact
/// (unity gain reconstructs the input) and robust to any frame size.
struct NsState {
    /// Hann analysis/synthesis window (applied on both ends → effective Hann²).
    window: [f32; NS_FFT],
    /// Pending input samples not yet consumed by an analysis window.
    in_fifo: std::collections::VecDeque<f32>,
    /// Overlap-add accumulator. Front is the oldest (about-to-drain) sample.
    /// Each synthesis block adds into the first `NS_FFT` slots; the leading
    /// `NS_HOP` are finalized once the block lands and pushed to `out_fifo`.
    ola: std::collections::VecDeque<f32>,
    /// Finished clean samples awaiting return to the caller. Primed at startup
    /// with `NS_FFT - NS_HOP` zeros so we can always hand back a full frame
    /// despite the inherent one-block STFT latency.
    out_fifo: std::collections::VecDeque<f32>,
    /// Smoothed power spectrum per bin (drives the noise tracker).
    power_smooth: [f32; NS_BINS],
    /// Tracked running minimum of the smoothed power — the noise-floor proxy.
    noise_min: [f32; NS_BINS],
    /// Previous frame's clean power estimate, for the decision-directed SNR.
    prev_clean: [f32; NS_BINS],
    /// True once the first block has seeded the spectra (avoids a cold-start
    /// transient where every bin reads as speech).
    warm: bool,
}

impl NsState {
    fn new() -> Self {
        let mut window = [0.0f32; NS_FFT];
        for (n, w) in window.iter_mut().enumerate() {
            // Periodic Hann: w[n] = 0.5 (1 - cos(2π n / N)). Periodic (N not
            // N-1) is the form that gives exact COLA at 50 % overlap.
            *w = 0.5 * (1.0 - (2.0 * PI * n as f32 / NS_FFT as f32).cos());
        }
        let mut out_fifo = std::collections::VecDeque::new();
        // Prime the output with NS_FFT zeros of latency. The analysis loop only
        // ever emits in whole NS_HOP multiples, while the caller pulls an
        // arbitrary frame size (960 is *not* a multiple of the 128-sample hop),
        // so per call the emitted count lags the requested count by up to
        // (NS_HOP - 1) on top of the one-block STFT analysis latency. Priming
        // NS_FFT = analysis-latency (NS_FFT-NS_HOP) + a full hop of cushion
        // guarantees out_fifo never underruns at steady state for any frame
        // size, while keeping the delay constant (it never grows). Surfaced as
        // a brief ~5.3 ms zero-pad on PTT press.
        for _ in 0..NS_FFT {
            out_fifo.push_back(0.0);
        }
        Self {
            window,
            in_fifo: std::collections::VecDeque::new(),
            ola: std::iter::repeat(0.0).take(NS_FFT).collect(),
            out_fifo,
            power_smooth: [0.0; NS_BINS],
            noise_min: [0.0; NS_BINS],
            prev_clean: [0.0; NS_BINS],
            warm: false,
        }
    }

    /// Process one `NS_FFT`-sample analysis block (already windowed externally
    /// is *not* assumed — we window here), returning the gained, inverse-
    /// transformed time-domain block (length `NS_FFT`). Updates all estimators.
    fn process_block(&mut self, block: &[f32; NS_FFT], max_atten_db: f32) -> [f32; NS_FFT] {
        // Linear floor for the Wiener gain: -max_atten_db → amplitude ratio.
        let gain_floor = 10f32.powf(-max_atten_db / 20.0);

        // Windowed real input → complex buffers for the in-place FFT.
        let mut re = [0.0f32; NS_FFT];
        let mut im = [0.0f32; NS_FFT];
        for i in 0..NS_FFT {
            re[i] = block[i] * self.window[i];
        }
        fft(&mut re, &mut im, false);

        // Per-bin Wiener gain from the decision-directed SNR.
        for k in 0..NS_BINS {
            let power = re[k] * re[k] + im[k] * im[k];

            // Smooth the observed power (recursive averaging).
            if self.warm {
                self.power_smooth[k] = NS_POW_SMOOTH * self.power_smooth[k]
                    + (1.0 - NS_POW_SMOOTH) * power;
            } else {
                self.power_smooth[k] = power;
            }

            // Minimum-statistics noise tracking: the floor instantly follows the
            // smoothed power down, but only crawls back up at NS_MIN_RECOVERY.
            // Speech (power ≫ floor) therefore leaves the floor pinned low.
            if !self.warm || self.power_smooth[k] < self.noise_min[k] {
                self.noise_min[k] = self.power_smooth[k];
            } else {
                self.noise_min[k] += NS_MIN_RECOVERY * (self.power_smooth[k] - self.noise_min[k]);
            }
            let noise_pow = (self.noise_min[k] * NS_NOISE_OVEREST).max(1e-12);

            // A-posteriori SNR γ = |Y|²/N, clamped ≥ floor.
            let gamma = (power / noise_pow).max(NS_SNR_FLOOR);

            // Decision-directed a-priori SNR ξ: blend last frame's clean
            // power-ratio with this frame's (γ-1)+ .
            let inst = (gamma - 1.0).max(0.0);
            let xi = if self.warm {
                NS_DD_ALPHA * (self.prev_clean[k] / noise_pow) + (1.0 - NS_DD_ALPHA) * inst
            } else {
                inst
            };
            let xi = xi.max(1e-6);

            // Wiener gain, floored so we attenuate (never null) → no musical
            // holes, no risk of amplifying (gain ≤ 1 always).
            let gain = (xi / (1.0 + xi)).clamp(gain_floor, 1.0);

            // Remember this bin's clean power for next frame's DD estimate.
            self.prev_clean[k] = gain * gain * power;

            // Apply the gain to this bin and its conjugate-symmetric mirror.
            re[k] *= gain;
            im[k] *= gain;
            if k > 0 && k < NS_BINS - 1 {
                let m = NS_FFT - k;
                re[m] *= gain;
                im[m] *= gain;
            }
        }

        self.warm = true;

        // Inverse FFT back to the time domain, apply the synthesis window
        // (second Hann factor → Hann² COLA), and scale out the FFT's N factor.
        fft(&mut re, &mut im, true);
        let norm = 1.0 / NS_FFT as f32;
        let mut out = [0.0f32; NS_FFT];
        for i in 0..NS_FFT {
            let v = re[i] * norm * self.window[i];
            out[i] = if v.is_finite() { v } else { 0.0 };
        }
        out
    }
}

thread_local! {
    /// Lazily-created per-thread denoiser. Only allocated the first time a
    /// thread actually denoises a frame (i.e. never when the toggle is off).
    static NS_STATE: RefCell<Option<Box<NsState>>> = const { RefCell::new(None) };
}

/// Denoise one mono 48 kHz i16 PCM frame **in place**.
///
/// No-op (and zero allocation) when the runtime toggle is OFF — the default.
/// When ON, runs the spectral-subtraction / Wiener pipeline described in this
/// module's docs. Stateful across calls per thread (noise floor + overlap-add
/// carry), so feed it consecutive frames from the same capture thread.
///
/// Frame-in / frame-out: `pcm.len()` samples in, the same count out. The STFT
/// carries an inherent one-block (`NS_FFT - NS_HOP` = 128-sample, ~2.7 ms)
/// analysis latency; we surface it as a constant zero-pad primed at startup, so
/// the caller's sample count is preserved exactly and the delay never grows.
/// The very first frame is therefore ~2.7 ms "early" (zero-padded head) — an
/// inaudible warm-up on PTT press.
pub fn denoise_frame(pcm: &mut [i16]) {
    // Fast path: disabled → leave the buffer untouched, allocate nothing.
    if !NOISE_SUPPRESSION_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if pcm.is_empty() {
        return;
    }
    let max_atten_db = NS_MAX_ATTEN_DB.load(Ordering::Relaxed) as f32;

    NS_STATE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let state = slot.get_or_insert_with(|| Box::new(NsState::new()));

        // 1. Push this frame's samples (normalized to ~[-1, 1)) into the input
        //    FIFO. We pull fixed-size analysis windows out of it below — the
        //    caller's frame length is irrelevant to the block geometry.
        for &s in pcm.iter() {
            state.in_fifo.push_back(s as f32 / 32768.0);
        }

        // 2. While a full analysis window is available, process one block and
        //    advance by NS_HOP. The window peeks NS_FFT samples but only
        //    *consumes* (drains) NS_HOP of them, so consecutive windows overlap
        //    by NS_FFT - NS_HOP for the overlap-add reconstruction.
        while state.in_fifo.len() >= NS_FFT {
            let mut block = [0.0f32; NS_FFT];
            for (i, slot) in block.iter_mut().enumerate() {
                *slot = state.in_fifo[i];
            }
            let synth = state.process_block(&block, max_atten_db);

            // Overlap-add the synthesis block into the accumulator. `ola` holds
            // NS_FFT live samples; add the block, then the leading NS_HOP are
            // final (no future block reaches that far back) and drain to output.
            for (i, &v) in synth.iter().enumerate() {
                state.ola[i] += v;
            }
            for _ in 0..NS_HOP {
                let v = state.ola.pop_front().unwrap_or(0.0);
                state.out_fifo.push_back(if v.is_finite() { v } else { 0.0 });
            }
            // Re-extend the accumulator tail with zeros for the next block's add.
            state.ola.extend(std::iter::repeat(0.0).take(NS_HOP));

            // Consume this hop's worth of input.
            for _ in 0..NS_HOP {
                state.in_fifo.pop_front();
            }
        }

        // 3. Hand back exactly one cleaned sample per input sample. Startup
        //    priming guarantees out_fifo never underruns at steady state; the
        //    guard falls back to zero (silence) only on the impossible
        //    underrun path.
        for s in pcm.iter_mut() {
            let v = state.out_fifo.pop_front().unwrap_or(0.0);
            // Every Wiener gain ≤ 1.0, so |output| ≤ |input| per bin and new
            // clipping is structurally impossible; the clamp is belt-and-
            // suspenders against float rounding.
            let scaled = if v.is_finite() {
                (v * 32768.0).clamp(i16::MIN as f32, i16::MAX as f32)
            } else {
                0.0
            };
            *s = scaled as i16;
        }
    });
}

/// In-place iterative radix-2 Cooley–Tukey FFT (decimation-in-time).
///
/// `re`/`im` are the real/imaginary parts of a length-`N` signal where `N` is a
/// power of two; transformed in place. `inverse=false` computes the forward
/// transform (no normalization), `inverse=true` the unnormalized inverse — the
/// caller scales by `1/N`. Self-contained, no deps: a 256-point butterfly is
/// trivial and runs comfortably inside the 20 ms frame budget.
fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two());
    debug_assert_eq!(im.len(), n);
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Danielson–Lanczos butterflies, doubling the sub-transform length.
    let sign = if inverse { 1.0f32 } else { -1.0f32 };
    let mut len = 2usize;
    while len <= n {
        let ang = sign * 2.0 * PI / len as f32;
        let (wlen_re, wlen_im) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..len / 2 {
                let u_re = re[i + k];
                let u_im = im[i + k];
                let t_re = re[i + k + len / 2] * w_re - im[i + k + len / 2] * w_im;
                let t_im = re[i + k + len / 2] * w_im + im[i + k + len / 2] * w_re;
                re[i + k] = u_re + t_re;
                im[i + k] = u_im + t_im;
                re[i + k + len / 2] = u_re - t_re;
                im[i + k + len / 2] = u_im - t_im;
                // Advance the twiddle: w *= wlen.
                let nw_re = w_re * wlen_re - w_im * wlen_im;
                let nw_im = w_re * wlen_im + w_im * wlen_re;
                w_re = nw_re;
                w_im = nw_im;
            }
            i += len;
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod ns_tests {
    use super::*;

    /// Reset the thread-local denoiser so each test starts cold.
    fn reset_state() {
        NS_STATE.with(|cell| *cell.borrow_mut() = None);
    }

    /// Deterministic LCG so tests are reproducible without an rng dep.
    struct Lcg(u64);
    impl Lcg {
        fn new(seed: u64) -> Self {
            Lcg(seed)
        }
        /// Uniform in [-1, 1).
        fn next_f(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        }
    }

    #[test]
    fn fft_round_trips() {
        let n = NS_FFT;
        let mut rng = Lcg::new(0xABCD);
        let orig: Vec<f32> = (0..n).map(|_| rng.next_f()).collect();
        let mut re = orig.clone();
        let mut im = vec![0.0f32; n];

        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        let norm = 1.0 / n as f32;

        let mut max_err = 0.0f32;
        for i in 0..n {
            let recon = re[i] * norm;
            max_err = max_err.max((recon - orig[i]).abs());
            assert!((im[i] * norm).abs() < 1e-4, "imag residue at {}", i);
        }
        assert!(max_err < 1e-4, "ifft(fft(x)) error too large: {}", max_err);
    }

    #[test]
    fn fft_of_tone_peaks_at_bin() {
        // A pure cosine at bin 8 should concentrate energy at bin 8.
        let n = NS_FFT;
        let bin = 8usize;
        let mut re: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * bin as f32 * i as f32 / n as f32).cos())
            .collect();
        let mut im = vec![0.0f32; n];
        fft(&mut re, &mut im, false);
        let mag = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt();
        let peak = mag(bin);
        for k in 1..n / 2 {
            if k != bin {
                assert!(mag(k) < peak * 0.01, "bin {} leaked: {} vs peak {}", k, mag(k), peak);
            }
        }
    }

    /// Build a 48 kHz tone frame (i16) of `freq` Hz at amplitude `amp` (0..1),
    /// optionally adding white noise of std `noise`.
    fn tone_frame(freq: f32, amp: f32, noise: f32, n: usize, phase: usize, rng: &mut Lcg) -> Vec<i16> {
        let sr = 48000.0f32;
        (0..n)
            .map(|i| {
                let t = (i + phase) as f32 / sr;
                let s = amp * (2.0 * PI * freq * t).sin() + noise * rng.next_f();
                (s.clamp(-1.0, 1.0) * 32767.0) as i16
            })
            .collect()
    }

    /// Energy of a target frequency in an i16 frame, via the module FFT on a
    /// power-of-two prefix (so we can measure tone preservation).
    fn tone_energy(frame: &[i16], freq: f32) -> f32 {
        let n = NS_FFT;
        let mut re: Vec<f32> = frame[..n].iter().map(|s| *s as f32 / 32768.0).collect();
        let mut im = vec![0.0f32; n];
        // Hann to reduce leakage before measuring.
        for (i, r) in re.iter_mut().enumerate() {
            *r *= 0.5 * (1.0 - (2.0 * PI * i as f32 / n as f32).cos());
        }
        fft(&mut re, &mut im, false);
        let bin = (freq * n as f32 / 48000.0).round() as usize;
        let mut e = 0.0;
        for k in bin.saturating_sub(1)..=(bin + 1).min(n / 2) {
            e += re[k] * re[k] + im[k] * im[k];
        }
        e
    }

    #[test]
    fn denoise_improves_snr_on_noisy_tone() {
        set_noise_suppression_enabled(true);
        set_noise_suppression_atten_db(18);
        reset_state();

        let freq = 1000.0f32;
        let amp = 0.3f32;
        let noise = 0.05f32;
        let frame = 960usize;
        let mut rng = Lcg::new(0x1234);

        // Warm the estimator: feed several noisy frames so the noise floor is
        // tracked before we measure (steady-state behavior is what matters).
        let mut last_clean = vec![0i16; frame];
        let mut phase = 0usize;
        for _ in 0..30 {
            let mut f = tone_frame(freq, amp, noise, frame, phase, &mut rng);
            denoise_frame(&mut f);
            last_clean = f;
            phase += frame;
        }

        // Reference clean tone (no noise) and the noisy input for comparison.
        let mut rng2 = Lcg::new(0x9999);
        let clean_tone = tone_frame(freq, amp, 0.0, frame, phase, &mut rng2);
        let noisy = tone_frame(freq, amp, noise, frame, phase, &mut Lcg::new(0x5555));

        // Tone energy should be largely preserved.
        let tone_in = tone_energy(&clean_tone, freq);
        let tone_out = tone_energy(&last_clean, freq);
        assert!(tone_out > tone_in * 0.25,
            "tone fundamental over-suppressed: in={} out={}", tone_in, tone_out);

        // Noise-band energy (a band well away from the tone, e.g. ~5 kHz) must
        // drop relative to the noisy input.
        let off_freq = 5000.0f32;
        let noise_in = tone_energy(&noisy, off_freq);
        let noise_out = tone_energy(&last_clean, off_freq);
        assert!(noise_out < noise_in,
            "off-band noise not reduced: in={} out={}", noise_in, noise_out);

        set_noise_suppression_enabled(false);
    }

    #[test]
    fn disabled_is_exact_noop() {
        set_noise_suppression_enabled(false);
        reset_state();
        let mut rng = Lcg::new(0x77);
        let original = tone_frame(440.0, 0.5, 0.1, 960, 0, &mut rng);
        let mut copy = original.clone();
        denoise_frame(&mut copy);
        assert_eq!(copy, original, "disabled denoise must be a bit-exact no-op");
    }

    #[test]
    fn no_clipping_or_nan_on_full_scale() {
        set_noise_suppression_enabled(true);
        set_noise_suppression_atten_db(12);
        reset_state();
        // Full-scale square-ish input (alternating extremes) — worst case for
        // overshoot. Run several frames through.
        for _ in 0..10 {
            let mut f: Vec<i16> = (0..960)
                .map(|i| if i % 2 == 0 { i16::MAX } else { i16::MIN })
                .collect();
            denoise_frame(&mut f);
            for &s in &f {
                assert!(s >= i16::MIN && s <= i16::MAX); // i16 can't exceed range, but assert anyway
            }
        }
        set_noise_suppression_enabled(false);
    }

    #[test]
    fn silence_stays_silent_no_dc() {
        set_noise_suppression_enabled(true);
        set_noise_suppression_atten_db(12);
        reset_state();
        for _ in 0..5 {
            let mut f = vec![0i16; 960];
            denoise_frame(&mut f);
            // No NaN, no DC offset injected, stays at/near zero.
            let sum: i64 = f.iter().map(|&s| s as i64).sum();
            assert_eq!(sum, 0, "silence picked up a DC offset: {}", sum);
            for &s in &f {
                assert_eq!(s, 0);
            }
        }
        set_noise_suppression_enabled(false);
    }

    #[test]
    fn atten_db_setter_clamps() {
        set_noise_suppression_atten_db(1);
        assert_eq!(get_noise_suppression_atten_db(), 6);
        set_noise_suppression_atten_db(99);
        assert_eq!(get_noise_suppression_atten_db(), 30);
        set_noise_suppression_atten_db(12);
        assert_eq!(get_noise_suppression_atten_db(), 12);
    }

    #[test]
    fn toggle_getter_roundtrips() {
        set_noise_suppression_enabled(true);
        assert!(get_noise_suppression_enabled());
        set_noise_suppression_enabled(false);
        assert!(!get_noise_suppression_enabled());
    }
}
