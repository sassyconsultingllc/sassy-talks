// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! TLS SPKI pins for the Cloudflare relay host.
//!
//! Cloudflare rotates the **leaf** about every 90 days, so a leaf pin would
//! brick clients. Cloudflare also chooses the CA for Universal SSL itself, and
//! it issues from Let's Encrypt, Google Trust Services and SSL.com; a renewal
//! can arrive from any of them. Until 2026-10-22 the set held only the GTS
//! WE1/WE2/WR1/WR2 intermediates, so a renewal from another CA would have cut
//! off every client, fail-closed, at once.
//!
//! The set therefore pins the **root keys** of all three CAs, plus the GTS
//! intermediates already shipped. A root key matches whether the chain ends in
//! the self-signed root or a cross-signed copy of it (same key), and roots
//! outlive intermediate rotations by a decade. Every platform matches against
//! the verified chain including its trust anchor: OkHttp's cleaned chain,
//! iOS's evaluated SecTrust, and desktop's webpki verified path.
//!
//! Production default is **on** because the set has backups
//! (`pins_complete()`). MDM `require_tls_pinning=false` or
//! `SASSYTALKIE_TLS_PINNING=0` disables (platform TLS). When enabled, mismatch
//! is fail-closed. See `docs/TLS_PINNING.md` for sources and rotation.

pub const RELAY_HOST: &str = "relay.sassyconsultingllc.com";

/// True when the configured pin-set has a primary plus at least one backup.
pub fn pins_complete() -> bool {
    SPKI_PINS_SHA256_B64.len() >= 2
}

/// Production default: pin when backups are present. Not a leaf-only pin.
pub const PINNING_DEFAULT: bool = true;

/// SHA-256 of SubjectPublicKeyInfo, base64 (OkHttp `sha256/` form without prefix).
///
/// Must stay in lockstep with Android `RelayTlsPins.kt` (a test below checks).
///
/// | Cert | Role | Key | Not after (UTC) |
/// |------|------|-----|-----------------|
/// | GTS WE1 | ECDSA intermediate (current Cloudflare chain) | P-256 | 2029-02-20 |
/// | GTS WE2 | ECDSA intermediate backup | P-256 | 2029-02-20 |
/// | GTS WR1 | RSA intermediate backup | RSA-2048 | 2029-02-20 |
/// | GTS WR2 | RSA intermediate backup | RSA-2048 | 2029-02-20 |
/// | GTS Root R1 | root | RSA-4096 | 2036-06-22 |
/// | GTS Root R2 | root | RSA-4096 | 2036-06-22 |
/// | GTS Root R3 | root | P-384 | 2036-06-22 |
/// | GTS Root R4 | root (current anchor, also served cross-signed by GlobalSign) | P-384 | 2036-06-22 |
/// | ISRG Root X1 | Let's Encrypt root | RSA-4096 | 2035-06-04 |
/// | ISRG Root X2 | Let's Encrypt root | P-384 | 2040-09-17 |
/// | ISRG Root YE | Let's Encrypt Gen Y root (cross-signed by X2) | P-384 | 2045-09-02 |
/// | ISRG Root YR | Let's Encrypt Gen Y root (cross-signed by X1) | RSA-4096 | 2045-09-02 |
/// | SSL.com TLS RSA Root CA 2022 | root (cross-signed by Certum) | RSA-4096 | 2046-08-19 |
/// | SSL.com TLS ECC Root CA 2022 | root (cross-signed by Certum) | P-384 | 2046-08-19 |
pub const SPKI_PINS_SHA256_B64: &[&str] = &[
    "kIdp6NNEd8wsugYyyIYFsi1ylMCED3hZbSR8ZFsa/A4=", // GTS WE1
    "vh78KSg1Ry4NaqGDV10w/cTb9VH3BQUZoCWNa93W/EY=", // GTS WE2
    "yDu9og255NN5GEf+Bwa9rTrqFQ0EydZ0r1FCh9TdAW4=", // GTS WR1
    "YPtHaftLw6/0vnc2BnNKGF54xiCA28WFcccjkA4ypCM=", // GTS WR2
    "hxqRlPTu1bMS/0DITB1SSu0vd4u/8l8TjPgfaAp63Gc=", // GTS Root R1
    "Vfd95BwDeSQo+NUYxVEEIlvkOlWY2SalKK1lPhzOx78=", // GTS Root R2
    "QXnt2YHvdHR3tJYmQIr0Paosp6t/nggsEGD4QJZ3Q0g=", // GTS Root R3
    "mEflZT5enoR1FuXLgYYGqnVEoZvmf9c2bVBpiOjYQ0c=", // GTS Root R4
    "C5+lpZ7tcVwmwQIMcRtPbsQtWLABXhQzejna0wHFr8M=", // ISRG Root X1
    "diGVwiVYbubAI3RW4hB9xU8e/CH2GnkuvVFZE8zmgzI=", // ISRG Root X2
    "sCkq5UWXjg+7mKu9lMhhYF5bGLsy7VI/UNW3tccdR7w=", // ISRG Root YE
    "fk6IOKit1ild5647BH06ujSIq5XbCgqlbYl6ANhhi88=", // ISRG Root YR
    "K89VOmb1cJAN3TK6bf4ezAbJGC1mLcG2Dh97dnwr3VQ=", // SSL.com TLS RSA Root CA 2022
    "G/ANXI8TwJTdF+AFBM8IiIUPEv0Gf6H5LA/b9guG4yE=", // SSL.com TLS ECC Root CA 2022
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
    fn pin_set_is_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for p in SPKI_PINS_SHA256_B64 {
            assert!(seen.insert(*p), "duplicate pin {p}");
            assert_eq!(p.len(), 44, "standard SHA-256 base64 length");
        }
    }

    #[test]
    fn every_ca_cloudflare_issues_from_is_pinned() {
        // Regression: only GTS intermediates were pinned, so a Universal SSL
        // renewal from Let's Encrypt or SSL.com would have failed closed on
        // every client. Each CA's root key must be present.
        for (ca, root) in [
            ("GTS Root R4 (live anchor)", "mEflZT5enoR1FuXLgYYGqnVEoZvmf9c2bVBpiOjYQ0c="),
            ("ISRG Root X1", "C5+lpZ7tcVwmwQIMcRtPbsQtWLABXhQzejna0wHFr8M="),
            ("ISRG Root X2", "diGVwiVYbubAI3RW4hB9xU8e/CH2GnkuvVFZE8zmgzI="),
            ("SSL.com RSA 2022", "K89VOmb1cJAN3TK6bf4ezAbJGC1mLcG2Dh97dnwr3VQ="),
            ("SSL.com ECC 2022", "G/ANXI8TwJTdF+AFBM8IiIUPEv0Gf6H5LA/b9guG4yE="),
        ] {
            assert!(SPKI_PINS_SHA256_B64.contains(&root), "{ca} not pinned");
        }
    }

    #[test]
    fn android_pin_list_is_in_lockstep() {
        let kt = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../android-app/app/src/main/java/com/sassyconsulting/sassytalkie/RelayTlsPins.kt"
        ))
        .expect("RelayTlsPins.kt");
        let android: Vec<&str> = kt
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"'))
            .filter_map(|l| l.split('"').next())
            .filter(|p| p.len() == 44 && p.ends_with('='))
            .collect();
        assert_eq!(android, SPKI_PINS_SHA256_B64, "Android pins differ from core");
    }
}
