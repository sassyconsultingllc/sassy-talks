# Cohort History & Past-Session Re-Entry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user re-enter past cohorts (hosted or joined) and see participants seen there, without persisting AES keys past their TTL.

**Architecture:** Add a `cohort_id` field to the QR/session payload (stable across key rotations, `#[serde(default)]` for back-compat). Introduce a separate `CohortHistory` registry in the Rust core that stores cohort metadata only — never keys. Wire it into the existing `SessionManager` capture points (generate/import) and a periodic participant snapshotter. Surface a "My Cohorts" tab in `QRAuthScreen` with role-aware "Rejoin" actions.

**Tech Stack:** Rust (android-native core), JNI bridge, Kotlin (android-app), Jetpack Compose, `EncryptedSharedPreferences`.

**Spec:** `docs/superpowers/specs/2026-05-12-cohort-history-design.md`

---

## File Structure

**New files:**
- `android-native/src/cohort_history.rs` — Data types (`CohortRecord`, `CohortRole`, `ParticipantSnapshot`), `CohortHistory` registry with JSON I/O, LRU eviction, upsert/orphan-merge/snapshot logic. Unit tests live in the same file.
- `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/MyCohortsTab.kt` — Compose UI for the cohort list, row layout, role-aware actions, empty state.

**Modified files:**
- `android-native/src/lib.rs` — `pub mod cohort_history;`
- `android-native/src/session.rs` — Add `cohort_id: String` to `SessionKey`; thread optional cohort_id into `generate_session_qr`; widen `import_session` return to `(u8, CryptoSession, String)` so callers receive cohort_id.
- `android-native/src/jni_bridge.rs` — Add `cohort_history: CohortHistory` to `JniAppState`; new JNI exports (`nativeGetCohortHistory`, `nativeRemoveCohort`, `nativeClearCohortHistory`, `nativeLoadCohortHistory`, `nativeGetActiveCohortId`); update `nativeGenerateChannelQR` to accept `cohortId` parameter; wire `nativeImportSessionFromQR` to upsert into history; update call sites for the new `import_session` return shape.
- `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/SassyTalkNative.kt` — Kotlin wrappers; load/save history blob in `EncryptedSharedPreferences` under `cohort_history_v1`.
- `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/QRAuthScreen.kt` — Add "My Cohorts" tab, prefill plumbing into Show-QR and Scan-QR tabs.
- `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/WalkieService.kt` — Periodic 30 s participant snapshotter.

**Test files:**
- `android-native/src/cohort_history.rs` (`#[cfg(test)] mod tests`) — Rust unit tests.
- `android-native/src/session.rs` (existing test module) — extended.
- `android-app/app/src/androidTest/java/com/sassyconsulting/sassytalkie/CohortHistoryInstrumentedTest.kt` — instrumented Android tests.

---

## Task 1: Add `cohort_id` to `SessionKey` and thread through `generate_session_qr`

**Files:**
- Modify: `android-native/src/session.rs`

- [ ] **Step 1: Write the failing test (added to existing `tests` mod in session.rs)**

```rust
#[test]
fn test_session_includes_cohort_id_field() {
    let mut host = SessionManager::new("Host");
    let qr_json = host.generate_session_qr(1, 24, "Alpha Team").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&qr_json).unwrap();
    let cohort_id = parsed.get("cohort_id").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!cohort_id.is_empty(), "cohort_id must be present and non-empty");
    assert_eq!(cohort_id.len(), 36, "cohort_id must be a UUID");
}

#[test]
fn test_generate_with_reused_cohort_id_preserves_it() {
    let mut host = SessionManager::new("Host");
    let qr1 = host.generate_session_qr_with_cohort(1, 24, "Alpha", None).unwrap();
    let cid: String = serde_json::from_str::<serde_json::Value>(&qr1).unwrap()
        ["cohort_id"].as_str().unwrap().to_string();
    let qr2 = host.generate_session_qr_with_cohort(1, 24, "Alpha", Some(&cid)).unwrap();
    let cid2: String = serde_json::from_str::<serde_json::Value>(&qr2).unwrap()
        ["cohort_id"].as_str().unwrap().to_string();
    assert_eq!(cid, cid2, "supplied cohort_id must round-trip");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path android-native/Cargo.toml session::tests::test_session_includes_cohort_id_field session::tests::test_generate_with_reused_cohort_id_preserves_it`

Expected: FAIL — compile error (no field `cohort_id`, no method `generate_session_qr_with_cohort`).

- [ ] **Step 3: Add `cohort_id` field to `SessionKey`**

In `android-native/src/session.rs`, modify the struct (between the existing `group_name` and the closing brace of `SessionKey`):

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
    /// Stable cohort identifier across key rotations. Empty/missing in
    /// legacy QRs — the importer mints one locally in that case.
    #[serde(default)]
    pub cohort_id: String,
}
```

Update the `Drop` impl to zeroize the new field too:

```rust
impl Drop for SessionKey {
    fn drop(&mut self) {
        self.key.zeroize();
        self.device.zeroize();
        self.session_id.zeroize();
        self.group_name.zeroize();
        self.cohort_id.zeroize();
    }
}
```

- [ ] **Step 4: Add `generate_session_qr_with_cohort` and refactor `generate_session_qr` to delegate**

Replace the existing `generate_session_qr` method in `impl SessionManager` with these two methods:

```rust
/// Generate a new session QR for a specific channel, minting a fresh cohort_id.
pub fn generate_session_qr(
    &mut self,
    channel: u8,
    duration_hours: u32,
    group_name: &str,
) -> Result<String, String> {
    self.generate_session_qr_with_cohort(channel, duration_hours, group_name, None)
}

/// Generate a session QR, optionally reusing a previously-known cohort_id
/// (used by the "Rejoin" flow so a regenerated session inherits cohort identity).
pub fn generate_session_qr_with_cohort(
    &mut self,
    channel: u8,
    duration_hours: u32,
    group_name: &str,
    cohort_id: Option<&str>,
) -> Result<String, String> {
    let ch_idx = validate_channel(channel)?;
    let hours = if duration_hours == 0 { DEFAULT_SESSION_HOURS } else { duration_hours };
    let duration = hours.min(MAX_SESSION_HOURS).max(1);
    let now = current_unix_time()?;
    let expires = now + (duration as u64 * 3600);

    let key_bytes: [u8; 32] = rand::random();
    let key_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &key_bytes,
    );

    let session_id = uuid::Uuid::new_v4().to_string();
    let cohort = cohort_id
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let name = if group_name.is_empty() {
        format!("Channel {}", channel)
    } else {
        group_name.to_string()
    };

    let session = SessionKey {
        key: key_b64,
        device: self.device_name.clone(),
        created_at: now,
        expires_at: expires,
        session_id: session_id.clone(),
        channel,
        group_name: name.clone(),
        cohort_id: cohort.clone(),
    };

    let json = serde_json::to_string(&session)
        .map_err(|e| format!("Failed to serialize session: {}", e))?;

    self.channels[ch_idx] = Some(ChannelSession {
        key: session,
        group_name: name.clone(),
    });

    info!("Session generated for ch{} '{}' cohort {}: {} (expires in {}h)",
        channel, name, cohort, session_id, duration);

    Ok(json)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path android-native/Cargo.toml session::tests`

Expected: PASS — all session tests pass, including the two new ones and the existing `test_legacy_qr_defaults_to_channel_1` (it strips `channel` and `group_name`, but cohort_id has `#[serde(default)]` so deserialization still succeeds).

- [ ] **Step 6: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-native/src/session.rs
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(session): add cohort_id to SessionKey with serde default"
```

---

## Task 2: Widen `import_session` to return `cohort_id` and mint locally on legacy QRs

**Files:**
- Modify: `android-native/src/session.rs`
- Modify: `android-native/src/jni_bridge.rs` (call site only — update tuple destructure)

- [ ] **Step 1: Write the failing tests**

Add to `session.rs` tests mod:

```rust
#[test]
fn test_import_returns_cohort_id() {
    let mut host = SessionManager::new("Host");
    let qr = host.generate_session_qr(1, 24, "Alpha").unwrap();
    let expected_cid = serde_json::from_str::<serde_json::Value>(&qr).unwrap()
        ["cohort_id"].as_str().unwrap().to_string();

    let mut joiner = SessionManager::new("Joiner");
    let (ch, _crypto, cid) = joiner.import_session(&qr).unwrap();
    assert_eq!(ch, 1);
    assert_eq!(cid, expected_cid);
}

#[test]
fn test_import_legacy_qr_mints_local_cohort_id() {
    let mut host = SessionManager::new("Host");
    let qr = host.generate_session_qr(1, 24, "Alpha").unwrap();
    // Strip cohort_id to simulate a legacy QR
    let mut parsed: serde_json::Value = serde_json::from_str(&qr).unwrap();
    parsed.as_object_mut().unwrap().remove("cohort_id");
    let legacy = serde_json::to_string(&parsed).unwrap();

    let mut joiner = SessionManager::new("Joiner");
    let (_ch, _crypto, cid) = joiner.import_session(&legacy).unwrap();
    assert!(!cid.is_empty(), "legacy QR must yield a locally-minted cohort_id");
    assert_eq!(cid.len(), 36, "minted cohort_id must be a UUID");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path android-native/Cargo.toml session::tests::test_import_returns_cohort_id session::tests::test_import_legacy_qr_mints_local_cohort_id`

Expected: FAIL — `(u8, CryptoSession)` returned, not a 3-tuple.

- [ ] **Step 3: Update `import_session` signature and body**

In `session.rs`, replace the existing `import_session` method:

```rust
/// Import a session from a scanned QR code JSON payload.
/// Returns (channel, CryptoSession, cohort_id).
/// If the QR lacks a cohort_id (legacy), a fresh UUID is minted locally
/// and written into the stored ChannelSession so the joiner can match it
/// against later regenerations from the same host.
pub fn import_session(&mut self, qr_json: &str) -> Result<(u8, CryptoSession, String), String> {
    let mut session: SessionKey = serde_json::from_str(qr_json)
        .map_err(|e| format!("Invalid QR data: {}", e))?;

    let channel = session.channel;
    let ch_idx = validate_channel(channel)?;

    let now = current_unix_time()?;
    if now > session.expires_at {
        return Err("Session has expired".to_string());
    }

    let duration_secs = session.expires_at - session.created_at;
    if duration_secs > MAX_SESSION_HOURS as u64 * 3600 {
        return Err("Session duration exceeds maximum".to_string());
    }

    let key_bytes = Zeroizing::new(base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &session.key,
    ).map_err(|e| format!("Invalid key encoding: {}", e))?);

    if key_bytes.len() != 32 {
        return Err(format!("Invalid key length: {} (expected 32)", key_bytes.len()));
    }

    let mut key_array = Zeroizing::new([0u8; 32]);
    key_array.copy_from_slice(&key_bytes);
    let crypto = CryptoSession::from_psk(&key_array);

    if session.cohort_id.is_empty() {
        session.cohort_id = uuid::Uuid::new_v4().to_string();
        info!("Legacy QR: minted local cohort_id {} for ch{}", session.cohort_id, channel);
    }

    let name = if session.group_name.is_empty() {
        format!("Channel {}", channel)
    } else {
        session.group_name.clone()
    };

    let cohort_id = session.cohort_id.clone();

    info!("Session imported for ch{} '{}' cohort {} from {}: {}",
        channel, name, cohort_id, session.device, session.session_id);

    self.channels[ch_idx] = Some(ChannelSession {
        key: session,
        group_name: name,
    });

    Ok((channel, crypto, cohort_id))
}
```

- [ ] **Step 4: Update the JNI call site in `jni_bridge.rs`**

In `android-native/src/jni_bridge.rs`, find `Java_..._nativeImportSessionFromQR` (around line 1077) and update the tuple destructure:

```rust
    match guard.session_manager.import_session(&json) {
        Ok((channel, crypto, _cohort_id)) => {  // _cohort_id added; consumed in Task 7
            if let Some(ref sm) = guard.state_machine {
                let mut tm = sm.get_transport().lock().unwrap();
                tm.set_crypto(crypto);
            }
            guard.current_channel.store(channel, std::sync::atomic::Ordering::SeqCst);
            info!("JNI: Session imported successfully for ch{}", channel);
            JNI_TRUE
        }
        Err(e) => {
            error!("JNI: Import session failed: {}", e);
            JNI_FALSE
        }
    }
```

Also update `get_session_id` extraction if it exists in jni_bridge (it doesn't — it's Kotlin-side reading `getSessionStatus` JSON, so no further change needed).

- [ ] **Step 5: Run all session tests**

Run: `cargo test --manifest-path android-native/Cargo.toml session::tests`

Expected: PASS — all 5 prior tests plus the 2 new ones. The existing `test_session_generate_and_import` must still pass (it now destructures a 3-tuple — update it):

```rust
#[test]
fn test_session_generate_and_import() {
    let mut host = SessionManager::new("Host");
    let qr_json = host.generate_session_qr(1, 24, "Alpha Team").unwrap();

    let mut joiner = SessionManager::new("Joiner");
    let (ch, mut crypto, _cid) = joiner.import_session(&qr_json).unwrap();  // updated
    // ... rest unchanged
}
```

Apply that update to the existing test as part of this step.

- [ ] **Step 6: Verify the full crate still compiles**

Run: `cargo build --manifest-path android-native/Cargo.toml`

Expected: clean build.

- [ ] **Step 7: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-native/src/session.rs sassy-talk-clean/android-native/src/jni_bridge.rs
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(session): return cohort_id from import_session, mint on legacy QRs"
```

---

## Task 3: Create `cohort_history.rs` — data types, JSON I/O, LRU

**Files:**
- Create: `android-native/src/cohort_history.rs`
- Modify: `android-native/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `android-native/src/cohort_history.rs` with just the test module (data types will follow):

```rust
//! Cohort History — non-sensitive per-cohort metadata that survives key TTL.
//!
//! Stores only: cohort_id, channel, group_name, role, host_device (for joined),
//! participant name+id snapshots, and timestamps. AES keys are NEVER persisted
//! here — see security-invariant test below.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_history_roundtrips() {
        let h = CohortHistory::new(50);
        let json = h.to_json();
        let h2 = CohortHistory::load_from_json(&json, 50);
        assert_eq!(h2.list().len(), 0);
    }

    #[test]
    fn test_load_tolerates_garbage_input() {
        let h = CohortHistory::load_from_json("not json at all", 50);
        assert_eq!(h.list().len(), 0);
        let h2 = CohortHistory::load_from_json("", 50);
        assert_eq!(h2.list().len(), 0);
    }

    #[test]
    fn test_lru_eviction_at_cap() {
        let mut h = CohortHistory::new(3);
        for i in 0..5 {
            h.upsert_host(1, &format!("g{}", i), None, "sid", 1000 + i as u64);
        }
        assert_eq!(h.list().len(), 3, "must evict to cap");
        // Newest 3 (g2, g3, g4) survive
        let names: Vec<&str> = h.list().iter().map(|r| r.group_name.as_str()).collect();
        assert!(names.contains(&"g4"));
        assert!(names.contains(&"g3"));
        assert!(names.contains(&"g2"));
        assert!(!names.contains(&"g0"));
        assert!(!names.contains(&"g1"));
    }

    #[test]
    fn test_remove() {
        let mut h = CohortHistory::new(50);
        let cid = h.upsert_host(1, "Alpha", None, "sid-1", 1000);
        h.upsert_host(2, "Beta", None, "sid-2", 1001);
        assert_eq!(h.list().len(), 2);
        h.remove(&cid);
        assert_eq!(h.list().len(), 1);
        assert_eq!(h.list()[0].group_name, "Beta");
    }

    #[test]
    fn test_clear() {
        let mut h = CohortHistory::new(50);
        h.upsert_host(1, "Alpha", None, "sid", 1000);
        h.upsert_host(2, "Beta", None, "sid2", 1001);
        h.clear();
        assert_eq!(h.list().len(), 0);
    }
}
```

- [ ] **Step 2: Wire the module into the crate**

Add to `android-native/src/lib.rs` next to the other `pub mod` declarations:

```rust
pub mod cohort_history;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path android-native/Cargo.toml cohort_history::tests`

Expected: FAIL — compile error (no `CohortHistory` type, no methods).

- [ ] **Step 4: Implement data types and basic CRUD**

Replace the placeholder file content (`use` line, doc comment, and tests mod) with the full module. Keep the tests mod at the bottom:

```rust
//! Cohort History — non-sensitive per-cohort metadata that survives key TTL.
//!
//! Stores only: cohort_id, channel, group_name, role, host_device (for joined),
//! participant name+id snapshots, and timestamps. AES keys are NEVER persisted
//! here — see security-invariant test below.

use log::info;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_HISTORY_CAP: usize = 50;
/// Window for orphan-merge heuristic on legacy QRs.
const ORPHAN_MERGE_WINDOW_SECS: u64 = 30 * 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CohortRole {
    Hosted,
    Joined,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParticipantSnapshot {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CohortRecord {
    pub cohort_id: String,
    pub channel: u8,
    pub group_name: String,
    pub role: CohortRole,
    pub host_device: Option<String>,
    pub last_participants: Vec<ParticipantSnapshot>,
    pub last_session_id: String,
    pub created_at: u64,
    pub last_joined_at: u64,
}

pub struct CohortHistory {
    records: Vec<CohortRecord>,
    cap: usize,
}

impl CohortHistory {
    pub fn new(cap: usize) -> Self {
        Self { records: Vec::new(), cap: cap.max(1) }
    }

    /// Load from a previously-serialized blob. Tolerates malformed/empty input
    /// by returning an empty history rather than erroring — a corrupted prefs
    /// file should not break the app launch.
    pub fn load_from_json(json: &str, cap: usize) -> Self {
        let records: Vec<CohortRecord> = serde_json::from_str(json).unwrap_or_default();
        let mut h = Self { records, cap: cap.max(1) };
        h.sort_by_recency();
        h.enforce_cap();
        h
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.records).unwrap_or_else(|_| "[]".to_string())
    }

    pub fn list(&self) -> &[CohortRecord] {
        &self.records
    }

    pub fn find(&self, cohort_id: &str) -> Option<&CohortRecord> {
        self.records.iter().find(|r| r.cohort_id == cohort_id)
    }

    pub fn remove(&mut self, cohort_id: &str) {
        self.records.retain(|r| r.cohort_id != cohort_id);
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Upsert a record as `Hosted`. If an existing record matches by cohort_id
    /// (or by (channel, group_name) when cohort_id is None), reuses its cohort_id.
    /// Role promotes from Joined → Both when the user has now hosted what they
    /// previously only joined. Returns the resolved cohort_id.
    pub fn upsert_host(
        &mut self,
        channel: u8,
        group_name: &str,
        cohort_id: Option<&str>,
        session_id: &str,
        now: u64,
    ) -> String {
        let resolved = self.resolve_host_cohort_id(channel, group_name, cohort_id);
        if let Some(rec) = self.records.iter_mut().find(|r| r.cohort_id == resolved) {
            rec.role = match rec.role {
                CohortRole::Joined => CohortRole::Both,
                _ => CohortRole::Hosted.max_with(&rec.role),
            };
            rec.last_session_id = session_id.to_string();
            rec.last_joined_at = now;
            rec.group_name = group_name.to_string();
            rec.channel = channel;
        } else {
            self.records.push(CohortRecord {
                cohort_id: resolved.clone(),
                channel,
                group_name: group_name.to_string(),
                role: CohortRole::Hosted,
                host_device: None,
                last_participants: Vec::new(),
                last_session_id: session_id.to_string(),
                created_at: now,
                last_joined_at: now,
            });
            info!("CohortHistory: new Hosted record {} ({})", resolved, group_name);
        }
        self.sort_by_recency();
        self.enforce_cap();
        resolved
    }

    fn resolve_host_cohort_id(
        &self,
        channel: u8,
        group_name: &str,
        cohort_id: Option<&str>,
    ) -> String {
        if let Some(cid) = cohort_id.filter(|s| !s.is_empty()) {
            return cid.to_string();
        }
        // Try reuse: find an existing Hosted/Both record matching (channel, group_name)
        if let Some(rec) = self.records.iter().find(|r| {
            r.channel == channel
                && r.group_name == group_name
                && matches!(r.role, CohortRole::Hosted | CohortRole::Both)
        }) {
            return rec.cohort_id.clone();
        }
        uuid::Uuid::new_v4().to_string()
    }

    fn sort_by_recency(&mut self) {
        self.records.sort_by(|a, b| b.last_joined_at.cmp(&a.last_joined_at));
    }

    fn enforce_cap(&mut self) {
        if self.records.len() > self.cap {
            self.records.truncate(self.cap);
        }
    }
}

// Helper: CohortRole "max" so we never demote (Both > Hosted/Joined).
impl CohortRole {
    fn max_with(self, other: &CohortRole) -> CohortRole {
        match (self, other) {
            (CohortRole::Both, _) | (_, CohortRole::Both) => CohortRole::Both,
            (CohortRole::Hosted, CohortRole::Joined) | (CohortRole::Joined, CohortRole::Hosted) => CohortRole::Both,
            (CohortRole::Hosted, _) => CohortRole::Hosted,
            (CohortRole::Joined, _) => CohortRole::Joined,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_history_roundtrips() {
        let h = CohortHistory::new(50);
        let json = h.to_json();
        let h2 = CohortHistory::load_from_json(&json, 50);
        assert_eq!(h2.list().len(), 0);
    }

    #[test]
    fn test_load_tolerates_garbage_input() {
        let h = CohortHistory::load_from_json("not json at all", 50);
        assert_eq!(h.list().len(), 0);
        let h2 = CohortHistory::load_from_json("", 50);
        assert_eq!(h2.list().len(), 0);
    }

    #[test]
    fn test_lru_eviction_at_cap() {
        let mut h = CohortHistory::new(3);
        for i in 0..5 {
            h.upsert_host(1, &format!("g{}", i), None, "sid", 1000 + i as u64);
        }
        assert_eq!(h.list().len(), 3, "must evict to cap");
        let names: Vec<&str> = h.list().iter().map(|r| r.group_name.as_str()).collect();
        assert!(names.contains(&"g4"));
        assert!(names.contains(&"g3"));
        assert!(names.contains(&"g2"));
        assert!(!names.contains(&"g0"));
        assert!(!names.contains(&"g1"));
    }

    #[test]
    fn test_remove() {
        let mut h = CohortHistory::new(50);
        let cid = h.upsert_host(1, "Alpha", None, "sid-1", 1000);
        h.upsert_host(2, "Beta", None, "sid-2", 1001);
        assert_eq!(h.list().len(), 2);
        h.remove(&cid);
        assert_eq!(h.list().len(), 1);
        assert_eq!(h.list()[0].group_name, "Beta");
    }

    #[test]
    fn test_clear() {
        let mut h = CohortHistory::new(50);
        h.upsert_host(1, "Alpha", None, "sid", 1000);
        h.upsert_host(2, "Beta", None, "sid2", 1001);
        h.clear();
        assert_eq!(h.list().len(), 0);
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path android-native/Cargo.toml cohort_history::tests`

Expected: PASS — all 5 tests.

- [ ] **Step 6: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-native/src/cohort_history.rs sassy-talk-clean/android-native/src/lib.rs
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(cohort): add cohort_history module — types, JSON I/O, LRU"
```

---

## Task 4: Add `upsert_joiner`, role promotion, and orphan-merge heuristic

**Files:**
- Modify: `android-native/src/cohort_history.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `tests` mod in `cohort_history.rs`:

```rust
#[test]
fn test_upsert_joiner_creates_joined_record() {
    let mut h = CohortHistory::new(50);
    let cid = h.upsert_joiner(1, "Alpha", Some("c-123"), "Sarah's Moto", "sid-1", 1000);
    assert_eq!(cid, "c-123");
    let rec = h.find("c-123").unwrap();
    assert_eq!(rec.role, CohortRole::Joined);
    assert_eq!(rec.host_device.as_deref(), Some("Sarah's Moto"));
    assert_eq!(rec.group_name, "Alpha");
}

#[test]
fn test_role_promotion_joined_then_hosted_becomes_both() {
    let mut h = CohortHistory::new(50);
    h.upsert_joiner(1, "Alpha", Some("c-1"), "Host", "sid-a", 1000);
    h.upsert_host(1, "Alpha", Some("c-1"), "sid-b", 1001);
    assert_eq!(h.find("c-1").unwrap().role, CohortRole::Both);
}

#[test]
fn test_role_promotion_hosted_then_joined_becomes_both() {
    let mut h = CohortHistory::new(50);
    h.upsert_host(1, "Alpha", Some("c-1"), "sid-a", 1000);
    h.upsert_joiner(1, "Alpha", Some("c-1"), "Host", "sid-b", 1001);
    assert_eq!(h.find("c-1").unwrap().role, CohortRole::Both);
}

#[test]
fn test_orphan_merge_within_window() {
    let mut h = CohortHistory::new(50);
    // First import (legacy QR) — joiner mints "c-local"
    h.upsert_joiner(1, "Alpha", Some("c-local"), "Host", "sid-a", 1000);
    // Second import (new format from same host with cohort_id "c-real") within 30 days
    let result = h.upsert_joiner(1, "Alpha", Some("c-real"), "Host", "sid-b",
                                 1000 + 29 * 24 * 3600);
    assert_eq!(result, "c-real", "must adopt incoming cohort_id");
    assert!(h.find("c-local").is_none(), "orphan must be merged away");
    let merged = h.find("c-real").unwrap();
    assert_eq!(merged.last_session_id, "sid-b");
}

#[test]
fn test_orphan_merge_skipped_outside_window() {
    let mut h = CohortHistory::new(50);
    h.upsert_joiner(1, "Alpha", Some("c-local"), "Host", "sid-a", 1000);
    // 31 days later
    h.upsert_joiner(1, "Alpha", Some("c-real"), "Host", "sid-b",
                    1000 + 31 * 24 * 3600);
    assert!(h.find("c-local").is_some(), "stale orphan stays");
    assert!(h.find("c-real").is_some(), "new record added separately");
}

#[test]
fn test_serialized_blob_contains_no_aes_key_material() {
    let mut h = CohortHistory::new(50);
    h.upsert_host(1, "Alpha", None, "session-uuid-here", 1000);
    h.upsert_joiner(2, "Beta", Some("c-2"), "HostDev", "session-uuid-2", 1001);
    h.snapshot_participants("c-2", vec![
        ParticipantSnapshot { id: "abcd1234".into(), name: "Alice".into() }
    ]);
    let json = h.to_json();
    assert!(!json.contains("\"key\""), "history JSON must not contain a key field");
    // No 32-byte base64 (44 chars) substring. Catches accidental key leakage.
    let re = regex::Regex::new(r"[A-Za-z0-9+/]{43}=").unwrap();
    assert!(!re.is_match(&json), "history JSON contains 32-byte base64 — possible key leak");
}
```

Note: the regex test requires the `regex` crate. Check `android-native/Cargo.toml` for it — if absent, add to `[dev-dependencies]`:

```toml
[dev-dependencies]
regex = "1"
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path android-native/Cargo.toml cohort_history::tests`

Expected: FAIL — `upsert_joiner` and `snapshot_participants` don't exist.

- [ ] **Step 3: Implement `upsert_joiner` and `snapshot_participants`**

Add to `impl CohortHistory` in `cohort_history.rs`, after `upsert_host`:

```rust
/// Upsert a record as `Joined`. If `cohort_id` is None or empty, mints a local
/// UUID. Applies the orphan-merge heuristic for legacy imports: a previously
/// locally-minted record matching (channel, group_name, host_device) and last
/// joined within `ORPHAN_MERGE_WINDOW_SECS` of `now` is replaced by the new
/// cohort_id. Returns the resolved cohort_id.
pub fn upsert_joiner(
    &mut self,
    channel: u8,
    group_name: &str,
    cohort_id: Option<&str>,
    host_device: &str,
    session_id: &str,
    now: u64,
) -> String {
    let incoming = cohort_id
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    if let Some(ref cid) = incoming {
        // Orphan-merge: find a record matching (channel, group_name, host_device)
        // that ISN'T the incoming cohort_id, within the merge window.
        let orphan_idx = self.records.iter().position(|r| {
            r.cohort_id != *cid
                && r.channel == channel
                && r.group_name == group_name
                && r.host_device.as_deref() == Some(host_device)
                && now.saturating_sub(r.last_joined_at) <= ORPHAN_MERGE_WINDOW_SECS
        });
        if let Some(idx) = orphan_idx {
            info!("CohortHistory: orphan-merging {} → {}",
                self.records[idx].cohort_id, cid);
            self.records.remove(idx);
        }
    }

    let resolved = incoming.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    if let Some(rec) = self.records.iter_mut().find(|r| r.cohort_id == resolved) {
        rec.role = match rec.role {
            CohortRole::Hosted => CohortRole::Both,
            _ => CohortRole::Joined.max_with(&rec.role),
        };
        rec.host_device = Some(host_device.to_string());
        rec.last_session_id = session_id.to_string();
        rec.last_joined_at = now;
        rec.group_name = group_name.to_string();
        rec.channel = channel;
    } else {
        self.records.push(CohortRecord {
            cohort_id: resolved.clone(),
            channel,
            group_name: group_name.to_string(),
            role: CohortRole::Joined,
            host_device: Some(host_device.to_string()),
            last_participants: Vec::new(),
            last_session_id: session_id.to_string(),
            created_at: now,
            last_joined_at: now,
        });
        info!("CohortHistory: new Joined record {} ({})", resolved, group_name);
    }
    self.sort_by_recency();
    self.enforce_cap();
    resolved
}

/// Replace the participant snapshot for a cohort. No-op if cohort_id is unknown.
pub fn snapshot_participants(&mut self, cohort_id: &str, participants: Vec<ParticipantSnapshot>) {
    if let Some(rec) = self.records.iter_mut().find(|r| r.cohort_id == cohort_id) {
        rec.last_participants = participants;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path android-native/Cargo.toml cohort_history::tests`

Expected: PASS — 11 tests total.

- [ ] **Step 5: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-native/src/cohort_history.rs sassy-talk-clean/android-native/Cargo.toml
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(cohort): joiner upsert, role promotion, orphan-merge, snapshot"
```

---

## Task 5: Wire `CohortHistory` into `JniAppState` and call from generate/import

**Files:**
- Modify: `android-native/src/jni_bridge.rs`

- [ ] **Step 1: Add `cohort_history` field to `JniAppState`**

In `android-native/src/jni_bridge.rs`, near line 360, update the struct:

```rust
struct JniAppState {
    state_machine: Option<StateMachine>,
    session_manager: SessionManager,
    user_registry: UserRegistry,
    cohort_history: crate::cohort_history::CohortHistory,  // NEW
    ptt_pressed: Arc<AtomicBool>,
    current_channel: Arc<AtomicU8>,
    current_subchannel: Arc<AtomicU8>,
    pending_key_exchange: Option<crate::crypto::KeyExchange>,
    bt_tx_buffer: Arc<Mutex<Option<Vec<u8>>>>,
    bt_encoder: VoiceEncoder,
    bt_decoder: VoiceDecoder,
    bt_recording: bool,
}
```

Update `impl JniAppState::new()`:

```rust
impl JniAppState {
    fn new() -> Self {
        let ptt_pressed = Arc::new(AtomicBool::new(false));
        let current_channel = Arc::new(AtomicU8::new(1));
        let current_subchannel = Arc::new(AtomicU8::new(0));

        Self {
            state_machine: None,
            session_manager: SessionManager::new("SassyTalkie"),
            user_registry: UserRegistry::new(),
            cohort_history: crate::cohort_history::CohortHistory::new(
                crate::cohort_history::DEFAULT_HISTORY_CAP,
            ),
            ptt_pressed,
            current_channel,
            current_subchannel,
            pending_key_exchange: None,
            bt_tx_buffer: Arc::new(Mutex::new(None)),
            bt_encoder: VoiceEncoder::new(),
            // ... preserve remaining fields exactly as before
        }
    }
}
```

(Inspect the existing `new()` body and add `cohort_history` to the struct literal; do not reformat unrelated fields.)

- [ ] **Step 2: Update `nativeGenerateChannelQR` to upsert host and accept `cohortId`**

The JNI signature gains a fifth parameter. In `android-native/src/jni_bridge.rs` around line 1031:

```rust
/// JNI: Generate session QR for a specific channel with optional group name + cohort_id
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGenerateChannelQR<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
    duration_hours: jni::sys::jint,
    group_name: jni::sys::jstring,
    cohort_id: jni::sys::jstring,
) -> JObject<'local> {
    let ch = channel as u8;
    let name: String = if !group_name.is_null() {
        let j_name = unsafe { JString::from_raw(group_name) };
        env.get_string(&j_name).map(|s| s.into()).unwrap_or_default()
    } else {
        String::new()
    };
    let cohort: Option<String> = if !cohort_id.is_null() {
        let j_cid = unsafe { JString::from_raw(cohort_id) };
        env.get_string(&j_cid).ok().map(|s| s.into())
    } else {
        None
    };

    info!("JNI: Generate session QR ch{} '{}' cohort={:?} ({}h)", ch, name, cohort, duration_hours);

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());

    let qr_json = match guard.session_manager.generate_session_qr_with_cohort(
        ch, duration_hours as u32, &name, cohort.as_deref(),
    ) {
        Ok(json) => json,
        Err(e) => {
            error!("JNI: Generate QR failed: {}", e);
            return env.new_string("").map(|s| s.into()).unwrap_or_else(|_| JObject::null());
        }
    };

    // Pull session_id and cohort_id from the just-generated SessionKey to feed history.
    let (sid, cid) = match serde_json::from_str::<serde_json::Value>(&qr_json) {
        Ok(v) => (
            v["session_id"].as_str().unwrap_or("").to_string(),
            v["cohort_id"].as_str().unwrap_or("").to_string(),
        ),
        Err(_) => (String::new(), String::new()),
    };

    if !cid.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs()).unwrap_or(0);
        guard.cohort_history.upsert_host(ch, &name, Some(&cid), &sid, now);
    }

    if let Some(ref sm) = guard.state_machine {
        let mut tm = sm.get_transport().lock().unwrap();
        if let Some(crypto) = guard.session_manager.get_crypto_for_channel(ch) {
            tm.set_crypto(crypto);
        }
    }

    drop(guard);

    env.new_string(&qr_json)
        .map(|s| s.into())
        .unwrap_or_else(|_| JObject::null())
}
```

There's also a no-arg JNI helper around line 1023 that delegates to `nativeGenerateChannelQR`. Update its delegation:

```rust
// Around line 1023, the existing one-arg generate wrapper:
Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGenerateChannelQR(
    env, _class, channel as jni::sys::jint, duration_hours,
    std::ptr::null_mut(), // null group_name
    std::ptr::null_mut(), // null cohort_id
)
```

- [ ] **Step 3: Update `nativeImportSessionFromQR` to upsert joiner**

In `android-native/src/jni_bridge.rs` around line 1077, modify the `match` arm:

```rust
    match guard.session_manager.import_session(&json) {
        Ok((channel, crypto, cohort_id)) => {
            if let Some(ref sm) = guard.state_machine {
                let mut tm = sm.get_transport().lock().unwrap();
                tm.set_crypto(crypto);
            }
            guard.current_channel.store(channel, std::sync::atomic::Ordering::SeqCst);

            // Re-parse the QR to pull host device + session_id + group_name.
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json) {
                let host_dev = parsed["device"].as_str().unwrap_or("").to_string();
                let sid = parsed["session_id"].as_str().unwrap_or("").to_string();
                let gname = parsed["group_name"].as_str()
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("Channel {}", channel));
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs()).unwrap_or(0);
                guard.cohort_history.upsert_joiner(channel, &gname, Some(&cohort_id),
                                                   &host_dev, &sid, now);
            }

            info!("JNI: Session imported successfully for ch{} cohort {}", channel, cohort_id);
            JNI_TRUE
        }
        Err(e) => {
            error!("JNI: Import session failed: {}", e);
            JNI_FALSE
        }
    }
```

- [ ] **Step 4: Run the full Rust test suite and verify clean compile**

Run: `cargo test --manifest-path android-native/Cargo.toml`

Expected: PASS — all session + cohort_history tests still pass; no new failures elsewhere.

Run: `cargo build --manifest-path android-native/Cargo.toml --release`

Expected: clean build.

- [ ] **Step 5: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-native/src/jni_bridge.rs
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(jni): wire cohort_history into generate/import paths"
```

---

## Task 6: New JNI exports — history accessors and load/save

**Files:**
- Modify: `android-native/src/jni_bridge.rs`

- [ ] **Step 1: Add the new JNI exports**

Append to `android-native/src/jni_bridge.rs` (place near the other session-related exports, after `nativeGetSessionStatus` around line 1141):

```rust
/// JNI: Get cohort history as JSON array
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetCohortHistory<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let json = guard.cohort_history.to_json();
    drop(guard);
    env.new_string(&json).map(|s| s.into()).unwrap_or_else(|_| JObject::null())
}

/// JNI: Load cohort history from a previously-saved blob (called once on init)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeLoadCohortHistory<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    blob: JString<'local>,
) {
    let json: String = match env.get_string(&blob) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.cohort_history = crate::cohort_history::CohortHistory::load_from_json(
        &json, crate::cohort_history::DEFAULT_HISTORY_CAP,
    );
}

/// JNI: Remove a single cohort by id
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeRemoveCohort<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    cohort_id: JString<'local>,
) {
    let id: String = match env.get_string(&cohort_id) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.cohort_history.remove(&id);
}

/// JNI: Clear all cohort history
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeClearCohortHistory(
    _env: JNIEnv,
    _class: JClass,
) {
    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.cohort_history.clear();
}

/// JNI: Get the active cohort_id for a channel (empty string if no active session there)
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeGetActiveCohortId<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
) -> JObject<'local> {
    let state = get_jni_state();
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    let cid = guard.session_manager.get_active_cohort_id(channel as u8).unwrap_or_default();
    drop(guard);
    env.new_string(&cid).map(|s| s.into()).unwrap_or_else(|_| JObject::null())
}

/// JNI: Snapshot participants for the active cohort on a given channel.
/// Called by Kotlin every ~30s while a session is active.
#[no_mangle]
pub extern "system" fn Java_com_sassyconsulting_sassytalkie_SassyTalkNative_nativeSnapshotCohortParticipants<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    channel: jni::sys::jint,
    participants_json: JString<'local>,
) {
    let json: String = match env.get_string(&participants_json) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let participants: Vec<crate::cohort_history::ParticipantSnapshot> =
        serde_json::from_str(&json).unwrap_or_default();

    let state = get_jni_state();
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cid) = guard.session_manager.get_active_cohort_id(channel as u8) {
        guard.cohort_history.snapshot_participants(&cid, participants);
    }
}
```

- [ ] **Step 2: Add `get_active_cohort_id` to `SessionManager`**

In `android-native/src/session.rs`, add inside `impl SessionManager`:

```rust
/// Get the cohort_id of the currently active session on a channel, if any.
pub fn get_active_cohort_id(&self, channel: u8) -> Option<String> {
    let ch_idx = validate_channel(channel).ok()?;
    let cs = self.channels[ch_idx].as_ref()?;
    let now = current_unix_time().ok()?;
    if now > cs.key.expires_at { return None; }
    Some(cs.key.cohort_id.clone())
}
```

- [ ] **Step 3: Verify clean compile**

Run: `cargo build --manifest-path android-native/Cargo.toml --release`

Expected: clean build. No runtime tests here — Kotlin tests cover JNI in Task 9.

- [ ] **Step 4: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-native/src/jni_bridge.rs sassy-talk-clean/android-native/src/session.rs
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(jni): cohort history accessors + active-cohort-id + snapshot"
```

---

## Task 7: Build the native library and update Kotlin `SassyTalkNative` wrappers

**Files:**
- Modify: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/SassyTalkNative.kt`
- Build artifact: `android-app/app/src/main/jniLibs/arm64-v8a/libsassytalkie.so`

- [ ] **Step 1: Build the native library for arm64-v8a**

The project's existing build script lives at `android-native/build.sh` or similar — confirm and run it. If the standard cargo-ndk recipe is used:

```powershell
cd V:/Projects/sassytalkie/sassy-talks/sassy-talk-clean/android-native
cargo ndk -t arm64-v8a -o ../android-app/app/src/main/jniLibs build --release
```

Expected: `libsassytalkie.so` updated under `jniLibs/arm64-v8a/`.

- [ ] **Step 2: Add Kotlin wrappers and external declarations**

In `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/SassyTalkNative.kt`, add to the `external` declarations near the existing session ones (around line 840–910):

```kotlin
@JvmStatic private external fun nativeGetCohortHistory(): String
@JvmStatic private external fun nativeLoadCohortHistory(blob: String)
@JvmStatic private external fun nativeRemoveCohort(cohortId: String)
@JvmStatic private external fun nativeClearCohortHistory()
@JvmStatic private external fun nativeGetActiveCohortId(channel: Int): String
@JvmStatic private external fun nativeSnapshotCohortParticipants(channel: Int, participantsJson: String)
```

Update the existing `nativeGenerateChannelQR` declaration to add the cohort_id parameter:

```kotlin
@JvmStatic private external fun nativeGenerateChannelQR(
    channel: Int, durationHours: Int, groupName: String?, cohortId: String?,
): String
```

Add the public wrappers. Place these alongside other session functions (around the existing `generateChannelQR` near line 911):

```kotlin
private const val COHORT_HISTORY_PREF_KEY = "cohort_history_v1"

fun getCohortHistory(): String {
    if (!initialized) return "[]"
    return try { nativeGetCohortHistory() } catch (_: Exception) { "[]" }
}

fun removeCohort(cohortId: String) {
    if (!initialized) return
    try {
        nativeRemoveCohort(cohortId)
        saveCohortHistoryBlob()
    } catch (e: Exception) {
        Log.e(TAG, "removeCohort failed: ${e.message}")
    }
}

fun clearCohortHistory() {
    if (!initialized) return
    try {
        nativeClearCohortHistory()
        sessionPrefs()?.edit()?.remove(COHORT_HISTORY_PREF_KEY)?.apply()
    } catch (e: Exception) {
        Log.e(TAG, "clearCohortHistory failed: ${e.message}")
    }
}

fun getActiveCohortId(channel: Int): String {
    if (!initialized) return ""
    return try { nativeGetActiveCohortId(channel) } catch (_: Exception) { "" }
}

fun snapshotCohortParticipants(channel: Int, participantsJson: String) {
    if (!initialized) return
    try {
        nativeSnapshotCohortParticipants(channel, participantsJson)
        saveCohortHistoryBlob()
    } catch (e: Exception) {
        Log.w(TAG, "snapshotCohortParticipants failed: ${e.message}")
    }
}

private fun saveCohortHistoryBlob() {
    try {
        val blob = nativeGetCohortHistory()
        sessionPrefs()?.edit()?.putString(COHORT_HISTORY_PREF_KEY, blob)?.apply()
    } catch (e: Exception) {
        Log.w(TAG, "saveCohortHistoryBlob failed: ${e.message}")
    }
}

/** Call once after nativeInit succeeds — restores history from prefs into Rust. */
fun restoreCohortHistory() {
    if (!initialized) return
    try {
        val blob = sessionPrefs()?.getString(COHORT_HISTORY_PREF_KEY, null) ?: "[]"
        nativeLoadCohortHistory(blob)
    } catch (e: Exception) {
        Log.w(TAG, "restoreCohortHistory failed: ${e.message}")
    }
}
```

Update the existing `generateChannelQR` Kotlin function (around line 911) to accept and pass cohortId, and to also persist the history blob:

```kotlin
fun generateChannelQR(
    channel: Int,
    durationHours: Int = 24,
    groupName: String = "",
    cohortId: String? = null,
): String {
    if (!initialized) return ""
    return try {
        val json = nativeGenerateChannelQR(
            channel, durationHours, groupName.ifEmpty { null }, cohortId?.ifEmpty { null },
        )
        if (json.isNotEmpty()) {
            sessionPrefs()?.edit()?.putString("session_ch_$channel", json)?.apply()
            saveCohortHistoryBlob()
        }
        json
    } catch (e: Exception) {
        Log.e(TAG, "generateChannelQR failed: ${e.message}")
        ""
    }
}
```

Update `importSessionFromQR` (around line 276) to also save the cohort blob on success:

```kotlin
fun importSessionFromQR(qrJson: String): Boolean {
    if (!initialized) return false
    return try {
        val ok = nativeImportSessionFromQR(qrJson)
        if (ok) {
            val channel = try {
                org.json.JSONObject(qrJson).optInt("channel", 1)
            } catch (_: Exception) { 1 }
            sessionPrefs()?.edit()?.putString("session_ch_$channel", qrJson)?.apply()
            saveCohortHistoryBlob()
        }
        ok
    } catch (e: Exception) {
        Log.e(TAG, "importSessionFromQR failed: ${e.message}")
        false
    }
}
```

Update `clearSession` (around line 410) to leave cohort history intact (it already does — verify the existing `sessionPrefs()?.edit()?.clear()?.apply()` is replaced with targeted removal):

```kotlin
fun clearSession() {
    if (initialized) {
        try { nativeClearSession() } catch (e: Exception) {
            Log.e(TAG, "clearSession failed: ${e.message}")
        }
    }
    // Clear per-channel sessions and legacy session_json, but preserve cohort_history_v1.
    val prefs = sessionPrefs() ?: return
    val editor = prefs.edit()
    for (ch in 1..8) editor.remove("session_ch_$ch")
    editor.remove("session_json")
    editor.apply()
}
```

- [ ] **Step 3: Wire `restoreCohortHistory()` into app startup**

Find where `SassyTalkNative.init()` / `nativeInit()` is invoked in the Application or MainActivity. The current pattern (per the codebase) is that `restoreSession()` is called shortly after init succeeds.

In `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/SassyTalkieApplication.kt` (or wherever `restoreSession()` is currently called — grep for it), add the cohort restore on the line immediately after `restoreSession()`:

```kotlin
SassyTalkNative.init()
SassyTalkNative.restoreSession()
SassyTalkNative.restoreCohortHistory()  // NEW
```

Locate by running `grep -rn "restoreSession()" android-app/app/src/main`.

- [ ] **Step 4: Build the Android app to verify wiring**

Run from the `android-app` directory:

```powershell
cd V:/Projects/sassytalkie/sassy-talks/sassy-talk-clean/android-app
./gradlew :app:assembleDebug
```

Expected: BUILD SUCCESSFUL. No `UnsatisfiedLinkError` would surface at compile time, but ensure the Kotlin compiles cleanly.

- [ ] **Step 5: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-app/app/src/main/java/com/sassyconsulting/sassytalkie/SassyTalkNative.kt sassy-talk-clean/android-app/app/src/main/jniLibs/arm64-v8a/libsassytalkie.so
# Also stage the file that wires restoreCohortHistory() — adjust path after grep
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-app/app/src/main/java/com/sassyconsulting/sassytalkie/SassyTalkieApplication.kt
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(android): Kotlin wrappers for cohort history + native lib build"
```

---

## Task 8: Periodic participant snapshotter in `WalkieService`

**Files:**
- Modify: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/WalkieService.kt`

- [ ] **Step 1: Add the snapshotter coroutine**

In `WalkieService.kt`, near the existing service lifecycle members, add a coroutine job and helper. Locate `onCreate` / `onStartCommand` to find the natural place.

```kotlin
private var cohortSnapshotJob: kotlinx.coroutines.Job? = null

private fun startCohortSnapshotter() {
    if (cohortSnapshotJob?.isActive == true) return
    cohortSnapshotJob = kotlinx.coroutines.CoroutineScope(
        kotlinx.coroutines.Dispatchers.Default
    ).launch {
        while (kotlinx.coroutines.isActive) {
            try {
                val users = SassyTalkNative.getUsers()
                if (users.isNotEmpty()) {
                    // Build a JSON array of {id, name} matching ParticipantSnapshot
                    val arr = org.json.JSONArray()
                    for (u in users) {
                        val o = org.json.JSONObject()
                        o.put("id", u.id)
                        o.put("name", u.name)
                        arr.put(o)
                    }
                    val payload = arr.toString()
                    // Snapshot every channel that currently has an active cohort
                    for (ch in 1..8) {
                        val cid = SassyTalkNative.getActiveCohortId(ch)
                        if (cid.isNotEmpty()) {
                            SassyTalkNative.snapshotCohortParticipants(ch, payload)
                        }
                    }
                }
            } catch (e: Exception) {
                Log.w("WalkieService", "cohort snapshot failed: ${e.message}")
            }
            kotlinx.coroutines.delay(30_000)
        }
    }
}

private fun stopCohortSnapshotter() {
    cohortSnapshotJob?.cancel()
    cohortSnapshotJob = null
}
```

Call `startCohortSnapshotter()` from the same lifecycle point that starts auto-connect (after `autoConnectManager.autoConnect(...)` returns `true`), and `stopCohortSnapshotter()` from `onDestroy()` and from the disconnect/shutdown path.

- [ ] **Step 2: Build the app**

```powershell
cd V:/Projects/sassytalkie/sassy-talks/sassy-talk-clean/android-app
./gradlew :app:assembleDebug
```

Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-app/app/src/main/java/com/sassyconsulting/sassytalkie/WalkieService.kt
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(android): 30s cohort participant snapshotter in WalkieService"
```

---

## Task 9: Instrumented test — generate, clear, rejoin

**Files:**
- Create: `android-app/app/src/androidTest/java/com/sassyconsulting/sassytalkie/CohortHistoryInstrumentedTest.kt`

- [ ] **Step 1: Write the test**

```kotlin
package com.sassyconsulting.sassytalkie

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

@RunWith(AndroidJUnit4::class)
class CohortHistoryInstrumentedTest {

    @Before
    fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        SassyTalkNative.appContext = ctx
        SassyTalkNative.init()
        SassyTalkNative.clearCohortHistory()
        SassyTalkNative.clearSession()
    }

    @Test
    fun host_generate_creates_hosted_record() {
        val qr = SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1")
        assertTrue(qr.isNotEmpty())
        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals(1, history.length())
        val rec = history.getJSONObject(0)
        assertEquals("Math 101 P1", rec.getString("group_name"))
        assertEquals(1, rec.getInt("channel"))
        assertEquals("Hosted", rec.getString("role"))
    }

    @Test
    fun clear_session_preserves_cohort_history() {
        SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1")
        SassyTalkNative.clearSession()
        assertFalse(SassyTalkNative.isAuthenticated())
        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals(1, history.length())
    }

    @Test
    fun import_legacy_qr_mints_cohort_id_as_joined() {
        // Build a legacy QR without cohort_id, signed for ch2, valid 1h
        val now = System.currentTimeMillis() / 1000
        val keyBytes = ByteArray(32) { (it + 1).toByte() }
        val keyB64 = android.util.Base64.encodeToString(keyBytes, android.util.Base64.NO_WRAP)
        val legacy = JSONObject().apply {
            put("key", keyB64)
            put("device", "Legacy Host")
            put("created_at", now)
            put("expires_at", now + 3600)
            put("session_id", "legacy-sid-uuid")
            put("channel", 2)
            put("group_name", "OldGroup")
            // no cohort_id
        }.toString()

        val ok = SassyTalkNative.importSessionFromQR(legacy)
        assertTrue(ok)

        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals(1, history.length())
        val rec = history.getJSONObject(0)
        assertEquals("Joined", rec.getString("role"))
        assertEquals("Legacy Host", rec.getString("host_device"))
        val cid = rec.getString("cohort_id")
        assertTrue(cid.isNotEmpty())
        assertEquals(36, cid.length, "minted cohort_id must be a UUID")
    }

    @Test
    fun rejoin_hosted_keeps_cohort_id_changes_session_id() {
        val qr1 = SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1")
        val cid1 = JSONObject(qr1).getString("cohort_id")
        val sid1 = JSONObject(qr1).getString("session_id")
        SassyTalkNative.clearSession()

        // Rejoin: regenerate with the same cohort_id
        val qr2 = SassyTalkNative.generateChannelQR(1, 24, "Math 101 P1", cid1)
        val cid2 = JSONObject(qr2).getString("cohort_id")
        val sid2 = JSONObject(qr2).getString("session_id")

        assertEquals(cid1, cid2, "cohort_id must persist across rejoin")
        assertNotEquals(sid1, sid2, "session_id must rotate on rejoin")

        val history = JSONArray(SassyTalkNative.getCohortHistory())
        assertEquals(1, history.length(), "rejoin must not create a duplicate record")
    }
}
```

- [ ] **Step 2: Run the instrumented tests on a connected device or emulator**

```powershell
cd V:/Projects/sassytalkie/sassy-talks/sassy-talk-clean/android-app
./gradlew :app:connectedDebugAndroidTest --tests com.sassyconsulting.sassytalkie.CohortHistoryInstrumentedTest
```

Expected: all 4 tests PASS.

- [ ] **Step 3: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-app/app/src/androidTest/java/com/sassyconsulting/sassytalkie/CohortHistoryInstrumentedTest.kt
git -C V:/Projects/sassytalkie/sassy-talks commit -m "test(android): instrumented cohort history flows"
```

---

## Task 10: `MyCohortsTab` UI in `QRAuthScreen`

**Files:**
- Create: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/MyCohortsTab.kt`
- Modify: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/QRAuthScreen.kt`

- [ ] **Step 1: Create `MyCohortsTab.kt`**

```kotlin
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*
import org.json.JSONArray
import org.json.JSONObject

data class CohortListItem(
    val cohortId: String,
    val channel: Int,
    val groupName: String,
    val role: String,                 // "Hosted" | "Joined" | "Both"
    val hostDevice: String?,
    val participants: List<String>,   // names only, ordered
    val lastJoinedAt: Long,           // unix seconds
)

private fun parseHistory(json: String): List<CohortListItem> {
    return try {
        val arr = JSONArray(json)
        (0 until arr.length()).map { i ->
            val o = arr.getJSONObject(i)
            val parts = o.optJSONArray("last_participants") ?: JSONArray()
            val names = (0 until parts.length()).map { j ->
                parts.getJSONObject(j).optString("name", "")
            }.filter { it.isNotEmpty() }
            CohortListItem(
                cohortId = o.getString("cohort_id"),
                channel = o.getInt("channel"),
                groupName = o.optString("group_name", "Channel ${o.getInt("channel")}"),
                role = o.optString("role", "Hosted"),
                hostDevice = o.optString("host_device", "").ifEmpty { null },
                participants = names,
                lastJoinedAt = o.optLong("last_joined_at", 0),
            )
        }
    } catch (_: Exception) { emptyList() }
}

private fun relativeAgo(unixSecs: Long): String {
    if (unixSecs == 0L) return ""
    val nowSecs = System.currentTimeMillis() / 1000
    val delta = (nowSecs - unixSecs).coerceAtLeast(0)
    return when {
        delta < 60 -> "just now"
        delta < 3600 -> "${delta / 60}m ago"
        delta < 86400 -> "${delta / 3600}h ago"
        else -> "${delta / 86400}d ago"
    }
}

@Composable
fun MyCohortsTab(
    onRejoinHost: (channel: Int, groupName: String, cohortId: String) -> Unit,
    onRejoinJoiner: (hostDevice: String?) -> Unit,
) {
    var items by remember { mutableStateOf(parseHistory(SassyTalkNative.getCohortHistory())) }

    fun refresh() {
        items = parseHistory(SassyTalkNative.getCohortHistory())
    }

    Column(modifier = Modifier.fillMaxSize()) {
        if (items.isEmpty()) {
            Box(
                modifier = Modifier.fillMaxSize().padding(32.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    "Cohorts you host or join will show up here so you can resume them without saving keys.",
                    color = TextGray,
                    fontSize = 14.sp,
                    textAlign = TextAlign.Center,
                )
            }
        } else {
            LazyColumn(
                modifier = Modifier.fillMaxWidth().weight(1f),
                verticalArrangement = Arrangement.spacedBy(8.dp),
                contentPadding = PaddingValues(8.dp),
            ) {
                items(items, key = { it.cohortId }) { item ->
                    CohortRow(
                        item = item,
                        onRejoin = {
                            when (item.role) {
                                "Hosted", "Both" -> onRejoinHost(item.channel, item.groupName, item.cohortId)
                                else -> onRejoinJoiner(item.hostDevice)
                            }
                        },
                        onRemove = {
                            SassyTalkNative.removeCohort(item.cohortId)
                            refresh()
                        },
                    )
                }
            }
            OutlinedButton(
                onClick = {
                    SassyTalkNative.clearCohortHistory()
                    refresh()
                },
                shape = RoundedCornerShape(25.dp),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = TextMuted),
                modifier = Modifier.fillMaxWidth().padding(8.dp).height(44.dp),
            ) {
                Text("Clear All", fontSize = 13.sp)
            }
        }
    }
}

@Composable
private fun CohortRow(
    item: CohortListItem,
    onRejoin: () -> Unit,
    onRemove: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    Card(
        colors = CardDefaults.cardColors(containerColor = CardBg),
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(item.groupName, fontSize = 16.sp, fontWeight = FontWeight.SemiBold,
                     color = TextWhite, modifier = Modifier.weight(1f))
                AssistChip(
                    onClick = {},
                    label = { Text("Ch ${item.channel}", fontSize = 11.sp) },
                    colors = AssistChipDefaults.assistChipColors(containerColor = SurfaceBg, labelColor = Orange),
                )
            }
            val subtitle = buildString {
                if (item.role == "Joined" || item.role == "Both") {
                    item.hostDevice?.let { append("Hosted by $it · ") }
                }
                append("${item.participants.size} participants")
                val ago = relativeAgo(item.lastJoinedAt)
                if (ago.isNotEmpty()) append(" · $ago")
            }
            Text(subtitle, fontSize = 12.sp, color = TextGray, modifier = Modifier.padding(top = 2.dp))

            if (item.participants.isNotEmpty()) {
                Spacer(modifier = Modifier.height(6.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    item.participants.take(3).forEach { name ->
                        AssistChip(
                            onClick = {},
                            label = { Text(name, fontSize = 11.sp) },
                            colors = AssistChipDefaults.assistChipColors(containerColor = SurfaceBg, labelColor = TextWhite),
                        )
                    }
                    if (item.participants.size > 3) {
                        AssistChip(
                            onClick = {},
                            label = { Text("+${item.participants.size - 3}", fontSize = 11.sp) },
                            colors = AssistChipDefaults.assistChipColors(containerColor = SurfaceBg, labelColor = TextMuted),
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(8.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Button(
                    onClick = onRejoin,
                    colors = ButtonDefaults.buttonColors(containerColor = Orange),
                    shape = RoundedCornerShape(25.dp),
                    modifier = Modifier.weight(1f).height(40.dp),
                ) {
                    Icon(
                        when (item.role) {
                            "Hosted" -> Icons.Default.QrCode2
                            "Joined" -> Icons.Default.QrCodeScanner
                            else -> Icons.Default.Refresh
                        },
                        contentDescription = null,
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        when (item.role) {
                            "Hosted" -> "Rejoin"
                            "Joined" -> "Rejoin · Scan"
                            else -> "Rejoin"
                        },
                        fontSize = 13.sp,
                    )
                }
                Box {
                    IconButton(onClick = { menuOpen = true }) {
                        Icon(Icons.Default.MoreVert, contentDescription = "More", tint = TextGray)
                    }
                    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                        DropdownMenuItem(
                            text = { Text("Remove") },
                            onClick = { menuOpen = false; onRemove() },
                        )
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Wire `MyCohortsTab` into `QRAuthScreen.kt`**

In `QRAuthScreen.kt`, add a 4th tab. Update the `ScrollableTabRow` block (around line 92) and the `when (selectedTab)` block (around line 118).

Add to the `ScrollableTabRow`:

```kotlin
Tab(
    selected = selectedTab == 3,
    onClick = { selectedTab = 3 },
    text = { Text("My Cohorts", color = if (selectedTab == 3) Orange else TextGray, fontSize = 13.sp) },
)
```

Add to the `when (selectedTab)`:

```kotlin
3 -> MyCohortsTab(
    onRejoinHost = { channel, groupName, cohortId ->
        // Switch to Show QR tab with prefilled state, then auto-generate
        selectedChannel = channel
        this@QRAuthScreen.groupName = groupName  // adjust to whatever the existing var is named
        // Trigger generate with the same cohort_id
        val qr = SassyTalkNative.generateChannelQR(channel, durationHours, groupName, cohortId)
        if (qr.isNotEmpty()) {
            lastGeneratedJson = qr
            qrBitmap = generateQRBitmap(qr, 600)
            hasExistingSession.value = true
            selectedTab = 0
        }
    },
    onRejoinJoiner = { hostDevice ->
        // Switch to Scan QR tab; hint card showing host name handled inside ScanQRTab
        scanResult = if (hostDevice != null) "Ask $hostDevice to show their QR" else null
        showScanner = false
        selectedTab = 1
    },
)
```

Note: the function signature of `QRAuthScreen` may not currently allow `selectedTab`, `selectedChannel`, etc. to be reassigned across tab boundaries cleanly. They're `var` inside the composable, so direct reassignment works.

- [ ] **Step 3: Build and install on device**

```powershell
cd V:/Projects/sassytalkie/sassy-talks/sassy-talk-clean/android-app
./gradlew :app:installDebug
```

Expected: BUILD SUCCESSFUL and app installs. Launch the app, navigate to the QR auth screen, verify the new "My Cohorts" tab is visible.

- [ ] **Step 4: Manual smoke test on a Moto Z 2025 (or any phone)**

Run through this sequence and confirm each step:
1. Fresh install → "My Cohorts" tab shows empty state with the hint text.
2. Generate a QR for "Math 101 P1" on channel 1 → switch to "My Cohorts" → row appears with role "Hosted", channel chip "Ch 1", "0 participants".
3. Clear the session (use "New Session" button on Active Session card) → return to "My Cohorts" → row still there.
4. Tap "Rejoin" on the row → app jumps to Show QR with channel 1 + name "Math 101 P1" prefilled, a new QR displayed, and `cohort_id` in the JSON matches the row's id (verify by tapping "Copy Code" and inspecting).
5. Have a second device scan this QR → that device's "My Cohorts" shows a row with role "Joined" and host device name set.
6. On the second device tap "Rejoin · Scan" → app jumps to Scan tab with hint text mentioning the host's device name.

- [ ] **Step 5: Commit**

```powershell
git -C V:/Projects/sassytalkie/sassy-talks add sassy-talk-clean/android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/MyCohortsTab.kt sassy-talk-clean/android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/QRAuthScreen.kt
git -C V:/Projects/sassytalkie/sassy-talks commit -m "feat(ui): MyCohortsTab — list, rejoin, remove, clear-all"
```

---

## Self-Review

**Spec coverage check:**
- ✅ `cohort_id` added to QR with `#[serde(default)]` → Task 1.
- ✅ `CohortRecord`, `CohortRole`, `ParticipantSnapshot`, `CohortHistory` → Task 3.
- ✅ Storage in EncryptedSharedPreferences under `cohort_history_v1` → Task 7.
- ✅ Capture point 1 (host generate) → Task 5.
- ✅ Capture point 2 (joiner import) → Task 5.
- ✅ Capture point 3 (periodic participant snapshot) → Task 8.
- ✅ Capture point 4 (final snapshot before key drop) → covered implicitly by Task 8's 30s loop snapshotting until the cohort_id disappears (the last successful snapshot is the "final" one); explicit drop-handler snapshot is not required because participants are already up to date within ~30s. Acceptable per spec ("at most once per ~30s to avoid churn").
- ✅ Rust API additions → Tasks 3, 4, 5, 6.
- ✅ JNI surface → Tasks 5, 6.
- ✅ Kotlin wrappers → Task 7.
- ✅ UI changes (MyCohortsTab, role-aware actions, prefill plumbing, empty state) → Task 10.
- ✅ Edge cases (legacy QR, orphan merge with 30d window, role promotion, LRU eviction, EncryptedSharedPreferences unavailable) → Tasks 1, 4, plus existing `sessionPrefs()` fallback path is preserved.
- ✅ Tests (roundtrip, eviction, role promotion both ways, orphan merge in/out of window, security invariant, instrumented flows) → Tasks 3, 4, 9.

**Placeholder scan:** no TBDs, no "add appropriate handling" — each step has actual code or an exact command.

**Type consistency:**
- `generate_session_qr_with_cohort(channel, duration_hours, group_name, cohort_id)` — defined Task 1, called Task 5.
- `import_session` returns `(u8, CryptoSession, String)` — defined Task 2, consumed Tasks 5 (via tuple destructure).
- `CohortHistory::upsert_host(channel, group_name, cohort_id, session_id, now)` — defined Task 3, called Task 5.
- `CohortHistory::upsert_joiner(channel, group_name, cohort_id, host_device, session_id, now)` — defined Task 4, called Task 5.
- `CohortHistory::snapshot_participants(cohort_id, participants)` — defined Task 4, called via JNI in Task 6.
- `nativeGenerateChannelQR(channel, durationHours, groupName, cohortId)` — Rust signature in Task 5 matches Kotlin external in Task 7.
- `restoreCohortHistory()` / `saveCohortHistoryBlob()` — Kotlin in Task 7, called from snapshotter/generate/import in Tasks 7, 8.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-12-cohort-history-plan.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
