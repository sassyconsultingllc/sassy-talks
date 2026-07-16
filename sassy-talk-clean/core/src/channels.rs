// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-FCKOWFPRMXZC
//! channels — Channel scan + priority-channel preemption (pure logic).
//!
//! **INTEGRATION STATUS (staged, not yet wired):** the `ScanController` state
//! machine compiles and self-tests, but no consumer crate feeds it channel
//! activity or reads `current_monitor` yet, so scan/priority-preempt is inert.
//! Activation is consumer-side only (no protocol change): drive it from
//! `android-native` / `tauri-desktop` with channel-activity events + `now_ms`
//! and park the receiver on `current_monitor`. Treat as staged until then.
//!
//! A deterministic state machine modelled on real walkie-talkie scan
//! behaviour (Motorola-style priority-channel scan). It has NO platform deps:
//! the consumer crates (`android-native`, `tauri-desktop`) drive it by feeding
//! channel-activity events plus a monotonically-increasing `now_ms`, then read
//! back [`ScanController::current_monitor`] — "which channel should I have my
//! receiver parked on right now".
//!
//! Why a pure state machine and not threads/timers: scan timing has to be
//! identical on every platform and trivially unit-testable. The caller owns
//! the clock — it injects `now_ms` from whatever monotonic source it has
//! (Android `SystemClock.elapsedRealtime`, desktop `Instant`) — so the same
//! event sequence always yields the same monitor decision regardless of host.
//!
//! ── Behaviour modelled ─────────────────────────────────────────────────────
//!   * Scan list: a set of channels each tagged with a [`Priority`]
//!     (`Normal`, `P2`, `P1`). `P1` is the highest (think Motorola's primary
//!     priority channel); `P2` is secondary; `Normal` is everything else.
//!   * Dwell + hang: when activity lands on a channel the receiver dwells
//!     there. After activity stops, a "hang time" keeps it parked there briefly
//!     so a quick back-and-forth reply isn't missed before scanning resumes.
//!   * Priority preemption: while parked on a lower-priority channel, fresh
//!     activity on a higher-priority channel preempts and moves the monitor.
//!     A designated priority channel is *sampled* (look-in) even while parked
//!     elsewhere, so a P1 call is caught without waiting for the scan cursor to
//!     come around to it.
//!
//! ── Precedence rules (documented + enforced in `recompute_monitor`) ─────────
//!   1. An active EMERGENCY/manual lock (caller pins a channel) wins outright.
//!      (Not modelled here directly — the consumer simply stops calling `tick`
//!      and parks; left out to keep this module about scan logic.)
//!   2. Among channels currently considered "active" (activity within their
//!      hang window), the one with the highest [`Priority`] wins. Ties between
//!      equal priority are broken by *most-recent activity* — the channel that
//!      most recently saw traffic — so a live P1 conversation isn't abandoned
//!      for an older P1 beacon.
//!   3. If nothing is active, the monitor follows the scan cursor across the
//!      enabled scan list (round-robin by `dwell_ms` step) when scanning, or
//!      rests on the configured home/priority channel when stopped.

use std::collections::HashMap;

/// Priority tier of a scan-list channel.
///
/// Ordered so `P1 > P2 > Normal` under the derived `Ord`, which the
/// preemption logic relies on (`max_by_key` picks the highest tier). Keep the
/// declaration order — reordering silently inverts preemption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Ordinary scan-list channel. No look-in, no preemption power.
    Normal = 0,
    /// Secondary priority channel. Preempts `Normal`; yields to `P1`.
    P2 = 1,
    /// Primary priority channel. Preempts everything; sampled even while
    /// parked on another channel so its traffic is never missed.
    P1 = 2,
}

impl Priority {
    /// True for the tiers that get a "look-in" sample while parked elsewhere.
    /// Only real priority channels (P1/P2) are sampled; Normal channels are
    /// only reached when the scan cursor lands on them.
    #[inline]
    fn is_priority(self) -> bool {
        matches!(self, Priority::P1 | Priority::P2)
    }
}

/// One channel in the scan list and its mutable runtime state.
#[derive(Debug, Clone)]
struct ChannelEntry {
    priority: Priority,
    /// Whether this channel participates in scanning. A channel can be present
    /// in the list but temporarily excluded (e.g. nuisance-deleted) without
    /// losing its priority tag.
    enabled: bool,
    /// `now_ms` of the most recent activity event seen on this channel, or
    /// `None` if no activity has ever been observed.
    last_activity_ms: Option<u64>,
    /// Most recent signal level/RSSI reported with an activity event, if any.
    /// Purely informational for the consumer (squelch/UI); not used in the
    /// monitor decision so that scan behaviour stays level-agnostic and
    /// deterministic.
    last_level: Option<i16>,
}

impl ChannelEntry {
    fn new(priority: Priority) -> Self {
        ChannelEntry {
            priority,
            enabled: true,
            last_activity_ms: None,
            last_level: None,
        }
    }
}

/// Timing knobs for the scan state machine. All durations in milliseconds.
#[derive(Debug, Clone, Copy)]
pub struct ScanConfig {
    /// How long the scan cursor lingers on each channel while sweeping with no
    /// activity. Lower = faster sweep but more chance of clipping the start of
    /// a transmission; higher = slower sweep, less clipping.
    pub dwell_ms: u64,
    /// "Hang time": how long the monitor stays parked on a channel after its
    /// last activity before that channel is no longer considered active and
    /// scanning resumes. Models the brief window that lets a reply come back.
    pub hang_ms: u64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        // Defaults chosen to feel like a hardware radio: a 250 ms-per-channel
        // sweep (≈4 channels/sec) and a 2.5 s hang after a transmission, long
        // enough to catch a quick "go ahead" reply without parking forever.
        ScanConfig {
            dwell_ms: 250,
            hang_ms: 2_500,
        }
    }
}

/// Deterministic channel-scan + priority-preemption controller.
///
/// Drive it by:
///   1. [`add_channel`](Self::add_channel) for each channel + its priority.
///   2. [`set_home`](Self::set_home) to pick the channel rested on when idle.
///   3. [`start_scan`](Self::start_scan) to begin sweeping.
///   4. On every RX activity detection: [`on_activity`](Self::on_activity)
///      (optionally [`on_activity_level`](Self::on_activity_level) to attach
///      an RSSI). On every timer cadence: [`tick`](Self::tick).
///   5. Read [`current_monitor`](Self::current_monitor) to know which channel
///      to keep the receiver on.
#[derive(Debug, Clone)]
pub struct ScanController {
    channels: HashMap<u8, ChannelEntry>,
    /// Stable scan order. The cursor walks this; insertion order is preserved
    /// so the sweep is deterministic (HashMap iteration order is not).
    order: Vec<u8>,
    cfg: ScanConfig,
    scanning: bool,
    /// Channel rested on when not scanning / nothing active. Defaults to the
    /// first channel added if never set explicitly.
    home: Option<u8>,
    /// The channel the receiver is currently parked on (the answer
    /// `current_monitor` returns).
    monitor: u8,
    /// Index into `order` of the scan cursor while sweeping.
    cursor: usize,
    /// `now_ms` at which the cursor last stepped — used to time `dwell_ms`.
    last_cursor_step_ms: u64,
    /// Last `now_ms` fed in via any entry point. Used so `current_monitor`
    /// can be called without a fresh timestamp.
    now_ms: u64,
}

impl ScanController {
    /// Create an empty controller with the given timing config. No channels,
    /// not scanning, monitor parked on channel 0 until something is added.
    pub fn new(cfg: ScanConfig) -> Self {
        ScanController {
            channels: HashMap::new(),
            order: Vec::new(),
            cfg,
            scanning: false,
            home: None,
            monitor: 0,
            cursor: 0,
            last_cursor_step_ms: 0,
            now_ms: 0,
        }
    }

    /// Create a controller with default ([`ScanConfig::default`]) timing.
    pub fn with_defaults() -> Self {
        Self::new(ScanConfig::default())
    }

    /// Add (or re-tag) a channel with a priority. Re-adding an existing channel
    /// updates its priority and re-enables it but preserves activity history.
    /// The first channel added becomes the default home if none is set.
    pub fn add_channel(&mut self, channel: u8, priority: Priority) {
        match self.channels.get_mut(&channel) {
            Some(e) => {
                e.priority = priority;
                e.enabled = true;
            }
            None => {
                self.channels.insert(channel, ChannelEntry::new(priority));
                self.order.push(channel);
                if self.home.is_none() {
                    self.home = Some(channel);
                    self.monitor = channel;
                }
            }
        }
    }

    /// Remove a channel from the scan list entirely. If it was the monitor or
    /// home, the controller falls back to the home (or first remaining)
    /// channel. Removing an unknown channel is a no-op.
    pub fn remove_channel(&mut self, channel: u8) {
        if self.channels.remove(&channel).is_none() {
            return;
        }
        self.order.retain(|&c| c != channel);
        if self.home == Some(channel) {
            self.home = self.order.first().copied();
        }
        if self.cursor >= self.order.len() {
            self.cursor = 0;
        }
        if self.monitor == channel {
            self.monitor = self.home.or_else(|| self.order.first().copied()).unwrap_or(0);
        }
    }

    /// Temporarily include/exclude a channel from scanning without forgetting
    /// its priority or history (nuisance-delete / re-add). Unknown channel is
    /// a no-op.
    pub fn set_enabled(&mut self, channel: u8, enabled: bool) {
        if let Some(e) = self.channels.get_mut(&channel) {
            e.enabled = enabled;
        }
    }

    /// Designate the channel rested on when idle (not scanning, nothing
    /// active). Must be a known channel, else `Err`.
    pub fn set_home(&mut self, channel: u8) -> Result<(), String> {
        if !self.channels.contains_key(&channel) {
            return Err(format!("set_home: unknown channel {channel}"));
        }
        self.home = Some(channel);
        Ok(())
    }

    /// Begin scanning. The cursor resets to the start of the scan list and
    /// the dwell timer is primed off `now_ms`.
    pub fn start_scan(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.scanning = true;
        self.cursor = 0;
        self.last_cursor_step_ms = now_ms;
        self.recompute_monitor();
    }

    /// Stop scanning. The monitor settles back onto the home channel (unless a
    /// channel is currently active within its hang window, which still wins).
    pub fn stop_scan(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.scanning = false;
        self.recompute_monitor();
    }

    /// True while sweeping.
    #[inline]
    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    /// Report channel activity (squelch break / received frame) at `now_ms`.
    /// This stamps the channel's hang window and may immediately preempt the
    /// monitor per the precedence rules. Activity on an unknown or disabled
    /// channel is ignored (the consumer shouldn't be RX-ing one, but we guard
    /// defensively rather than panic).
    pub fn on_activity(&mut self, channel: u8, now_ms: u64) {
        self.record_activity(channel, now_ms, None);
    }

    /// Like [`on_activity`](Self::on_activity) but also records a signal
    /// level / RSSI for the consumer's UI/squelch. The level does NOT affect
    /// the monitor decision (kept deterministic + level-agnostic).
    pub fn on_activity_level(&mut self, channel: u8, now_ms: u64, level: i16) {
        self.record_activity(channel, now_ms, Some(level));
    }

    fn record_activity(&mut self, channel: u8, now_ms: u64, level: Option<i16>) {
        self.now_ms = now_ms;
        let known_enabled = match self.channels.get_mut(&channel) {
            Some(e) if e.enabled => {
                e.last_activity_ms = Some(now_ms);
                if level.is_some() {
                    e.last_level = level;
                }
                true
            }
            _ => false,
        };
        if known_enabled {
            self.recompute_monitor();
        }
    }

    /// Advance the clock. Steps the scan cursor when `dwell_ms` has elapsed and
    /// re-evaluates preemption (so a hang window that just expired releases the
    /// monitor back to scanning). Call this on a steady cadence (e.g. every
    /// 50–100 ms) — the math is edge-driven, so the exact cadence only affects
    /// timing granularity, not correctness.
    pub fn tick(&mut self, now_ms: u64) {
        // Guard against a non-monotonic clock: never let time go backwards.
        if now_ms > self.now_ms {
            self.now_ms = now_ms;
        }
        if self.scanning && self.now_ms.saturating_sub(self.last_cursor_step_ms) >= self.cfg.dwell_ms {
            self.step_cursor();
            self.last_cursor_step_ms = self.now_ms;
        }
        self.recompute_monitor();
    }

    /// The channel the receiver should be parked on right now.
    #[inline]
    pub fn current_monitor(&self) -> u8 {
        self.monitor
    }

    /// Last recorded signal level for a channel, if any was reported.
    pub fn channel_level(&self, channel: u8) -> Option<i16> {
        self.channels.get(&channel).and_then(|e| e.last_level)
    }

    /// True if the channel is "active" — saw activity within its hang window
    /// relative to the current clock. Public so the consumer can light a
    /// per-channel busy indicator.
    pub fn is_active(&self, channel: u8) -> bool {
        self.channels
            .get(&channel)
            .map(|e| self.entry_active(e))
            .unwrap_or(false)
    }

    // ── internals ──────────────────────────────────────────────────────────

    /// Whether an entry is within its hang window as of `self.now_ms`.
    fn entry_active(&self, e: &ChannelEntry) -> bool {
        match e.last_activity_ms {
            Some(ts) => self.now_ms.saturating_sub(ts) < self.cfg.hang_ms,
            None => false,
        }
    }

    /// Move the scan cursor to the next *enabled* channel, wrapping. If no
    /// channel is enabled the cursor is left where it is.
    fn step_cursor(&mut self) {
        let n = self.order.len();
        if n == 0 {
            return;
        }
        for _ in 0..n {
            self.cursor = (self.cursor + 1) % n;
            let ch = self.order[self.cursor];
            if self.channels.get(&ch).map(|e| e.enabled).unwrap_or(false) {
                return;
            }
        }
        // Nothing enabled — leave cursor as-is.
    }

    /// Core decision: set `self.monitor` per the documented precedence rules.
    ///
    /// Precedence (see module docs):
    ///   2. Highest-priority *active* channel wins; equal priority broken by
    ///      most-recent activity. Priority channels (P1/P2) are sampled here
    ///      even while the cursor is parked elsewhere ("look-in").
    ///   3. Otherwise the scan cursor's enabled channel (while scanning), or
    ///      the home channel (while stopped).
    fn recompute_monitor(&mut self) {
        // Rule 2: among active channels pick the winner. We fold over the
        // entries selecting by (priority, last_activity_ms) lexicographically.
        // Both real priority channels and the currently-parked/scanned channel
        // can be "active"; the priority comparison gives P1/P2 their look-in
        // preemption automatically.
        let mut best: Option<(u8, Priority, u64)> = None;
        for (&ch, e) in self.channels.iter() {
            if !e.enabled || !self.entry_active(e) {
                continue;
            }
            let ts = e.last_activity_ms.unwrap_or(0);
            let cand = (ch, e.priority, ts);
            best = match best {
                None => Some(cand),
                Some(cur) => {
                    // Higher priority wins; tie → more recent activity wins;
                    // still tied → lower channel number for total determinism.
                    let better = (cand.1, cand.2, std::cmp::Reverse(cand.0))
                        > (cur.1, cur.2, std::cmp::Reverse(cur.0));
                    if better { Some(cand) } else { Some(cur) }
                }
            };
        }

        if let Some((ch, _, _)) = best {
            self.monitor = ch;
            return;
        }

        // Rule 3: nothing active.
        if self.scanning {
            // Park on the cursor's enabled channel. If the cursor somehow
            // points at a disabled/missing channel, advance to a valid one.
            if let Some(&ch) = self.order.get(self.cursor) {
                if self.channels.get(&ch).map(|e| e.enabled).unwrap_or(false) {
                    self.monitor = ch;
                    return;
                }
            }
            self.step_cursor();
            if let Some(&ch) = self.order.get(self.cursor) {
                self.monitor = ch;
                return;
            }
        }

        // Not scanning (or no enabled channels): rest on home.
        if let Some(h) = self.home {
            self.monitor = h;
        } else if let Some(&first) = self.order.first() {
            self.monitor = first;
        }
        // else: keep whatever monitor we had (likely 0 on an empty list).
    }

    /// True if `channel` is a designated priority channel (P1 or P2) — i.e.
    /// one that gets a look-in sample while parked elsewhere. Exposed mostly
    /// for the consumer's UI ("PRI" tag) and to document the look-in set.
    pub fn is_priority_channel(&self, channel: u8) -> bool {
        self.channels
            .get(&channel)
            .map(|e| e.priority.is_priority())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> ScanController {
        // Tight, round numbers so the timing math in tests is obvious.
        ScanController::new(ScanConfig { dwell_ms: 100, hang_ms: 1_000 })
    }

    #[test]
    fn first_added_channel_becomes_home_and_monitor() {
        let mut c = ctrl();
        c.add_channel(7, Priority::Normal);
        assert_eq!(c.current_monitor(), 7);
        c.add_channel(8, Priority::Normal);
        // Home stays on the first one added.
        assert_eq!(c.current_monitor(), 7);
    }

    #[test]
    fn scan_cursor_steps_after_dwell() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::Normal);
        c.add_channel(3, Priority::Normal);
        c.start_scan(0);
        assert_eq!(c.current_monitor(), 1);
        // Not enough time elapsed → still on 1.
        c.tick(50);
        assert_eq!(c.current_monitor(), 1);
        // Dwell elapsed → step to 2, then 3, then wrap to 1.
        c.tick(100);
        assert_eq!(c.current_monitor(), 2);
        c.tick(200);
        assert_eq!(c.current_monitor(), 3);
        c.tick(300);
        assert_eq!(c.current_monitor(), 1);
    }

    #[test]
    fn activity_dwells_then_hang_expires_and_resumes_scan() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::Normal);
        c.start_scan(0);
        // Activity on ch2 while parked on ch1 → preempt to ch2 (equal prio,
        // but ch2 is the only active channel).
        c.on_activity(2, 10);
        assert_eq!(c.current_monitor(), 2);
        // Within hang window → stays on 2 even as cursor would step.
        c.tick(500);
        assert_eq!(c.current_monitor(), 2);
        // Hang window (1000ms from last activity @10) expires → resume scan.
        c.tick(1_011);
        assert_ne!(c.current_monitor(), 2, "should have released ch2 after hang");
    }

    #[test]
    fn p1_preempts_normal_via_lookin() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(9, Priority::P1);
        c.start_scan(0);
        // Normal traffic on ch1 first.
        c.on_activity(1, 10);
        assert_eq!(c.current_monitor(), 1);
        // P1 traffic arrives on ch9 while parked on ch1 → look-in preempts.
        c.on_activity(9, 20);
        assert_eq!(c.current_monitor(), 9, "P1 must preempt Normal");
    }

    #[test]
    fn p1_beats_p2_beats_normal() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::P2);
        c.add_channel(3, Priority::P1);
        c.start_scan(0);
        // All three active simultaneously within hang.
        c.on_activity(1, 10);
        c.on_activity(2, 11);
        c.on_activity(3, 12);
        assert_eq!(c.current_monitor(), 3, "P1 wins over P2 and Normal");
        // P1 goes quiet (hang expires) but P2 still active → P2 wins.
        // Refresh P2 activity to keep it inside its hang window.
        c.on_activity(2, 1_005);
        c.tick(1_020); // ch3 last activity @12 → expired; ch2 @1005 → active.
        assert_eq!(c.current_monitor(), 2, "P2 wins once P1 quiet");
    }

    #[test]
    fn equal_priority_breaks_tie_by_most_recent_activity() {
        let mut c = ctrl();
        c.add_channel(1, Priority::P1);
        c.add_channel(2, Priority::P1);
        c.start_scan(0);
        c.on_activity(1, 10);
        c.on_activity(2, 20); // more recent
        assert_eq!(c.current_monitor(), 2);
        // Newer activity on ch1 flips it back.
        c.on_activity(1, 30);
        assert_eq!(c.current_monitor(), 1);
    }

    #[test]
    fn stop_scan_rests_on_home() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::Normal);
        c.add_channel(5, Priority::Normal);
        c.set_home(5).unwrap();
        c.start_scan(0);
        c.tick(100); // cursor moves off home
        assert_eq!(c.current_monitor(), 2);
        c.stop_scan(150);
        assert_eq!(c.current_monitor(), 5, "rest on home when stopped");
    }

    #[test]
    fn disabled_channel_is_skipped_by_cursor() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::Normal);
        c.add_channel(3, Priority::Normal);
        c.set_enabled(2, false);
        c.start_scan(0);
        assert_eq!(c.current_monitor(), 1);
        c.tick(100);
        assert_eq!(c.current_monitor(), 3, "skip disabled ch2");
    }

    #[test]
    fn disabled_channel_ignores_activity() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::P1);
        c.set_enabled(2, false);
        c.start_scan(0);
        c.on_activity(2, 10); // disabled P1 must not preempt
        assert_eq!(c.current_monitor(), 1);
    }

    #[test]
    fn remove_monitor_channel_falls_back_to_home() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal); // becomes home
        c.add_channel(2, Priority::Normal);
        c.start_scan(0);
        c.on_activity(2, 10);
        assert_eq!(c.current_monitor(), 2);
        c.remove_channel(2);
        assert_eq!(c.current_monitor(), 1, "fall back to home after removal");
    }

    #[test]
    fn unknown_channel_activity_is_ignored() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.start_scan(0);
        c.on_activity(99, 10); // not in list
        assert_eq!(c.current_monitor(), 1);
    }

    #[test]
    fn level_is_recorded_but_not_in_decision() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::Normal);
        c.start_scan(0);
        c.on_activity_level(2, 10, -40);
        assert_eq!(c.channel_level(2), Some(-40));
        assert_eq!(c.current_monitor(), 2);
    }

    #[test]
    fn priority_channel_classification() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::P2);
        c.add_channel(3, Priority::P1);
        assert!(!c.is_priority_channel(1));
        assert!(c.is_priority_channel(2));
        assert!(c.is_priority_channel(3));
        assert!(!c.is_priority_channel(99));
    }

    #[test]
    fn non_monotonic_clock_does_not_regress() {
        let mut c = ctrl();
        c.add_channel(1, Priority::Normal);
        c.add_channel(2, Priority::Normal);
        c.start_scan(0);
        c.tick(100);
        assert_eq!(c.current_monitor(), 2);
        // A backwards tick must not rewind the dwell timer / cursor.
        c.tick(50);
        assert_eq!(c.current_monitor(), 2);
    }
}
