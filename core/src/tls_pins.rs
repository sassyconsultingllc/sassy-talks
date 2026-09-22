// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! TLS SPKI pins for the Cloudflare relay host.
//!
//! Cloudflare / Google Trust Services rotate the **leaf** about every 90 days;
//! pinning a single leaf would brick clients. The pin-set below is the
//! **intermediates** (GTS WE1/WE2 + WR1/WR2) captured from the live chain on
//! 2026-08-14 so a leaf rotation still matches. Pins are not-after 2029-02-20.
//!
//! Production default is **on** because this set includes backup intermediates
//! (`pins_complete()`). MDM `require_tls_pinning=false` or
//! `SASSYTALKIE_TLS_PINNING=0` disables (platform TLS). When enabled, mismatch
//! is fail-closed. Rotation: replace the pin-set, keep at least one backup,
//! ship, then retire the old intermediate after Google's overlap window.
//! See `docs/TLS_PINNING.md`.

pub const RELAY_HOST: &str = "relay.sassyconsultingllc.com";

/// True when the configured pin-set has a primary plus at least one backup.
pub fn pins_complete() -> bool {
    SPKI_PINS_SHA256_B64.len() >= 2
}

/// Production default: pin when backups are present. Not a leaf-only pin.
pub const PINNING_DEFAULT: bool = true;

/// SHA-256 of SubjectPublicKeyInfo, base64 (OkHttp `sha256/` form without prefix).
///
/// | Cert | Role | Not after (UTC) |
/// |------|------|-----------------|
/// | GTS WE1 | ECDSA intermediate (current Cloudflare) | 2029-02-20 |
/// | GTS WE2 | ECDSA intermediate backup | 2029-02-20 |
/// | GTS WR1 | RSA intermediate backup | 2029-02-20 |
/// | GTS WR2 | RSA intermediate backup | 2029-02-20 |
pub const SPKI_PINS_SHA256_B64: &[&str] = &[
    "kIdp6NNEd8wsugYyyIYFsi1ylMCED3hZbSR8ZFsa/A4=", // GTS WE1
    "vh78KSg1Ry4NaqGDV10w/cTb9VH3BQUZoCWNa93W/EY=", // GTS WE2
    "yDu9og255NN5GEf+Bwa9rTrqFQ0EydZ0r1FCh9TdAW4=", // GTS WR1
    "YPtHaftLw6/0vnc2BnNKGF54xiCA28WFcccjkA4ypCM=", // GTS WR2
];

/// True when any presented SPKI pin matches the configured set.
/// Empty presented or empty configured is a mismatch (fail-closed).
pub fn pin_match(presented_b64: &[&str], configured_b64: &[&str]) -> bool {
    if presented_b64.is_empty() || configured_b64.is_empty() {
        return false;
    }
    presented_b64.iter().any(|p| configured_b64.iter().any(|c| c == p))
}

pub fn pinning_enabled_from_env() -> bool {
    match std::env::var("SASSYTALKIE_TLS_PINNING") {
        Ok(v) => match v.as_str() {
            "0" | "false" | "FALSE" | "no" | "off" => false,
            "1" | "true" | "TRUE" | "yes" | "on" => true,
            _ => PINNING_DEFAULT && pins_complete(),
        },
        Err(_) => PINNING_DEFAULT && pins_complete(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinning_defaults_on_because_backups_exist() {
        assert!(pins_complete());
        assert!(PINNING_DEFAULT);
        assert!(PINNING_DEFAULT && pins_complete());
    }

    #[test]
    fn known_intermediate_matches() {
        assert!(pin_match(
            &[SPKI_PINS_SHA256_B64[0]],
            SPKI_PINS_SHA256_B64
        ));
    }

    #[test]
    fn mismatch_fails_closed() {
        assert!(!pin_match(
            &["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="],
            SPKI_PINS_SHA256_B64
        ));
        assert!(!pin_match(&[], SPKI_PINS_SHA256_B64));
        assert!(!pin_match(&[SPKI_PINS_SHA256_B64[0]], &[]));
    }

    #[test]
    fn pin_set_has_backup_intermediates() {
        assert_eq!(SPKI_PINS_SHA256_B64.len(), 4);
        let mut seen = std::collections::HashSet::new();
        for p in SPKI_PINS_SHA256_B64 {
            assert!(seen.insert(*p), "duplicate pin {p}");
            assert_eq!(p.len(), 44, "standard SHA-256 base64 length");
        }
    }
}
