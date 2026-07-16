// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-WEEXX3XNKKW2
//! emergency — SOS / man-down / emergency-broadcast signalling (pure logic).
//!
//! **INTEGRATION STATUS (staged, not yet wired):** the state machines and the
//! `EmergencySignal` TLV codec compile and self-test, but no consumer drives
//! them yet — pressing SOS or a fall does NOT currently produce a wire frame.
//! Activation needs a consumer-side wiring on each platform: feed sensor samples
//! into `ManDownDetector`, drive `EmergencyState::tick`, broadcast the encoded
//! `EmergencySignal` over the transport, and promote the provisional `OP_*`
//! opcodes here into `crate::protocol`. Treat these features as inert until then.
//!
//! Owns three things, none of which touch a sensor or a socket:
//!   1. New TLV opcodes + a wire payload [`EmergencySignal`] with
//!      `encode()` / `decode()` round-trip, framed through the shared
//!      `crate::protocol` TLV helpers so it stays in lockstep with every other
//!      opcode.
//!   2. [`EmergencyState`] — a manual-SOS controller with an auto-repeat
//!      beacon cadence (emergency frames re-broadcast every N seconds until
//!      cleared), driven by `tick(now_ms)`.
//!   3. [`ManDownDetector`] — a deterministic state machine fed periodic
//!      motion/orientation samples; it trips man-down when the device is
//!      tilted past a threshold AND has been motionless past a timeout, with a
//!      user-cancellable pre-alarm grace window.
//!
//! Why here and not in the consumer crates: the wire format and the trip logic
//! must be byte-identical and behaviour-identical across Android / desktop /
//! iOS, and must be unit-testable with an injected clock. The consumer crates
//! supply only the platform glue — accelerometer samples in, TLV frames out.
//!
//! ── NEW OPCODES (provisional — promote into `protocol.rs` later) ───────────
//! These live here to avoid editing `protocol.rs` while it's owned elsewhere.
//! They are deliberately in the TLV-framed `>= 0x10` range and do NOT collide
//! with the in-use set (0x01,0x02,0x10,0x14,0x15,0x16,0x17,0x19). When merged,
//! move these three constants into `protocol.rs` next to `OP_REPLAY_FRAME` and
//! delete them here.
//!
//!   `OP_EMERGENCY`       = 0x1A  → an SOS / man-down / custom distress beacon.
//!   `OP_MANDOWN`         = 0x1B  → man-down auto-trip beacon (semantic alias
//!                                  carrying the same [`EmergencySignal`] body
//!                                  but distinguishing an *automatic* man-down
//!                                  trip from a *manual* SOS press on the wire).
//!   `OP_EMERGENCY_CLEAR` = 0x1C  → "I'm OK / cancel" — stand-down for a prior
//!                                  beacon from this sender. Carries just the
//!                                  sender id + timestamp.

use crate::protocol;

/// SOS / man-down / custom distress beacon. Manual SOS press.
/// Payload is an [`EmergencySignal`].
pub const OP_EMERGENCY: u8 = 0x1A;
/// Automatic man-down trip beacon. Payload is an [`EmergencySignal`] whose
/// `kind` is [`EmergencyKind::ManDown`]; the distinct opcode lets a relay /
/// receiver special-case auto-trips (louder alert, no de-dupe with a manual
/// SOS) without parsing the body first.
pub const OP_MANDOWN: u8 = 0x1B;
/// Stand-down / "I'm OK". Payload is an [`EmergencyClear`].
pub const OP_EMERGENCY_CLEAR: u8 = 0x1C;

/// Protocol/format version stamped into the [`EmergencySignal`] header so the
/// payload can grow fields later without a receiver misreading an old frame.
/// Receivers reject a version they don't understand rather than guessing.
const SIGNAL_VERSION: u8 = 1;
const CLEAR_VERSION: u8 = 1;

/// Scale factor for the fixed-point lat/lon encoding. Coordinates are stored as
/// `degrees * 1e7` in an `i32`, the standard 1e-7° (~1.1 cm) GPS fixed-point
/// representation. `i32` range (±214.7) comfortably covers ±180° longitude.
const COORD_SCALE: f64 = 1e7;

/// Max length of the optional free-text note, in UTF-8 bytes. Bounded so a
/// single emergency frame stays inside one transport datagram and a hostile
/// peer can't make us allocate unboundedly. 255 fits the `u8` length prefix.
pub const MAX_NOTE_LEN: usize = 255;

/// Kind of distress signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyKind {
    /// Manual SOS — the user deliberately triggered a distress call.
    Sos,
    /// Automatic man-down — tripped by [`ManDownDetector`] with no user input.
    ManDown,
    /// Application-defined variant; the `u8` discriminates client-side.
    Custom(u8),
}

impl EmergencyKind {
    /// Wire byte for the kind. `Custom(n)` reserves the high half (>=0x80) so
    /// it can never collide with a future built-in kind in the low half.
    fn to_byte(self) -> u8 {
        match self {
            EmergencyKind::Sos => 0x01,
            EmergencyKind::ManDown => 0x02,
            // 0x80 | n keeps custom kinds clearly separated from built-ins and
            // round-trips the low 7 bits of the discriminator.
            EmergencyKind::Custom(n) => 0x80 | (n & 0x7F),
        }
    }

    fn from_byte(b: u8) -> EmergencyKind {
        match b {
            0x01 => EmergencyKind::Sos,
            0x02 => EmergencyKind::ManDown,
            other if other & 0x80 != 0 => EmergencyKind::Custom(other & 0x7F),
            // Unknown low-half byte from a newer peer: treat as a generic SOS
            // rather than dropping a life-safety frame on the floor.
            _ => EmergencyKind::Sos,
        }
    }
}

/// A geographic fix as fixed-point degrees (`lat`, `lon` each `degrees * 1e7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedCoord {
    pub lat_e7: i32,
    pub lon_e7: i32,
}

impl FixedCoord {
    /// Build from floating-point degrees, rounding to the 1e-7° grid.
    pub fn from_degrees(lat: f64, lon: f64) -> FixedCoord {
        FixedCoord {
            lat_e7: (lat * COORD_SCALE).round() as i32,
            lon_e7: (lon * COORD_SCALE).round() as i32,
        }
    }

    /// Back to floating-point degrees.
    pub fn to_degrees(self) -> (f64, f64) {
        (self.lat_e7 as f64 / COORD_SCALE, self.lon_e7 as f64 / COORD_SCALE)
    }
}

/// The emergency-beacon payload.
///
/// Wire layout of the TLV *payload* (the bytes after `[op][len:u16]`), all
/// multi-byte integers little-endian to match `protocol::encode_tlv`:
/// ```text
///   [version:u8]                       = SIGNAL_VERSION (1)
///   [kind:u8]                          EmergencyKind byte (see to_byte)
///   [flags:u8]                         bit0 = has_coord, bit1 = has_note
///   [timestamp_ms:u64 LE]              sender wall/elapsed clock
///   [sender_id_len:u8][sender_id ...]  UTF-8, <= 255 bytes
///   [lat_e7:i32 LE][lon_e7:i32 LE]     present iff flags.bit0 (has_coord)
///   [note_len:u8][note ...]            present iff flags.bit1, UTF-8 <=255
/// ```
/// `decode` validates every length against the remaining buffer before
/// reading, mirroring the defensive style in `protocol.rs` (a truncated or
/// over-long frame yields `Err`, never a panic or out-of-bounds read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencySignal {
    pub sender_id: String,
    pub kind: EmergencyKind,
    pub timestamp_ms: u64,
    pub coord: Option<FixedCoord>,
    pub note: Option<String>,
}

const FLAG_HAS_COORD: u8 = 0b0000_0001;
const FLAG_HAS_NOTE: u8 = 0b0000_0010;

impl EmergencySignal {
    /// Construct a signal. The note (if any) is truncated to [`MAX_NOTE_LEN`]
    /// bytes on a UTF-8 char boundary so encoding can never overflow the `u8`
    /// length prefix.
    pub fn new(
        sender_id: impl Into<String>,
        kind: EmergencyKind,
        timestamp_ms: u64,
        coord: Option<FixedCoord>,
        note: Option<String>,
    ) -> EmergencySignal {
        let note = note.map(|n| truncate_utf8(&n, MAX_NOTE_LEN));
        EmergencySignal {
            sender_id: sender_id.into(),
            kind,
            timestamp_ms,
            coord,
            note,
        }
    }

    /// The opcode this signal should be framed under: man-down auto-trips use
    /// the dedicated [`OP_MANDOWN`]; everything else uses [`OP_EMERGENCY`].
    pub fn opcode(&self) -> u8 {
        match self.kind {
            EmergencyKind::ManDown => OP_MANDOWN,
            _ => OP_EMERGENCY,
        }
    }

    /// Serialize the payload bytes (no TLV header).
    fn encode_payload(&self) -> Vec<u8> {
        let mut flags = 0u8;
        if self.coord.is_some() {
            flags |= FLAG_HAS_COORD;
        }
        if self.note.is_some() {
            flags |= FLAG_HAS_NOTE;
        }

        let sender = self.sender_id.as_bytes();
        // sender_id length prefix is a u8; clamp defensively.
        let sender_len = sender.len().min(u8::MAX as usize);

        let mut p = Vec::with_capacity(3 + 8 + 1 + sender_len + 8 + 1 + 32);
        p.push(SIGNAL_VERSION);
        p.push(self.kind.to_byte());
        p.push(flags);
        p.extend_from_slice(&self.timestamp_ms.to_le_bytes());
        p.push(sender_len as u8);
        p.extend_from_slice(&sender[..sender_len]);
        if let Some(c) = self.coord {
            p.extend_from_slice(&c.lat_e7.to_le_bytes());
            p.extend_from_slice(&c.lon_e7.to_le_bytes());
        }
        if let Some(ref n) = self.note {
            let nb = n.as_bytes();
            let nlen = nb.len().min(MAX_NOTE_LEN);
            p.push(nlen as u8);
            p.extend_from_slice(&nb[..nlen]);
        }
        p
    }

    /// Encode to a full TLV wire frame (`[op][len:u16 LE][payload]`) via
    /// `crate::protocol::encode_tlv`, using the kind-appropriate opcode.
    pub fn encode(&self) -> Vec<u8> {
        protocol::encode_tlv(self.opcode(), &self.encode_payload())
    }

    /// Decode from a TLV *payload* (the bytes the caller already extracted via
    /// `protocol::parse_tlv`, i.e. `tlv.payload`). Returns `Err` on any length
    /// inconsistency, unknown version, or invalid UTF-8 — never panics.
    pub fn decode(payload: &[u8]) -> Result<EmergencySignal, String> {
        let mut r = Reader::new(payload);
        let version = r.u8("version")?;
        if version != SIGNAL_VERSION {
            return Err(format!(
                "EmergencySignal: unsupported version {version} (expected {SIGNAL_VERSION})"
            ));
        }
        let kind = EmergencyKind::from_byte(r.u8("kind")?);
        let flags = r.u8("flags")?;
        let timestamp_ms = r.u64_le("timestamp")?;
        let sender_len = r.u8("sender_id_len")? as usize;
        let sender_bytes = r.take(sender_len, "sender_id")?;
        let sender_id = String::from_utf8(sender_bytes.to_vec())
            .map_err(|_| "EmergencySignal: sender_id is not valid UTF-8".to_string())?;

        let coord = if flags & FLAG_HAS_COORD != 0 {
            let lat_e7 = r.i32_le("lat_e7")?;
            let lon_e7 = r.i32_le("lon_e7")?;
            Some(FixedCoord { lat_e7, lon_e7 })
        } else {
            None
        };

        let note = if flags & FLAG_HAS_NOTE != 0 {
            let nlen = r.u8("note_len")? as usize;
            let nb = r.take(nlen, "note")?;
            Some(
                String::from_utf8(nb.to_vec())
                    .map_err(|_| "EmergencySignal: note is not valid UTF-8".to_string())?,
            )
        } else {
            None
        };

        Ok(EmergencySignal {
            sender_id,
            kind,
            timestamp_ms,
            coord,
            note,
        })
    }
}

/// Stand-down / "I'm OK" payload for [`OP_EMERGENCY_CLEAR`].
///
/// Wire layout of the TLV payload:
/// ```text
///   [version:u8] = CLEAR_VERSION (1)
///   [timestamp_ms:u64 LE]
///   [sender_id_len:u8][sender_id ... UTF-8]
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyClear {
    pub sender_id: String,
    pub timestamp_ms: u64,
}

impl EmergencyClear {
    pub fn new(sender_id: impl Into<String>, timestamp_ms: u64) -> EmergencyClear {
        EmergencyClear {
            sender_id: sender_id.into(),
            timestamp_ms,
        }
    }

    fn encode_payload(&self) -> Vec<u8> {
        let sender = self.sender_id.as_bytes();
        let sender_len = sender.len().min(u8::MAX as usize);
        let mut p = Vec::with_capacity(1 + 8 + 1 + sender_len);
        p.push(CLEAR_VERSION);
        p.extend_from_slice(&self.timestamp_ms.to_le_bytes());
        p.push(sender_len as u8);
        p.extend_from_slice(&sender[..sender_len]);
        p
    }

    /// Encode to a full TLV frame under [`OP_EMERGENCY_CLEAR`].
    pub fn encode(&self) -> Vec<u8> {
        protocol::encode_tlv(OP_EMERGENCY_CLEAR, &self.encode_payload())
    }

    /// Decode from a TLV payload. Defensive length/UTF-8 validation as above.
    pub fn decode(payload: &[u8]) -> Result<EmergencyClear, String> {
        let mut r = Reader::new(payload);
        let version = r.u8("version")?;
        if version != CLEAR_VERSION {
            return Err(format!(
                "EmergencyClear: unsupported version {version} (expected {CLEAR_VERSION})"
            ));
        }
        let timestamp_ms = r.u64_le("timestamp")?;
        let sender_len = r.u8("sender_id_len")? as usize;
        let sender_bytes = r.take(sender_len, "sender_id")?;
        let sender_id = String::from_utf8(sender_bytes.to_vec())
            .map_err(|_| "EmergencyClear: sender_id is not valid UTF-8".to_string())?;
        Ok(EmergencyClear {
            sender_id,
            timestamp_ms,
        })
    }
}

// ── EmergencyState ──────────────────────────────────────────────────────────

/// Default beacon re-broadcast cadence: every 5 s. Frequent enough that a
/// listener who just powered on catches the distress quickly, slow enough not
/// to swamp the channel.
pub const DEFAULT_BEACON_INTERVAL_MS: u64 = 5_000;

/// Manual-SOS controller with an auto-repeat beacon cadence.
///
/// The consumer drives it like:
///   * On SOS button press: [`activate`](Self::activate) (or
///     [`activate_with`](Self::activate_with) to attach coord/note). The first
///     beacon frame is returned immediately.
///   * On a steady timer: [`tick`](Self::tick); when the interval has elapsed
///     it returns `Some(frame)` to re-broadcast, else `None`.
///   * On "I'm OK" / cancel: [`clear`](Self::clear), which returns the
///     [`OP_EMERGENCY_CLEAR`] stand-down frame to send once.
///
/// All clock-bearing methods take `now_ms`; nothing here reads a real clock,
/// so the cadence is fully deterministic in tests.
#[derive(Debug, Clone)]
pub struct EmergencyState {
    sender_id: String,
    interval_ms: u64,
    /// `Some` while an emergency is active — holds the signal we keep
    /// re-broadcasting and the `now_ms` of the last beacon we emitted.
    active: Option<ActiveBeacon>,
}

#[derive(Debug, Clone)]
struct ActiveBeacon {
    signal: EmergencySignal,
    last_beacon_ms: u64,
}

impl EmergencyState {
    /// New, inactive controller for `sender_id` with the default cadence.
    pub fn new(sender_id: impl Into<String>) -> EmergencyState {
        EmergencyState {
            sender_id: sender_id.into(),
            interval_ms: DEFAULT_BEACON_INTERVAL_MS,
            active: None,
        }
    }

    /// New, inactive controller with a custom beacon interval. A zero interval
    /// is clamped to 1 ms so `tick` always makes forward progress.
    pub fn with_interval(sender_id: impl Into<String>, interval_ms: u64) -> EmergencyState {
        EmergencyState {
            sender_id: sender_id.into(),
            interval_ms: interval_ms.max(1),
            active: None,
        }
    }

    /// Whether an emergency is currently active (broadcasting).
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The active signal, if any (for UI display / inspection).
    pub fn active_signal(&self) -> Option<&EmergencySignal> {
        self.active.as_ref().map(|a| &a.signal)
    }

    /// Activate a plain SOS now. Returns the first beacon TLV frame to send
    /// immediately. Re-activating overwrites any in-flight emergency with a
    /// fresh SOS (e.g. user re-presses) and re-emits.
    pub fn activate(&mut self, now_ms: u64) -> Vec<u8> {
        self.activate_with(EmergencyKind::Sos, now_ms, None, None)
    }

    /// Activate with an explicit kind, coordinate, and/or note. Returns the
    /// first beacon frame. Used by the man-down path to raise a
    /// [`EmergencyKind::ManDown`] beacon (which frames under [`OP_MANDOWN`]).
    pub fn activate_with(
        &mut self,
        kind: EmergencyKind,
        now_ms: u64,
        coord: Option<FixedCoord>,
        note: Option<String>,
    ) -> Vec<u8> {
        let signal = EmergencySignal::new(self.sender_id.clone(), kind, now_ms, coord, note);
        let frame = signal.encode();
        self.active = Some(ActiveBeacon {
            signal,
            last_beacon_ms: now_ms,
        });
        frame
    }

    /// Advance the clock. Returns `Some(frame)` when the beacon interval has
    /// elapsed since the last emission (re-broadcast), else `None`. The
    /// re-broadcast carries the *original* signal but refreshes its embedded
    /// `timestamp_ms` to `now_ms` so a receiver sees the beacon is live, not
    /// stale. No-op (returns `None`) when inactive.
    pub fn tick(&mut self, now_ms: u64) -> Option<Vec<u8>> {
        let active = self.active.as_mut()?;
        if now_ms.saturating_sub(active.last_beacon_ms) < self.interval_ms {
            return None;
        }
        active.last_beacon_ms = now_ms;
        active.signal.timestamp_ms = now_ms;
        Some(active.signal.encode())
    }

    /// Clear the emergency ("I'm OK"). Returns the stand-down TLV frame
    /// ([`OP_EMERGENCY_CLEAR`]) to broadcast once, or `None` if nothing was
    /// active.
    pub fn clear(&mut self, now_ms: u64) -> Option<Vec<u8>> {
        if self.active.take().is_some() {
            Some(EmergencyClear::new(self.sender_id.clone(), now_ms).encode())
        } else {
            None
        }
    }
}

// ── ManDownDetector ─────────────────────────────────────────────────────────

/// Phase of the [`ManDownDetector`] state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManDownPhase {
    /// Conditions normal — device upright and/or moving.
    Normal,
    /// Tilt+no-motion conditions met; counting down the no-motion timeout
    /// before entering the cancellable grace window. (Internally we fold the
    /// timeout into this phase: we are "settling" toward an alarm.)
    Settling,
    /// Pre-alarm grace window: the timeout elapsed, the user is being warned
    /// (e.g. countdown beep) and can still [`cancel`](ManDownDetector::cancel)
    /// before it trips.
    Grace,
    /// Tripped — man-down confirmed. The consumer should raise a beacon. Stays
    /// here until [`cancel`](ManDownDetector::cancel) or
    /// [`reset`](ManDownDetector::reset).
    Tripped,
}

/// Tuning for [`ManDownDetector`]. All durations in milliseconds, angle in
/// degrees from upright (0° = perfectly upright, 90° = flat / horizontal).
#[derive(Debug, Clone, Copy)]
pub struct ManDownConfig {
    /// Tilt past this angle (degrees from upright) counts as "fallen". A
    /// typical man-down setting is ~60°: the device is closer to horizontal
    /// than vertical.
    pub tilt_threshold_deg: f32,
    /// Continuous no-motion duration (while tilted past threshold) required
    /// before entering the grace window. Models "lying still after a fall".
    pub no_motion_timeout_ms: u64,
    /// Length of the cancellable pre-alarm grace window. The user can abort a
    /// false positive (set the phone down, bent over) within this window.
    pub grace_ms: u64,
}

impl Default for ManDownConfig {
    fn default() -> Self {
        // Industry-typical man-down defaults: 60° tilt, 30 s motionless, 10 s
        // grace countdown to cancel a false alarm.
        ManDownConfig {
            tilt_threshold_deg: 60.0,
            no_motion_timeout_ms: 30_000,
            grace_ms: 10_000,
        }
    }
}

/// Deterministic man-down detector.
///
/// Feed it periodic samples with [`update`](Self::update); it returns the
/// current [`ManDownPhase`]. It trips man-down when the device has been tilted
/// past `tilt_threshold_deg` AND motionless for `no_motion_timeout_ms`, after
/// which a `grace_ms` window lets the user [`cancel`](Self::cancel). If the
/// grace window expires without a cancel, the phase becomes
/// [`ManDownPhase::Tripped`] and the consumer should fire an
/// [`EmergencyKind::ManDown`] beacon.
///
/// Any sample showing motion, or an upright tilt, resets the countdown back to
/// [`ManDownPhase::Normal`] (as long as we haven't already tripped). This is
/// pure logic — the consumer maps its accelerometer/gyro into the
/// `(is_moving, tilt_deg)` pair.
#[derive(Debug, Clone)]
pub struct ManDownDetector {
    cfg: ManDownConfig,
    phase: ManDownPhase,
    /// `now_ms` at which the current settling/grace countdown began. Meaning
    /// depends on `phase`: in `Settling` it's when tilt+no-motion first held;
    /// in `Grace` it's when the grace window opened.
    phase_since_ms: u64,
    /// Whether the detector is armed. When disarmed it parks in `Normal` and
    /// ignores samples — lets the consumer suspend detection (e.g. on charger).
    armed: bool,
}

impl ManDownDetector {
    /// New armed detector with the given config, starting in `Normal`.
    pub fn new(cfg: ManDownConfig) -> ManDownDetector {
        ManDownDetector {
            cfg,
            phase: ManDownPhase::Normal,
            phase_since_ms: 0,
            armed: true,
        }
    }

    /// New armed detector with [`ManDownConfig::default`] tuning.
    pub fn with_defaults() -> ManDownDetector {
        Self::new(ManDownConfig::default())
    }

    /// Current phase.
    #[inline]
    pub fn phase(&self) -> ManDownPhase {
        self.phase
    }

    /// True once man-down has tripped (until cancel/reset).
    #[inline]
    pub fn is_tripped(&self) -> bool {
        self.phase == ManDownPhase::Tripped
    }

    /// Arm or disarm. Disarming forces the phase back to `Normal` and ignores
    /// subsequent samples until re-armed.
    pub fn set_armed(&mut self, armed: bool) {
        self.armed = armed;
        if !armed {
            self.phase = ManDownPhase::Normal;
            self.phase_since_ms = 0;
        }
    }

    /// Feed one sample at `now_ms`. `is_moving` is the consumer's motion
    /// decision (e.g. accelerometer magnitude above a noise floor); `tilt_deg`
    /// is the device's tilt from upright in degrees. Returns the resulting
    /// phase.
    ///
    /// State transitions:
    ///   * Moving OR upright → countdown resets to `Normal` (unless already
    ///     `Tripped`; a trip latches until explicitly cleared).
    ///   * Tilted past threshold AND still → `Settling`; after
    ///     `no_motion_timeout_ms` continuous → `Grace`; after `grace_ms` more
    ///     → `Tripped`.
    pub fn update(&mut self, now_ms: u64, is_moving: bool, tilt_deg: f32) -> ManDownPhase {
        if !self.armed {
            return self.phase;
        }

        // A latched trip stays tripped regardless of later samples — only an
        // explicit cancel/reset clears it (matches hardware man-down: once it
        // fires, picking the radio up doesn't silently un-fire the alarm).
        if self.phase == ManDownPhase::Tripped {
            return self.phase;
        }

        let danger = !is_moving && tilt_deg >= self.cfg.tilt_threshold_deg;

        if !danger {
            // Any motion or returning upright clears the countdown.
            if self.phase != ManDownPhase::Normal {
                self.phase = ManDownPhase::Normal;
                self.phase_since_ms = 0;
            }
            return self.phase;
        }

        // Danger condition holds. Advance the countdown machine.
        match self.phase {
            ManDownPhase::Normal => {
                self.phase = ManDownPhase::Settling;
                self.phase_since_ms = now_ms;
            }
            ManDownPhase::Settling => {
                if now_ms.saturating_sub(self.phase_since_ms) >= self.cfg.no_motion_timeout_ms {
                    self.phase = ManDownPhase::Grace;
                    self.phase_since_ms = now_ms;
                }
            }
            ManDownPhase::Grace => {
                if now_ms.saturating_sub(self.phase_since_ms) >= self.cfg.grace_ms {
                    self.phase = ManDownPhase::Tripped;
                }
            }
            ManDownPhase::Tripped => {} // handled above
        }
        self.phase
    }

    /// User cancelled (the grace-window abort, or dismissing a trip). Returns
    /// to `Normal`. Safe to call in any phase.
    pub fn cancel(&mut self) {
        self.phase = ManDownPhase::Normal;
        self.phase_since_ms = 0;
    }

    /// Full reset to `Normal` (alias of [`cancel`](Self::cancel) kept for
    /// caller clarity when re-arming after handling a trip).
    pub fn reset(&mut self) {
        self.cancel();
    }
}

// ── small helpers ───────────────────────────────────────────────────────────

/// Truncate `s` to at most `max` UTF-8 bytes without splitting a char.
fn truncate_utf8(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Tiny cursor over a byte slice with bounds-checked reads, returning `Err`
/// (never panicking) when the buffer is shorter than the field being read.
/// Mirrors the "validate length before acting" discipline in `protocol.rs`.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    fn take(&mut self, n: usize, field: &str) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| format!("decode: length overflow reading {field}"))?;
        if end > self.buf.len() {
            return Err(format!(
                "decode: truncated reading {field} (need {n} bytes at offset {}, have {})",
                self.pos,
                self.buf.len().saturating_sub(self.pos)
            ));
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self, field: &str) -> Result<u8, String> {
        Ok(self.take(1, field)?[0])
    }

    fn u64_le(&mut self, field: &str) -> Result<u64, String> {
        let b = self.take(8, field)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn i32_le(&mut self, field: &str) -> Result<i32, String> {
        let b = self.take(4, field)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(b);
        Ok(i32::from_le_bytes(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── opcode sanity ───────────────────────────────────────────────────────

    #[test]
    fn opcodes_are_in_tlv_range_and_unique() {
        for op in [OP_EMERGENCY, OP_MANDOWN, OP_EMERGENCY_CLEAR] {
            assert!(protocol::is_tlv_opcode(op), "op {op:#x} must be >= 0x10");
        }
        // Must not collide with the in-use set.
        let in_use = [0x01u8, 0x02, 0x10, 0x14, 0x15, 0x16, 0x17, 0x19];
        for op in [OP_EMERGENCY, OP_MANDOWN, OP_EMERGENCY_CLEAR] {
            assert!(!in_use.contains(&op), "op {op:#x} collides with in-use opcode");
        }
        assert_ne!(OP_EMERGENCY, OP_MANDOWN);
        assert_ne!(OP_EMERGENCY, OP_EMERGENCY_CLEAR);
        assert_ne!(OP_MANDOWN, OP_EMERGENCY_CLEAR);
    }

    // ── encode/decode round trips ───────────────────────────────────────────

    #[test]
    fn signal_round_trip_full() {
        let sig = EmergencySignal::new(
            "alice-7",
            EmergencyKind::Sos,
            1_700_000_000_123,
            Some(FixedCoord::from_degrees(37.7749, -122.4194)),
            Some("help, leg pinned".to_string()),
        );
        let frame = sig.encode();
        let tlv = protocol::parse_tlv(&frame).expect("parse");
        assert_eq!(tlv.opcode, OP_EMERGENCY);
        let back = EmergencySignal::decode(tlv.payload).expect("decode");
        assert_eq!(back, sig);
    }

    #[test]
    fn signal_round_trip_minimal_no_coord_no_note() {
        let sig = EmergencySignal::new("bob", EmergencyKind::Sos, 42, None, None);
        let frame = sig.encode();
        let tlv = protocol::parse_tlv(&frame).unwrap();
        let back = EmergencySignal::decode(tlv.payload).unwrap();
        assert_eq!(back, sig);
        assert!(back.coord.is_none());
        assert!(back.note.is_none());
    }

    #[test]
    fn mandown_signal_frames_under_mandown_opcode() {
        let sig = EmergencySignal::new("carol", EmergencyKind::ManDown, 99, None, None);
        assert_eq!(sig.opcode(), OP_MANDOWN);
        let frame = sig.encode();
        let tlv = protocol::parse_tlv(&frame).unwrap();
        assert_eq!(tlv.opcode, OP_MANDOWN);
        let back = EmergencySignal::decode(tlv.payload).unwrap();
        assert_eq!(back.kind, EmergencyKind::ManDown);
    }

    #[test]
    fn custom_kind_round_trip() {
        let sig = EmergencySignal::new("dave", EmergencyKind::Custom(5), 1, None, None);
        let frame = sig.encode();
        let tlv = protocol::parse_tlv(&frame).unwrap();
        let back = EmergencySignal::decode(tlv.payload).unwrap();
        assert_eq!(back.kind, EmergencyKind::Custom(5));
        // Custom frames use the generic emergency opcode.
        assert_eq!(tlv.opcode, OP_EMERGENCY);
    }

    #[test]
    fn coord_fixed_point_is_within_one_grid_step() {
        let c = FixedCoord::from_degrees(51.5074, -0.1278);
        let (lat, lon) = c.to_degrees();
        assert!((lat - 51.5074).abs() < 1e-6);
        assert!((lon - -0.1278).abs() < 1e-6);
    }

    #[test]
    fn decode_rejects_truncated_payload() {
        let sig = EmergencySignal::new("alice", EmergencyKind::Sos, 7, None, None);
        let frame = sig.encode();
        let tlv = protocol::parse_tlv(&frame).unwrap();
        // Chop the payload mid-field; decode must Err, not panic.
        let truncated = &tlv.payload[..tlv.payload.len() - 2];
        assert!(EmergencySignal::decode(truncated).is_err());
    }

    #[test]
    fn decode_rejects_bad_version() {
        // version byte 0xFF, rest arbitrary.
        let payload = [0xFFu8, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(EmergencySignal::decode(&payload).is_err());
    }

    #[test]
    fn decode_rejects_lying_sender_len() {
        // version=1, kind=SOS, flags=0, ts(8)=0, sender_len=200 but no bytes.
        let mut payload = vec![SIGNAL_VERSION, 0x01, 0x00];
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.push(200); // claims 200-byte sender id
        assert!(EmergencySignal::decode(&payload).is_err());
    }

    #[test]
    fn clear_round_trip() {
        let clr = EmergencyClear::new("alice-7", 1234);
        let frame = clr.encode();
        let tlv = protocol::parse_tlv(&frame).unwrap();
        assert_eq!(tlv.opcode, OP_EMERGENCY_CLEAR);
        let back = EmergencyClear::decode(tlv.payload).unwrap();
        assert_eq!(back, clr);
    }

    #[test]
    fn note_is_truncated_to_max_len() {
        let long = "x".repeat(MAX_NOTE_LEN + 50);
        let sig = EmergencySignal::new("a", EmergencyKind::Sos, 1, None, Some(long));
        let note = sig.note.as_ref().unwrap();
        assert!(note.len() <= MAX_NOTE_LEN);
        // Still round-trips.
        let frame = sig.encode();
        let tlv = protocol::parse_tlv(&frame).unwrap();
        let back = EmergencySignal::decode(tlv.payload).unwrap();
        assert_eq!(back.note.as_ref().unwrap().len(), note.len());
    }

    // ── EmergencyState cadence ──────────────────────────────────────────────

    #[test]
    fn activate_emits_first_beacon_immediately() {
        let mut st = EmergencyState::with_interval("alice", 5_000);
        assert!(!st.is_active());
        let frame = st.activate(0);
        assert!(st.is_active());
        let tlv = protocol::parse_tlv(&frame).unwrap();
        assert_eq!(tlv.opcode, OP_EMERGENCY);
        let sig = EmergencySignal::decode(tlv.payload).unwrap();
        assert_eq!(sig.kind, EmergencyKind::Sos);
        assert_eq!(sig.sender_id, "alice");
    }

    #[test]
    fn tick_rebroadcasts_only_after_interval() {
        let mut st = EmergencyState::with_interval("alice", 5_000);
        st.activate(0);
        // Before the interval: no re-broadcast.
        assert!(st.tick(4_999).is_none());
        // At/after the interval: a frame.
        let f = st.tick(5_000).expect("rebroadcast at interval");
        let tlv = protocol::parse_tlv(&f).unwrap();
        let sig = EmergencySignal::decode(tlv.payload).unwrap();
        assert_eq!(sig.timestamp_ms, 5_000, "beacon ts refreshed to now");
        // Next interval is measured from the last beacon.
        assert!(st.tick(9_999).is_none());
        assert!(st.tick(10_000).is_some());
    }

    #[test]
    fn tick_is_noop_when_inactive() {
        let mut st = EmergencyState::with_interval("alice", 1_000);
        assert!(st.tick(10_000).is_none());
    }

    #[test]
    fn clear_emits_standdown_and_deactivates() {
        let mut st = EmergencyState::with_interval("alice", 1_000);
        st.activate(0);
        let f = st.clear(123).expect("clear frame");
        let tlv = protocol::parse_tlv(&f).unwrap();
        assert_eq!(tlv.opcode, OP_EMERGENCY_CLEAR);
        let clr = EmergencyClear::decode(tlv.payload).unwrap();
        assert_eq!(clr.sender_id, "alice");
        assert_eq!(clr.timestamp_ms, 123);
        assert!(!st.is_active());
        // No further beacons.
        assert!(st.tick(100_000).is_none());
        // Clearing again is a no-op.
        assert!(st.clear(200).is_none());
    }

    #[test]
    fn mandown_activation_uses_mandown_opcode() {
        let mut st = EmergencyState::with_interval("alice", 1_000);
        let f = st.activate_with(EmergencyKind::ManDown, 0, None, None);
        let tlv = protocol::parse_tlv(&f).unwrap();
        assert_eq!(tlv.opcode, OP_MANDOWN);
    }

    // ── ManDownDetector ─────────────────────────────────────────────────────

    fn det() -> ManDownDetector {
        // Short, round timings for legible tests.
        ManDownDetector::new(ManDownConfig {
            tilt_threshold_deg: 60.0,
            no_motion_timeout_ms: 1_000,
            grace_ms: 500,
        })
    }

    #[test]
    fn trips_after_timeout_then_grace() {
        let mut d = det();
        // Upright + moving → normal.
        assert_eq!(d.update(0, true, 10.0), ManDownPhase::Normal);
        // Fall: tilted, still → settling.
        assert_eq!(d.update(100, false, 80.0), ManDownPhase::Settling);
        // Still settling before timeout.
        assert_eq!(d.update(900, false, 80.0), ManDownPhase::Settling);
        // Timeout (>=1000ms since settle@100) → grace.
        assert_eq!(d.update(1_100, false, 80.0), ManDownPhase::Grace);
        // Still in grace before grace_ms.
        assert_eq!(d.update(1_500, false, 80.0), ManDownPhase::Grace);
        // grace_ms (>=500 since grace@1100) elapsed → tripped.
        assert_eq!(d.update(1_600, false, 80.0), ManDownPhase::Tripped);
        assert!(d.is_tripped());
    }

    #[test]
    fn cancel_during_grace_aborts_the_trip() {
        let mut d = det();
        d.update(0, false, 80.0); // settling
        d.update(1_000, false, 80.0); // grace
        assert_eq!(d.phase(), ManDownPhase::Grace);
        d.cancel();
        assert_eq!(d.phase(), ManDownPhase::Normal);
        // Subsequent quiet samples restart the countdown, not jump to tripped.
        assert_eq!(d.update(1_100, false, 80.0), ManDownPhase::Settling);
    }

    #[test]
    fn motion_resets_countdown_before_trip() {
        let mut d = det();
        d.update(0, false, 80.0); // settling
        // Movement detected → back to normal.
        assert_eq!(d.update(500, true, 80.0), ManDownPhase::Normal);
        // Re-falling starts the countdown fresh.
        assert_eq!(d.update(600, false, 80.0), ManDownPhase::Settling);
    }

    #[test]
    fn returning_upright_resets_countdown() {
        let mut d = det();
        d.update(0, false, 80.0); // settling (tilted, still)
        // Stood back up (low tilt) even while still → normal.
        assert_eq!(d.update(500, false, 10.0), ManDownPhase::Normal);
    }

    #[test]
    fn trip_latches_until_cancel() {
        let mut d = det();
        d.update(0, false, 80.0);
        d.update(1_000, false, 80.0);
        d.update(1_500, false, 80.0); // tripped
        assert!(d.is_tripped());
        // Even if the user starts moving, a trip stays latched.
        assert_eq!(d.update(2_000, true, 0.0), ManDownPhase::Tripped);
        d.reset();
        assert_eq!(d.phase(), ManDownPhase::Normal);
    }

    #[test]
    fn disarmed_detector_ignores_samples() {
        let mut d = det();
        d.set_armed(false);
        assert_eq!(d.update(0, false, 89.0), ManDownPhase::Normal);
        assert_eq!(d.update(100_000, false, 89.0), ManDownPhase::Normal);
        // Re-arm and it works again.
        d.set_armed(true);
        assert_eq!(d.update(0, false, 89.0), ManDownPhase::Settling);
    }
}
