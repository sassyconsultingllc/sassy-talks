// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! Tamper-evident hash-chained technical audit. This is not a legal chain of
//! custody and is not court-certified evidence.

use sha2::{Digest, Sha256};

pub const FORMAT: &str = "sassytalkie-technical-audit-v1";
pub const DISCLAIMER: &str =
    "technical audit export — not a legal chain of custody / not court-certified evidence";
pub const MAX_EVENTS: usize = 500;
pub const MAX_DETAIL_CHARS: usize = 256;
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub ts_unix_ms: u64,
    pub event: String,
    pub detail: String,
    pub previous: String,
    pub hash: String,
}

#[derive(Debug, Clone)]
pub struct AuditChain {
    events: Vec<AuditEvent>,
    max_events: usize,
}

impl Default for AuditChain {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            max_events: MAX_EVENTS,
        }
    }
}

impl AuditChain {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Vec::new(),
            max_events: max_events.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    pub fn append(&mut self, ts_unix_ms: u64, event: &str, detail: &str) -> AuditEvent {
        let previous = self
            .events
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_default();
        let normalized = redact_detail(detail);
        let hash = event_hash(&previous, ts_unix_ms, event, &normalized);
        let record = AuditEvent {
            ts_unix_ms,
            event: event.to_string(),
            detail: normalized,
            previous,
            hash,
        };
        self.events.push(record.clone());
        if self.events.len() > self.max_events {
            let drop = self.events.len() - self.max_events;
            self.events.drain(0..drop);
        }
        record
    }

    pub fn verify(&self) -> bool {
        verify_events(&self.events)
    }

    pub fn retain_since(&mut self, oldest_unix_ms: u64) {
        self.events.retain(|e| e.ts_unix_ms >= oldest_unix_ms);
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// JSON technical-audit package. Not a legal chain of custody.
    pub fn export_package(&self, app_id: &str, app_version: &str, install_id: &str) -> String {
        export_package(self, app_id, app_version, install_id)
    }
}

pub fn export_package(chain: &AuditChain, app_id: &str, app_version: &str, install_id: &str) -> String {
    let events: Vec<serde_json::Value> = chain
        .events()
        .iter()
        .map(|e| {
            serde_json::json!({
                "ts": e.ts_unix_ms,
                "ts_utc": unix_ms_to_utc(e.ts_unix_ms),
                "event": e.event,
                "detail": e.detail,
                "previous": e.previous,
                "hash": e.hash,
            })
        })
        .collect();
    let head = chain
        .events()
        .last()
        .map(|e| e.hash.clone())
        .unwrap_or_default();
    let first_previous = chain
        .events()
        .first()
        .map(|e| e.previous.clone())
        .unwrap_or_default();
    let mut body = serde_json::json!({
        "format": FORMAT,
        "disclaimer": DISCLAIMER,
        "hash_algorithm": "SHA-256",
        "exported_at_utc": unix_ms_to_utc(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ),
        "app_id": app_id,
        "app_version": app_version,
        "install_id": install_id,
        "event_count": chain.len(),
        "first_previous_hash": first_previous,
        "head_hash": head,
        "chain_valid": chain.verify(),
        "retention_max_events": MAX_EVENTS,
        "events": events,
    });
    let canonical = body.to_string();
    let payload_hash = sha256_hex(canonical.as_bytes());
    body["manifest_hash"] = serde_json::Value::String(payload_hash);
    body["manifest_signature"] = serde_json::Value::Null;
    body["signature_alg"] = serde_json::Value::String("none".into());
    body.to_string()
}

pub fn event_hash(previous: &str, ts_unix_ms: u64, event: &str, detail: &str) -> String {
    hex_lower(&Sha256::digest(format!("{previous}|{ts_unix_ms}|{event}|{detail}").as_bytes()))
}

pub fn verify_events(events: &[AuditEvent]) -> bool {
    let mut previous = events
        .first()
        .map(|e| e.previous.as_str())
        .unwrap_or("")
        .to_string();
    for item in events {
        if item.previous != previous {
            return false;
        }
        let expected = event_hash(&previous, item.ts_unix_ms, &item.event, &item.detail);
        if item.hash != expected {
            return false;
        }
        previous = expected;
    }
    true
}

/// Strip likely secrets from operator-facing audit text.
pub fn redact_detail(detail: &str) -> String {
    let mut out = String::new();
    for token in detail.split_whitespace() {
        let redacted = if looks_like_secret(token) {
            "[redacted]"
        } else {
            token
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(redacted);
        if out.len() >= MAX_DETAIL_CHARS {
            out.truncate(MAX_DETAIL_CHARS);
            break;
        }
    }
    if out.is_empty() {
        detail.chars().take(MAX_DETAIL_CHARS).collect()
    } else {
        out
    }
}

fn looks_like_secret(token: &str) -> bool {
    let t = token.trim_matches(|c| c == ',' || c == ';' || c == '=');
    if t.len() >= 32 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    if t.len() >= 24 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        && (t.contains('+') || t.contains('/') || t.ends_with('='))
    {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    lower.starts_with("psk=") || lower.starts_with("key=") || lower.starts_with("token=")
}

pub fn unix_ms_to_utc(ts_unix_ms: u64) -> String {
    let secs = ts_unix_ms / 1000;
    let millis = ts_unix_ms % 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hh = rem / 3_600;
    let mm = (rem % 3_600) / 60;
    let ss = rem % 60;
    let (y, m, d) = civil_from_unix_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z")
}

/// Howard Hinnant civil-from-days. `z` is days since 1970-01-01 UTC.
fn civil_from_unix_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_detects_tamper() {
        let mut chain = AuditChain::default();
        chain.append(1_700_000_000_000, "tx_requested", "ok");
        chain.append(1_700_000_000_100, "tx_stopped", "reason=user");
        assert!(chain.verify());
        chain.events[0].detail = "tampered".into();
        assert!(!chain.verify());
    }

    #[test]
    fn redacts_hex_and_psk_tokens() {
        let d = redact_detail("reason=ok psk=0123456789abcdef0123456789abcdef key=AAAA");
        assert!(d.contains("[redacted]"));
        assert!(!d.contains("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn disclaimer_is_explicit() {
        assert!(DISCLAIMER.contains("not a legal chain of custody"));
        assert!(DISCLAIMER.contains("not court-certified evidence"));
    }

    #[test]
    fn utc_format_is_zulu() {
        let s = unix_ms_to_utc(0);
        assert!(s.ends_with('Z'));
        assert_eq!(s, "1970-01-01T00:00:00.000Z");
        assert_eq!(unix_ms_to_utc(1_700_000_000_000), "2023-11-14T22:13:20.000Z");
    }

    #[test]
    fn export_package_carries_disclaimer_and_hashes() {
        let mut chain = AuditChain::default();
        chain.append(1_700_000_000_000, "enrollment", "ok");
        let json = chain.export_package("app", "1.0", "install-1");
        assert!(json.contains(DISCLAIMER));
        assert!(json.contains("manifest_hash"));
        assert!(json.contains("not court-certified evidence"));
    }
}
