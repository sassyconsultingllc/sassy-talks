//! Cohort History — non-sensitive per-cohort metadata that survives key TTL.
//!
//! Stores only: cohort_id, channel, group_name, role, host_device (for joined),
//! participant name+id snapshots, and timestamps. AES keys are NEVER persisted
//! here — see security-invariant test (added in Task 4).

use log::info;
use serde::{Deserialize, Serialize};

pub const DEFAULT_HISTORY_CAP: usize = 50;

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

    /// Upsert a record as `Hosted`. If `cohort_id` is `Some(non-empty)`, reuses it;
    /// otherwise looks for an existing Hosted/Both record matching (channel, group_name)
    /// and reuses its cohort_id; otherwise mints a fresh UUID. Returns the resolved cohort_id.
    ///
    /// Role promotion (Joined → Both when host action lands on a previously-joined cohort)
    /// is handled in Task 4 alongside `upsert_joiner`.
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
