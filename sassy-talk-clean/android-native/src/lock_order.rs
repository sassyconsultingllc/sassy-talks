//! lock_order — canonical mutex acquisition order + a debug-only regression guard.
//!
//! Resolves `docs/deferred-hardening.md` item 2 (lock-order audit). The audit in
//! `docs/audit-2026-07-03.md` found **no** live deadlock triangle, but the
//! acquisition order was only an unwritten convention. This module writes it
//! down and provides a `#[cfg(debug_assertions)]` guard so a future edit that
//! acquires locks out of order trips a loud assertion in debug/test runs instead
//! of silently reintroducing an ABC-deadlock risk.
//!
//! ## Canonical order
//!
//! The five StateMachine-owned mutexes MUST be acquired in this ascending order
//! whenever more than one is held at once (a lower level may never be acquired
//! while a higher one is held):
//!
//! | Level | Lock            | Rationale                                        |
//! |-------|-----------------|-------------------------------------------------|
//! | 0     | `Transport`     | RX reads the wire first, before touching audio. |
//! | 1     | `AudioCache`    | Frames are ingested into the cache next.         |
//! | 2     | `UserRegistry`  | Mute/favorite lookups happen during ingest.      |
//! | 3     | `Audio`         | The engine is the leaf — acquired last, held     |
//! |       |                 | briefly for playback/recording.                  |
//!
//! This matches the existing RX path (`audio_pipeline.rs`):
//! `transport → audio_cache → user_registry → audio`, and the replay path which
//! snapshots the cache, drops `JNI_STATE`, then takes `audio` last.
//!
//! `JNI_STATE` is a control-plane coordinator, not part of this ordering: JNI
//! entry points take it first and then at most one StateMachine lock; the RX/TX
//! threads never take `JNI_STATE`, so it can't form a cycle with these four.
//!
//! ## Adoption
//!
//! Wrap a nested acquisition in a [`LockScope`]:
//! ```ignore
//! let _t = LockScope::enter(LockLevel::Transport);
//! let tm = transport.lock().unwrap();
//! let _a = LockScope::enter(LockLevel::Audio);
//! let eng = audio.lock().unwrap();
//! ```
//! In release builds `LockScope` compiles to nothing. In debug/test builds it
//! pushes onto a thread-local stack and asserts the level is strictly greater
//! than the current top, catching any inversion at the point it happens.

/// Ordered lock levels. Lower must be acquired before higher when nesting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LockLevel {
    Transport = 0,
    AudioCache = 1,
    UserRegistry = 2,
    Audio = 3,
}

#[cfg(debug_assertions)]
thread_local! {
    static HELD: std::cell::RefCell<Vec<LockLevel>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// RAII guard that records a held lock level for the duration of a scope and
/// asserts (debug only) that lock order is respected. No-op in release builds.
#[must_use]
pub struct LockScope {
    #[cfg(debug_assertions)]
    level: LockLevel,
}

impl LockScope {
    /// Record that a lock of `level` is being acquired. In debug builds this
    /// panics if a lock of an equal-or-higher level is already held on this
    /// thread (an out-of-order acquisition).
    #[inline]
    pub fn enter(level: LockLevel) -> Self {
        #[cfg(debug_assertions)]
        {
            HELD.with(|h| {
                let mut stack = h.borrow_mut();
                if let Some(&top) = stack.last() {
                    debug_assert!(
                        level > top,
                        "lock-order violation: acquiring {:?} while holding {:?} \
                         (canonical order is Transport < AudioCache < UserRegistry < Audio)",
                        level,
                        top,
                    );
                }
                stack.push(level);
            });
            return Self { level };
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = level;
            Self {}
        }
    }
}

impl Drop for LockScope {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            HELD.with(|h| {
                let mut stack = h.borrow_mut();
                // Pop the matching level. Nested scopes drop in reverse order, so
                // the top should be ours; tolerate mismatch defensively.
                if let Some(pos) = stack.iter().rposition(|&l| l == self.level) {
                    stack.remove(pos);
                }
            });
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn ascending_order_is_allowed() {
        let _t = LockScope::enter(LockLevel::Transport);
        let _c = LockScope::enter(LockLevel::AudioCache);
        let _r = LockScope::enter(LockLevel::UserRegistry);
        let _a = LockScope::enter(LockLevel::Audio);
    }

    #[test]
    fn single_locks_are_allowed() {
        let _a = LockScope::enter(LockLevel::Audio);
        drop(_a);
        let _t = LockScope::enter(LockLevel::Transport);
    }

    #[test]
    #[should_panic(expected = "lock-order violation")]
    fn inversion_is_caught() {
        let _a = LockScope::enter(LockLevel::Audio);
        // Acquiring Transport (0) while holding Audio (3) is the classic
        // inversion that could form a deadlock — must trip the assertion.
        let _t = LockScope::enter(LockLevel::Transport);
    }

    #[test]
    fn levels_are_ordered() {
        assert!(LockLevel::Transport < LockLevel::AudioCache);
        assert!(LockLevel::AudioCache < LockLevel::UserRegistry);
        assert!(LockLevel::UserRegistry < LockLevel::Audio);
    }
}
