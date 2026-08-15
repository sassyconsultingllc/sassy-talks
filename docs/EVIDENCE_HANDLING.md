<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-->
# Technical audit export — evidence-handling procedure

**This file is a technical audit export — not a legal chain of custody / not court-certified evidence.**

It is **not** a discovery production, **not** a public-records package, and **not**
a court-certified exhibit. Counsel and agency policy must add whatever those
processes require.

## What the file proves technically

When exported from Settings → Technical audit (or iOS/desktop equivalent), the
JSON (`format: sassytalkie-technical-audit-v1`) contains:

- UTC timestamps (`ts_utc`)
- App id, app version, and install-id
- Bounded, redacted event list (max 500 events)
- Per-record SHA-256 over `previous|ts|event|detail`
- `head_hash` / `first_previous_hash` and `chain_valid`
- `manifest_hash` of the package body
- Optional device-bound HMAC (`HmacSHA256-AndroidKeyStore`) when Android Keystore
  can create the audit key; otherwise `signature_alg: none`

Events include TX start/stop/fail, SOS raise/clear, hybrid rekey, control
authentication failures, enrollment accept/reject, and session wipe.

Redaction strips likely secrets (long hex, `psk=` / `key=` / `token=` prefixes).

## What it does **not** prove

- Who physically held the device
- That the clock was correct (device clock, not a trusted time source)
- That the APK was unmodified relative to a lab hash (use your own signing/attestation)
- Continuity of custody after the file left the device
- Completeness if the ring buffer wrapped (oldest events dropped)
- Any legal element of authenticity, hearsay, or business-records foundation

## What counsel / agency policy must add

1. Seizure and bagging procedure, photographer, and custodian log.
2. Hash of the export file recorded on a separate evidence sheet (SHA-256 of the
   bytes as stored).
3. Transfer receipts, access log, and retention/destruction schedule.
4. Legal hold vs the app’s 500-event cap.
5. Any public-records or discovery protocol — **this app makes no such claim**.
6. Expert declaration if offered in court; this README is not that declaration.

## Retention and wipe

- Local store is bounded (500 events). Idle timeout or Settings “Clear session
  keys” / MDM `force_session_wipe` clears **keys**; export first if you need the
  technical log.
- Device-level wipe is MDM/EMM (DPC), not this application.
