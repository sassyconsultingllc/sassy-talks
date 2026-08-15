// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! diag — live counters for the diagnostics panel and relay transfer tracing.
//!
//! WHY THIS EXISTS
//!
//! Two problems it solves.
//!
//! 1. **The panel's CAPTURE / CODEC / GATE sections could never update.** They
//!    were fed exclusively by the Kotlin `PttAudioPipeline`, which is never
//!    constructed anywhere in the app — capture actually lives here in Rust.
//!    Kotlin cannot observe frames it never touches, so those sections showed
//!    boot-time defaults forever. Counters have to originate where the work
//!    happens, which is this crate.
//!
//! 2. **"Audio isn't getting through the relay" was undiagnosable.** The
//!    existing relay stats count packets that arrive *on the wire*
//!    (`packets_received`), not packets that survive AEAD. Those two numbers
//!    diverging is the single most common silent failure — two peers on
//!    different session keys exchange traffic all day and hear nothing, while
//!    every visible counter looks healthy. Splitting decrypt outcomes apart
//!    turns that from a mystery into a one-line answer.
//!
//! Design: process-global lock-free atomics, `Relaxed` ordering. These are
//! diagnostics, not control flow — no reader depends on ordering between
//! counters, and the audio TX path must not pay for a mutex or a fence.
//! Everything here is written from the hot path and read ~1/s by the UI.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

/// Mic frames handed to the encoder.
static FRAMES_CAPTURED: AtomicU64 = AtomicU64::new(0);
/// Frames the codec produced output for.
static FRAMES_ENCODED: AtomicU64 = AtomicU64::new(0);
/// Cumulative encoded payload bytes (pre-encryption, pre-framing).
static ENCODED_BYTES: AtomicU64 = AtomicU64::new(0);
/// Most recent capture level in centi-dBFS (dBFS * 100) so it fits an integer
/// atomic without a float. -12000 = silence floor.
static LAST_CAPTURE_DBFS_CENTI: AtomicI32 = AtomicI32::new(-12_000);

/// Inbound frames that decrypted and authenticated cleanly.
static DECRYPT_OK: AtomicU64 = AtomicU64::new(0);
/// Inbound frames that failed the GCM tag — almost always a session-key
/// mismatch between peers, occasionally corruption on the wire.
static DECRYPT_FAIL: AtomicU64 = AtomicU64::new(0);
/// Inbound frames dropped because this device has no session key at all.
static DECRYPT_NO_SESSION: AtomicU64 = AtomicU64::new(0);
/// Authentic frames rejected as replays (nonce already seen).
static REPLAY_REJECTED: AtomicU64 = AtomicU64::new(0);

/// Wall-clock ms of the last outbound audio send and last successful inbound
/// decrypt. Counters alone can't distinguish "500 frames, all an hour ago"
/// from "500 frames, still flowing" — which is exactly the question being
/// asked when someone reports one-way audio.
static LAST_TX_MS: AtomicU64 = AtomicU64::new(0);
static LAST_RX_OK_MS: AtomicU64 = AtomicU64::new(0);
static TX_QUEUED: AtomicU64 = AtomicU64::new(0);
static TX_SUCCEEDED: AtomicU64 = AtomicU64::new(0);
static TX_FAILED: AtomicU64 = AtomicU64::new(0);

/// Debug-only crypto tracing. When on, the first bytes of each audio frame are
/// logged BEFORE and AFTER encryption so an operator can confirm with their own
/// eyes that what leaves the device is not the plaintext.
///
/// Gated behind an explicit runtime toggle AND compiled behaviour that only the
/// diagnostic tooling enables: this logs fragments of live microphone audio, so
/// it must never be on during normal use.
static CRYPTO_TRACE: AtomicBool = AtomicBool::new(false);
static TRACE_REMAINING: AtomicI32 = AtomicI32::new(0);

pub fn set_crypto_trace(on: bool, frames: i32) {
    // Production/release .so: no-op even if JNI is invoked. Debug APKs that
    // ship a --release native library also cannot dump mic prefixes.
    if !cfg!(debug_assertions) {
        let _ = (on, frames);
        return;
    }
    CRYPTO_TRACE.store(on, Ordering::Relaxed);
    TRACE_REMAINING.store(if on { frames } else { 0 }, Ordering::Relaxed);
}

pub fn crypto_trace_enabled() -> bool {
    if !cfg!(debug_assertions) {
        return false;
    }
    CRYPTO_TRACE.load(Ordering::Relaxed) && TRACE_REMAINING.load(Ordering::Relaxed) > 0
}

fn hex_prefix(b: &[u8], n: usize) -> String {
    b.iter()
        .take(n)
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Log one frame's plaintext and ciphertext prefixes. Bounded by the frame
/// budget passed to [set_crypto_trace] so a forgotten toggle cannot stream
/// audio fragments into logcat indefinitely.
pub fn trace_crypto(plain: &[u8], cipher: &[u8]) {
    if !cfg!(debug_assertions) {
        let _ = (plain, cipher);
        return;
    }
    if !crypto_trace_enabled() {
        return;
    }
    let left = TRACE_REMAINING.fetch_sub(1, Ordering::Relaxed);
    if left <= 0 {
        return;
    }
    log::info!(
        "CRYPTO_TRACE frame#{} PLAIN len={} [{}] -> CIPHER len={} [{}] (nonce is the leading 12B)",
        left,
        plain.len(),
        hex_prefix(plain, 16),
        cipher.len(),
        hex_prefix(cipher, 16),
    );
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Producers (hot path) ────────────────────────────────────────────────────

/// One mic frame captured. `dbfs` is the frame's level in dBFS.
#[inline]
pub fn note_capture(dbfs: f32) {
    FRAMES_CAPTURED.fetch_add(1, Ordering::Relaxed);
    LAST_CAPTURE_DBFS_CENTI.store((dbfs * 100.0) as i32, Ordering::Relaxed);
}

/// One frame encoded, producing `bytes` of compressed payload.
#[inline]
pub fn note_encode(bytes: usize) {
    FRAMES_ENCODED.fetch_add(1, Ordering::Relaxed);
    ENCODED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

/// One audio payload handed to transport fanout (not proof of socket send).
#[inline]
pub fn note_tx() {
    TX_QUEUED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_tx_success() {
    TX_SUCCEEDED.fetch_add(1, Ordering::Relaxed);
    LAST_TX_MS.store(now_ms(), Ordering::Relaxed);
}

#[inline]
pub fn note_tx_failure() {
    TX_FAILED.fetch_add(1, Ordering::Relaxed);
}

/// Inbound frame decrypted and authenticated.
#[inline]
pub fn note_decrypt_ok() {
    DECRYPT_OK.fetch_add(1, Ordering::Relaxed);
    LAST_RX_OK_MS.store(now_ms(), Ordering::Relaxed);
}

/// Inbound frame failed to decrypt. Classified from the error text so the
/// panel can separate a key mismatch (the common, actionable case) from a
/// replay rejection (benign — duplicate delivery).
#[inline]
pub fn note_decrypt_fail(err: &str) {
    if err.contains("Replay") {
        REPLAY_REJECTED.fetch_add(1, Ordering::Relaxed);
    } else {
        DECRYPT_FAIL.fetch_add(1, Ordering::Relaxed);
    }
}

/// Inbound frame dropped: no session key on this device.
#[inline]
pub fn note_decrypt_no_session() {
    DECRYPT_NO_SESSION.fetch_add(1, Ordering::Relaxed);
}

// ── Consumer ────────────────────────────────────────────────────────────────

/// Snapshot every counter as JSON for the diagnostics panel / adb dump.
///
/// `*_age_ms` fields are -1 when the event has never happened, so a reader can
/// tell "never" apart from "just now" without a second field.
pub fn snapshot_json() -> String {
    let now = now_ms();
    let age = |t: u64| -> i64 {
        if t == 0 {
            -1
        } else {
            (now.saturating_sub(t)) as i64
        }
    };

    let ok = DECRYPT_OK.load(Ordering::Relaxed);
    let fail = DECRYPT_FAIL.load(Ordering::Relaxed);
    let no_sess = DECRYPT_NO_SESSION.load(Ordering::Relaxed);
    let replay = REPLAY_REJECTED.load(Ordering::Relaxed);

    // The headline triage number: of everything that arrived and was handed to
    // the AEAD, what fraction actually opened. A high rate with zero ok is the
    // "peers on different session keys" signature.
    let attempted = ok + fail + no_sess + replay;
    let ok_pct = if attempted == 0 {
        -1.0
    } else {
        (ok as f64 * 100.0) / attempted as f64
    };

    format!(
        concat!(
            r#"{{"capture":{{"frames":{},"last_dbfs":{:.1}}},"#,
            r#""codec":{{"frames_encoded":{},"encoded_bytes":{},"avg_frame_bytes":{}}},"#,
            r#""crypto_rx":{{"ok":{},"fail":{},"no_session":{},"replay":{},"attempted":{},"ok_pct":{:.1}}},"#,
            r#""tx":{{"queued":{},"succeeded":{},"failed":{}}},"#,
            r#""activity":{{"last_tx_success_age_ms":{},"last_tx_age_ms":{},"last_rx_ok_age_ms":{}}}}}"#
        ),
        FRAMES_CAPTURED.load(Ordering::Relaxed),
        LAST_CAPTURE_DBFS_CENTI.load(Ordering::Relaxed) as f32 / 100.0,
        FRAMES_ENCODED.load(Ordering::Relaxed),
        ENCODED_BYTES.load(Ordering::Relaxed),
        {
            let f = FRAMES_ENCODED.load(Ordering::Relaxed);
            if f == 0 {
                0
            } else {
                ENCODED_BYTES.load(Ordering::Relaxed) / f
            }
        },
        ok,
        fail,
        no_sess,
        replay,
        attempted,
        ok_pct,
        TX_QUEUED.load(Ordering::Relaxed),
        TX_SUCCEEDED.load(Ordering::Relaxed),
        TX_FAILED.load(Ordering::Relaxed),
        age(LAST_TX_MS.load(Ordering::Relaxed)),
        age(LAST_TX_MS.load(Ordering::Relaxed)),
        age(LAST_RX_OK_MS.load(Ordering::Relaxed)),
    )
}

/// Zero every counter. Used by the diagnostic tooling to take a clean baseline
/// before a test transmission, so a reading reflects THIS run rather than the
/// whole process lifetime.
pub fn reset() {
    for c in [
        &FRAMES_CAPTURED,
        &FRAMES_ENCODED,
        &ENCODED_BYTES,
        &DECRYPT_OK,
        &DECRYPT_FAIL,
        &DECRYPT_NO_SESSION,
        &REPLAY_REJECTED,
        &LAST_TX_MS,
        &LAST_RX_OK_MS,
        &TX_QUEUED,
        &TX_SUCCEEDED,
        &TX_FAILED,
    ] {
        c.store(0, Ordering::Relaxed);
    }
    LAST_CAPTURE_DBFS_CENTI.store(-12_000, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These counters are process-global by design, so the tests that reset
    /// and read them cannot run concurrently — under the default parallel
    /// test runner they stomp each other's baselines and fail at random.
    /// Serialise them here rather than requiring `--test-threads=1`, which
    /// would silently not apply in CI.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn snapshot_is_valid_json_when_empty() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let s = snapshot_json();
        assert!(s.starts_with('{') && s.ends_with('}'));
        // Never-happened events report -1, not 0, so "never" is distinguishable.
        assert!(s.contains(r#""last_tx_age_ms":-1"#));
        assert!(s.contains(r#""ok_pct":-1.0"#));
    }

    #[test]
    fn decrypt_outcomes_are_classified() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        note_decrypt_ok();
        note_decrypt_fail("Decryption failed: aead::Error");
        note_decrypt_fail("Replay detected: nonce already seen");
        note_decrypt_no_session();
        let s = snapshot_json();
        assert!(s.contains(r#""ok":1"#), "{s}");
        assert!(s.contains(r#""fail":1"#), "{s}");
        assert!(s.contains(r#""replay":1"#), "{s}");
        assert!(s.contains(r#""no_session":1"#), "{s}");
        assert!(s.contains(r#""attempted":4"#), "{s}");
    }

    #[test]
    fn encode_tracks_average_frame_size() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        note_encode(100);
        note_encode(200);
        let s = snapshot_json();
        assert!(s.contains(r#""frames_encoded":2"#), "{s}");
        assert!(s.contains(r#""encoded_bytes":300"#), "{s}");
        assert!(s.contains(r#""avg_frame_bytes":150"#), "{s}");
    }

    #[test]
    fn tx_success_is_distinct_from_queueing() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        note_tx();
        note_tx_failure();
        note_tx_success();
        let s = snapshot_json();
        assert!(
            s.contains(r#""tx":{"queued":1,"succeeded":1,"failed":1}"#),
            "{s}"
        );
        assert!(!s.contains(r#""last_tx_success_age_ms":-1"#), "{s}");
    }
}
