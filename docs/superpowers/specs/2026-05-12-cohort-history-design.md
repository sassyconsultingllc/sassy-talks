<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-SS7PIX63BHRN
-->
# Cohort History & Past-Session Re-Entry Design

**Date:** 2026-05-12
**Status:** Approved (brainstorming complete)
**Approach:** Symmetric cohort history (hosted + joined), metadata-only persistence — see decision (b) in brainstorming.

## Problem

A teacher running multiple cohort groups (e.g. "Math 101 P1", "Math 101 P2", "Math 102") has no way to return to a previous cohort after its session ends. Today the only persisted state is the per-channel AES-256-GCM key (`session_ch_N` in `EncryptedSharedPreferences`); once that key is cleared or TTL-expires, the cohort disappears from the UI entirely.

Storing the key longer to enable re-entry is unacceptable: long-lived AES keys defeat forward secrecy, and "find your old encryption key" is not a workflow we want users performing. The cohort *identity* (its name, channel, last-seen participants) is non-sensitive and can live longer than the key.

## Goals

1. After a session ends, the user can still see the cohorts they hosted **or** joined, with the names of participants the device saw.
2. Re-entering a cohort works without any AES key surviving past its TTL — every rejoin produces a fresh key.
3. The mechanism is symmetric: cohorts you joined and cohorts you hosted both appear, with role-appropriate actions.
4. Back-compat: legacy QRs (without the new `cohort_id` field) still import; their cohort_id is minted locally on the joiner's device.

## Non-Goals

- **Auto-rejoin for joiners without a fresh QR scan.** A joiner cannot resume a cohort unilaterally — the host must show a fresh QR. This is by design: otherwise a stolen joiner device could resume long-gone classes.
- **Stable per-student identity across key rotations.** User IDs are still derived from the session key (`SHA-256(key)[..8]`), so the same student appears with a different id each session. We show names from snapshots; we do not claim identity continuity. Stable peer IDs are a separate follow-up (would require adding a per-device salt to the CAPABILITIES handshake).
- **Relay-side "request rejoin" pings.** No new control opcodes; the relay protocol is untouched. If you want the host's device to get a notification when an ex-joiner wants to resume, that's v2.
- **Cross-device cohort sync.** History is local to each device's `EncryptedSharedPreferences`. No cloud sync.

## Threat Model Notes

- **What we persist:** cohort_id (UUID), channel (1..8), group_name, role, host_device (string), participant name+id snapshots, timestamps. No keys, no derivable secrets.
- **Storage:** `EncryptedSharedPreferences` (Keystore-backed AES) under `cohort_history_v1`. Encryption at rest is defense in depth; the contents would not be catastrophic if extracted.
- **What we explicitly do not persist:** AES session keys, base64 key material in any form, anything from which a key could be derived. A regression test asserts the serialized blob contains neither `"key":` nor any 32-byte base64 substring.
- **TTL behavior:** unchanged. Keys still zeroize on `Drop` when the channel is cleared / replaced / expires. History entries persist independently and have their own LRU eviction (cap 50, oldest `last_joined_at` first).

## Data Model

### QR schema (additive change in `android-native/src/session.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionKey {
    pub key: String,
    pub device: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub session_id: String,
    #[serde(default = "default_channel")]
    pub channel: u8,
    #[serde(default)]
    pub group_name: String,
    #[serde(default)]
    pub cohort_id: String,   // NEW — stable UUID across key rotations
}
```

`#[serde(default)]` keeps legacy QRs (no `cohort_id` field) importable. When importing such a QR, the joiner mints a local cohort_id and stores it on the new cohort history record — the host's device will mint its own cohort_id the first time it regenerates under the new code.

### Cohort history record (new file `android-native/src/cohort_history.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CohortRole {
    Hosted,
    Joined,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantSnapshot {
    pub id: String,    // 8-hex (from SHA-256(key)[..8] at snapshot time)
    pub name: String,  // peer device/display name at snapshot time
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CohortRecord {
    pub cohort_id: String,
    pub channel: u8,
    pub group_name: String,
    pub role: CohortRole,
    pub host_device: Option<String>,   // Some(name) for Joined/Both, None for pure Hosted
    pub last_participants: Vec<ParticipantSnapshot>,
    pub last_session_id: String,       // most recent session_id, for display only — may be expired
    pub created_at: u64,
    pub last_joined_at: u64,
}

pub struct CohortHistory {
    records: Vec<CohortRecord>,        // ordered by last_joined_at desc
    cap: usize,                        // default 50
}
```

The history is serialized as a JSON array stored in `EncryptedSharedPreferences` under key `cohort_history_v1`. The Rust side owns the data structure; Kotlin only persists the blob.

## Capture Points (when records are written)

1. **Host generates a QR** (`generate_session_qr` / `generate_channel_qr`):
   - If caller supplies a `cohort_id`, reuse it. Otherwise, look up an existing `Hosted` or `Both` record matching `(channel, group_name)` and reuse its cohort_id. If none, mint a fresh UUID.
   - Upsert: role becomes `Hosted` (or `Both` if an existing record was `Joined`). Bump `last_joined_at`. `host_device` stays `None` on pure host entries — it's only set for the joiner-side view.
2. **Joiner imports a QR** (`import_session`):
   - Use QR's `cohort_id` if it is non-empty; treat `""` and missing field identically (mint one locally).
   - Attempt orphan merge: if there is an existing local record (from a prior legacy import — minted-local cohort_id) matching `(channel, group_name, host_device == session.device)` and last joined within the past 30 days, merge into the new cohort_id and adopt the new id as canonical.
   - Upsert: role becomes `Joined` (or `Both` if it was previously `Hosted`). `host_device = Some(session.device)`. Bump `last_joined_at`.
3. **Active session, peer announces name** (existing flow → `register_user`):
   - Look up the cohort record matching the channel's active `cohort_id`. Throttle to at most one snapshot write per ~30s per cohort to avoid churn. Replace `last_participants` with the current `UserRegistry` view filtered to this channel.
4. **Session ends** (`clear_session`, `clear_channel`, TTL expiry detected in `get_crypto_for_channel`):
   - Take one final participant snapshot before the key drops. The cohort record itself is **not** deleted — only the key.

## Rust API (new in `cohort_history.rs`, plus session.rs threading)

```rust
impl CohortHistory {
    pub fn new(cap: usize) -> Self;
    pub fn load_from_json(json: &str, cap: usize) -> Self;  // tolerant of empty/invalid
    pub fn to_json(&self) -> String;

    pub fn upsert_host(&mut self, channel: u8, group_name: &str,
                       cohort_id: Option<&str>, session_id: &str) -> String; // returns cohort_id
    pub fn upsert_joiner(&mut self, channel: u8, group_name: &str,
                         cohort_id: Option<&str>, host_device: &str,
                         session_id: &str) -> String;

    pub fn snapshot_participants(&mut self, cohort_id: &str,
                                 participants: Vec<ParticipantSnapshot>);

    pub fn remove(&mut self, cohort_id: &str);
    pub fn clear(&mut self);
    pub fn find(&self, cohort_id: &str) -> Option<&CohortRecord>;
    pub fn list(&self) -> &[CohortRecord];
}
```

`SessionManager::generate_session_qr` gains an optional `cohort_id` parameter (threaded through `generate_channel_qr` JNI as a nullable Kotlin string). `SessionManager::import_session` returns the cohort_id alongside the `(channel, CryptoSession)` tuple so the bridge can update history.

## JNI Surface (`android-native/src/jni_bridge.rs`)

```rust
nativeGetCohortHistory() -> String                   // JSON array
nativeRemoveCohort(cohort_id: String)
nativeClearCohortHistory()
nativeGenerateChannelQR(channel, hours, group_name, cohort_id_or_null) -> String  // existing + cohort_id param
nativeImportSessionFromQR(qr_json) -> Boolean        // unchanged signature; side effect updates history
nativeGetActiveCohortId(channel: u8) -> String       // used by the snapshotter
```

## Kotlin Wrappers (`SassyTalkNative.kt`)

```kotlin
fun getCohortHistory(): String
fun removeCohort(cohortId: String)
fun clearCohortHistory()
fun generateChannelQR(channel: Int, durationHours: Int = 24,
                     groupName: String = "", cohortId: String? = null): String
```

Kotlin also gains a small periodic snapshotter (in `WalkieService` or `PttCoordinator`) that calls into native every ~30s while a session is active to refresh participant lists.

Storage flow: Rust returns the full history JSON; Kotlin writes the blob to `EncryptedSharedPreferences` under `cohort_history_v1`. On native init, Kotlin reads the blob and passes it to `nativeLoadCohortHistory(json)` so the in-memory state is restored.

## UI — `QRAuthScreen.kt` changes

A new "My Cohorts" tab is added alongside the existing Show QR / Scan QR / Enter Code tabs. Its content is conditional on `getCohortHistory()` being non-empty; empty state shows a short hint: *"Cohorts you host or join will show up here so you can resume them without saving keys."*

**Row layout (per `CohortRecord`):**

```
┌──────────────────────────────────────────────────────────┐
│ Math 101 P1                                     [Ch 1]   │  ← group_name + channel chip
│ Hosted by Sarah's Moto · 4 participants · 2d ago         │  ← host_device (if Joined/Both), count, relative time
│ [chip] Sarah  [chip] Devon  [chip] Mia  [chip] +1        │  ← last_participants (first 3 names + overflow)
│                                                          │
│ [ Rejoin ]                                          [⋮]  │  ← role-aware action + overflow → Remove
└──────────────────────────────────────────────────────────┘
```

**Action behavior by role:**

- **Hosted:** "Rejoin" → switch to Show-QR tab with `channel`, `groupName`, `cohortId` pre-filled and auto-run Generate. New QR appears; existing flow takes over.
- **Joined:** "Rejoin" → switch to Scan-QR tab, show a hint card: *"Hosted by {host_device}. Ask them to show their QR."* On a successful scan: if scanned cohort_id matches this record, bump `last_joined_at` and stay; if it doesn't match, the new cohort is added as a separate entry (the orphan-merge heuristic may still link them transparently).
- **Both:** two stacked actions — "Show My QR" (host path) and "Scan to Rejoin" (joiner path).

**Overflow menu:** Remove (calls `removeCohort(cohortId)`). A "Clear All" button lives at the bottom of the tab.

## Edge Cases

- **Two cohorts with same group_name + channel:** they're separate records with different cohort_ids. Upsert-on-host only reuses an existing cohort_id when both name *and* channel match; rename mid-session does not merge entries.
- **Legacy QR (no cohort_id field):** joiner mints a local cohort_id. If the host later regenerates under the new code, their QR will carry a different cohort_id; the orphan-merge heuristic (match on `channel + group_name + host_device`) catches it and merges. If the heuristic misses, two entries coexist — acceptable; user can manually Remove the stale one.
- **TTL expiry while app is backgrounded:** the next call into `get_crypto_for_channel` finds the session expired. We still capture a final snapshot from the last known `UserRegistry` view before dropping the key.
- **History overflows cap:** evict by oldest `last_joined_at`. Caller is never blocked from creating a new record.
- **`EncryptedSharedPreferences` unavailable** (Keystore init failed — same fallback path as session keys): history becomes in-memory-only for that launch and is lost on shutdown. Consistent with how session keys behave today; no cleartext fallback.

## Testing

**Rust unit tests (`cohort_history.rs`):**
- JSON roundtrip preserves all fields.
- LRU eviction at cap.
- `upsert_host` then `upsert_joiner` (or vice versa) on the same cohort_id promotes role to `Both`.
- Importing a QR without `cohort_id` mints a local one; orphan-merge heuristic merges on `(channel, group_name, host_device)` match.
- **Security invariant:** `to_json()` output contains no `"key":` field and no contiguous 32-byte base64 substring. Asserted by regex.

**Rust tests (`session.rs`):**
- `generate_session_qr(..., cohort_id=Some(x))` reuses `x` in the emitted JSON.
- `import_session` on a QR with cohort_id returns it; on a legacy QR returns a freshly minted UUID.

**Kotlin instrumented tests (`android-app`):**
- Generate as host → record appears as `Hosted` with the device's cohort_id.
- Clear session → record retained; key cleared (assert `isAuthenticated()` is false but `getCohortHistory()` still lists the cohort).
- Import legacy-format QR → record appears as `Joined` with a minted cohort_id.
- Rejoin a Joined cohort by scanning a fresh QR with matching cohort_id → `last_joined_at` updates, role stays `Joined`.
- Rejoin a Hosted cohort via "Rejoin" button → new `session_id` differs from `last_session_id`, but `cohort_id` and `channel` match.

## File Touch List

**New:**
- `android-native/src/cohort_history.rs` — data structure, JSON I/O, LRU, snapshot/upsert logic, tests.

**Modified:**
- `android-native/src/session.rs` — add `cohort_id` to `SessionKey` (with `#[serde(default)]`); thread optional cohort_id through `generate_session_qr` / `generate_channel_qr`; return cohort_id alongside `import_session`'s output.
- `android-native/src/jni_bridge.rs` — new JNI exports (`nativeGetCohortHistory`, `nativeRemoveCohort`, `nativeClearCohortHistory`, `nativeLoadCohortHistory`, `nativeGetActiveCohortId`); update `nativeGenerateChannelQR` signature to accept nullable cohort_id; wire `nativeImportSessionFromQR` to update history on success.
- `android-native/src/lib.rs` — `pub mod cohort_history;` and wire it into the global state owned alongside `SessionManager`.
- `android-app/.../SassyTalkNative.kt` — Kotlin wrappers, history blob persistence in `EncryptedSharedPreferences` (`cohort_history_v1`), load-on-init, save-on-change hook.
- `android-app/.../ui/QRAuthScreen.kt` — new `MyCohortsTab` composable, role-aware row layout, prefill plumbing to Show-QR and Scan-QR tabs, empty-state hint.
- `android-app/.../WalkieService.kt` — periodic 30s participant snapshotter that calls into native while a session is active. (`WalkieService` already owns the foreground service lifecycle and the `PttCoordinator`, so it's the natural snapshot driver.)

**Not changed:**
- Audio pipeline, Bluetooth transport, Cellular relay protocol, Cloudflare Durable Object, Tauri desktop UI (this is an Android-only feature for v1).

## Open Questions Deferred to v2

- Stable per-student peer IDs across key rotations (would change how user_ids are derived in `users.rs`).
- Relay-side rejoin notifications (new opcode + DO routing).
- Tauri desktop equivalent UI.
- Cross-device cohort sync.
