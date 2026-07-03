# Deferred hardening — items NOT shipped in v2.6.7

Both items below were identified during the codebase QC (v2.6.7 batch fixes).
They're real but not shipped in this round because each requires more design
work than a mechanical fix can give. Recording them here so they don't get
lost.

---

## 1. AES-GCM nonce prefix width (`crypto.rs`)

**Current:** `nonce_prefix: [u8; 4]` (32 bits) + 8-byte counter.

**Issue:** Across ~65 000 session creations from the same PSK, birthday-bound
collision of the random prefix becomes possible. With prefix collision and
overlapping counter ranges, AES-GCM nonce reuse is feasible — catastrophic
for confidentiality and authenticity.

**Proposed fix:** widen to `nonce_prefix: [u8; 8]` (64 bits), move counter
to remaining 4 bytes (32-bit, ~24 days of audio at 50 fps per session before
mandatory rekey — well within the 60-second session-key rotation cadence the
app already does).

**Why deferred:** changes the wire format of every encrypted frame. Existing
peers will fail to decrypt newer peers' frames and vice versa. Needs:
- Protocol version bump
- Per-session negotiation of nonce layout (or a hard cutover with a release
  that both peers must run)
- Audit of every nonce-handling site to confirm 64-bit prefix + 32-bit counter
  doesn't break GCM's 96-bit invariant
- Test vectors verifying decryption interop across the cutover

**Mitigation in the meantime:** SassyTalkie already rotates session keys
every 60 s (per `session.rs`), so the practical lifetime of any single nonce
prefix is short. Birthday collision risk applies to the prefix space across
session resets, not within a session. As long as a single device doesn't
create 65 k+ sessions against the same PSK without rotating the PSK, the
risk is theoretical.

---

## 2. Lock-order audit (`audio.rs` / `audio_cache.rs` / `transport.rs`)

**Current:** Three primary mutexes — `audio_cache`, `transport`, `audio` —
are acquired by TX, RX, control-plane, and replay code paths in different
orders depending on the entry point.

**Issue:** Reviewer flagged a possible deadlock triangle:
- TX path holds `audio` → tries to acquire `transport`
- RX path holds `transport` → tries to acquire `audio_cache`
- A control-plane path could hold `audio_cache` → try to acquire `audio`

If all three happen concurrently, classic ABC-deadlock. No reported live
deadlock so far, but the absence is not evidence of safety.

**Proposed fix:** establish and document a single global lock order
(suggested: `cache < transport < audio`, i.e. always acquire `cache` first,
`audio` last) and audit every acquisition site to confirm it follows the
order, refactoring any that don't.

**Why deferred:** this is a holistic audit, not a mechanical fix. Needs:
- A pass through every `.lock()` call in `audio_pipeline.rs`, `jni_bridge.rs`,
  `state.rs`, `transport.rs`, `cellular_transport.rs`, `wifi_transport.rs`
- For each site, document the *set* of locks held when it runs
- Reorder any inverted acquisitions, possibly splitting locks if the inversion
  is structural (e.g. RX genuinely needs to react to audio + transport state
  in tandem)
- Add a `#[cfg(debug_assertions)]` lock-order assertion (e.g. a thread-local
  stack of held-mutex names) to catch regressions in test runs

**Mitigation in the meantime:** the single-owner audio gate added in v2.6.6
narrows the RX↔replay surface (replay holds NO global locks for its loop —
only the small `playback_lock: Mutex<()>` self-exclusion). The triangle
risk is reduced but not eliminated.

---

Pick these up in v2.7.x when there's bandwidth for a protocol-version bump
(item 1) and a structured concurrency audit (item 2). Neither blocks daily
operation today.

---

## Status update (2026-07-03)

**Item 2 (lock-order audit):** The `docs/audit-2026-07-03.md` pass confirmed
there is **no live deadlock triangle** — the RX path acquires
`transport → audio_cache → user_registry → audio` sequentially and replay
snapshots the cache before taking `audio` last. The previously-unwritten
convention is now:

1. Documented as the canonical order in `android-native/src/lock_order.rs`
   (`Transport < AudioCache < UserRegistry < Audio`).
2. Backed by a `#[cfg(debug_assertions)]` `LockScope` guard (thread-local
   held-stack) that trips an assertion on any out-of-order acquisition, so a
   future edit can't silently reintroduce the risk. Adopt it by wrapping nested
   acquisitions; it compiles to nothing in release builds.

Incremental instrumentation of the ~24 individual `.lock()` sites in
`audio_pipeline.rs` remains optional follow-up — deliberately not done in bulk
to avoid perturbing the realtime audio path.

**Item 1 (nonce-prefix widening):** still deferred — it is a hard wire-format
break. See the note in `docs/audit-2026-07-03.md` (P2-1): SassyTalkie's project
policy forbids breaking interop with deployed v2.7.5 clients without a
coordinated, version-negotiated cutover. Recommended path when scheduled: add a
`v` (nonce-layout) field to the session handshake, negotiate 8+4 vs the current
4+8 per session, and ship a release that understands both before any device
emits the new layout.
