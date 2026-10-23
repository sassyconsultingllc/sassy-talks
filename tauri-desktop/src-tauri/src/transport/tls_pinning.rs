// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! Desktop TLS pin-set for the Cloudflare relay. Uses the SPKI pins from
//! `sassytalkie_core::tls_pins` (CA roots + GTS intermediates). Enabled by
//! default when the pin-set is complete; `SASSYTALKIE_TLS_PINNING=0` disables.
//! Mismatch is fail-closed. Pins are matched against the path webpki verified,
//! including its trust anchor. Rotation: `docs/TLS_PINNING.md`.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, TrustAnchor, UnixTime};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};

use sassytalkie_core::tls_pins::{
    pin_match, pinning_enabled_from_env, pins_complete, RELAY_HOST, SPKI_PINS_SHA256_B64,
};

pub fn pinning_active() -> bool {
    pinning_enabled_from_env() && pins_complete()
}

pub fn spki_sha256_b64(der: &[u8]) -> Option<String> {
    let spki = spki_der_from_cert(der)?;
    let hash = Sha256::digest(spki);
    Some(base64::engine::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        hash,
    ))
}

/// SHA-256 of SubjectPublicKeyInfo (RFC 7469) extracted from an X.509 DER cert.
fn spki_der_from_cert(der: &[u8]) -> Option<&[u8]> {
    let (cert_body, _) = der_take_seq(der)?;
    let (tbs, _) = der_take_seq(cert_body)?;
    let mut rest = tbs;
    if rest.first().copied() == Some(0xA0) {
        rest = der_skip(rest)?;
    }
    rest = der_skip(rest)?; // serial
    rest = der_skip(rest)?; // signature
    rest = der_skip(rest)?; // issuer
    rest = der_skip(rest)?; // validity
    rest = der_skip(rest)?; // subject
    let (spki, _) = der_take_full(rest)?;
    Some(spki)
}

fn der_len(input: &[u8]) -> Option<(usize, usize)> {
    if input.is_empty() {
        return None;
    }
    let b = input[0];
    if b < 0x80 {
        return Some((b as usize, 1));
    }
    let n = (b & 0x7F) as usize;
    if n == 0 || n > 4 || input.len() < 1 + n {
        return None;
    }
    let mut len = 0usize;
    for i in 1..=n {
        len = (len << 8) | input[i] as usize;
    }
    Some((len, 1 + n))
}

fn der_take_seq(input: &[u8]) -> Option<(&[u8], &[u8])> {
    if input.first().copied() != Some(0x30) {
        return None;
    }
    let (len, hdr) = der_len(&input[1..])?;
    let start = 1 + hdr;
    if start + len > input.len() {
        return None;
    }
    Some((&input[start..start + len], &input[start + len..]))
}

fn der_take_full(input: &[u8]) -> Option<(&[u8], &[u8])> {
    if input.is_empty() {
        return None;
    }
    let (len, hdr) = der_len(&input[1..])?;
    let total = 1 + hdr + len;
    if total > input.len() {
        return None;
    }
    Some((&input[..total], &input[total..]))
}

fn der_skip(input: &[u8]) -> Option<&[u8]> {
    der_take_full(input).map(|(_, rest)| rest)
}

/// SPKI pins of the chain webpki actually verifies: its intermediates and its
/// trust anchor. Certificates the server sends that the path does not use are
/// ignored. `None` when no path verifies.
///
/// The old check hashed every certificate the server sent. An attacker with a
/// certificate for the relay host from any trusted CA could append the public
/// GTS WE1 certificate as an unused extra and pass. Matching the anchor is
/// also what makes the root pins work: servers don't send their root, and
/// webpki's verified path carries it.
pub fn verified_path_pins(
    roots: &RootCertStore,
    end_entity: &CertificateDer<'_>,
    intermediates: &[CertificateDer<'_>],
    now: UnixTime,
) -> Option<Vec<String>> {
    let ee = webpki::EndEntityCert::try_from(end_entity).ok()?;
    let algs = rustls::crypto::ring::default_provider().signature_verification_algorithms;
    let path = ee
        .verify_for_usage(
            algs.all,
            &roots.roots,
            intermediates,
            now,
            webpki::KeyUsage::server_auth(),
            None,
            None,
        )
        .ok()?;
    let mut pins: Vec<String> = path
        .intermediate_certificates()
        .filter_map(|c| spki_sha256_b64(c.der().as_ref()))
        .collect();
    pins.push(anchor_spki_sha256_b64(path.anchor()));
    Some(pins)
}

/// webpki stores a trust anchor's subjectPublicKeyInfo without its outer
/// SEQUENCE header, so the pin hashes the re-wrapped SEQUENCE.
fn anchor_spki_sha256_b64(anchor: &TrustAnchor<'_>) -> String {
    let body = anchor.subject_public_key_info.as_ref();
    let mut spki = Vec::with_capacity(body.len() + 4);
    spki.push(0x30);
    if body.len() < 0x80 {
        spki.push(body.len() as u8);
    } else if body.len() <= 0xFF {
        spki.extend_from_slice(&[0x81, body.len() as u8]);
    } else {
        spki.extend_from_slice(&[0x82, (body.len() >> 8) as u8, body.len() as u8]);
    }
    spki.extend_from_slice(body);
    base64::engine::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        Sha256::digest(&spki),
    )
}

#[derive(Debug)]
struct PinningVerifier {
    inner: Arc<WebPkiServerVerifier>,
    roots: Arc<RootCertStore>,
    pinning: bool,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        if !self.pinning {
            return Ok(ServerCertVerified::assertion());
        }
        let pinned = verified_path_pins(&self.roots, end_entity, intermediates, now)
            .map(|pins| {
                let refs: Vec<&str> = pins.iter().map(String::as_str).collect();
                pin_match(&refs, SPKI_PINS_SHA256_B64)
            })
            .unwrap_or(false);
        if pinned {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::General("tls pin mismatch (fail-closed)".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

pub fn client_config() -> Result<ClientConfig, String> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let roots = Arc::new(roots);
    let inner = WebPkiServerVerifier::builder(roots.clone())
        .build()
        .map_err(|e| format!("tls verifier: {e}"))?;
    let pinning = pinning_active();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let verifier = Arc::new(PinningVerifier {
        inner,
        roots,
        pinning,
    });
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(cfg)
}

pub fn relay_host() -> &'static str {
    RELAY_HOST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_enable_default_pinning() {
        assert!(pins_complete());
        assert!(SPKI_PINS_SHA256_B64.len() >= 2);
    }

    #[test]
    fn mismatch_fails_closed() {
        assert!(!pin_match(
            &["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="],
            SPKI_PINS_SHA256_B64
        ));
        assert!(spki_der_from_cert(&[0x30, 0x00]).is_none());
    }

    /// Fixtures captured 2026-10-22 (Unix 1792686144); verifying "as of" that
    /// moment keeps the tests valid after the certificates expire.
    const CAPTURED_AT: u64 = 1_792_686_144;
    const GTS_WE1: &str = "kIdp6NNEd8wsugYyyIYFsi1ylMCED3hZbSR8ZFsa/A4=";
    const GTS_ROOT_R4: &str = "mEflZT5enoR1FuXLgYYGqnVEoZvmf9c2bVBpiOjYQ0c=";

    fn store() -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        roots
    }

    fn at_capture() -> UnixTime {
        UnixTime::since_unix_epoch(std::time::Duration::from_secs(CAPTURED_AT))
    }

    fn matches(pins: &[String]) -> bool {
        let refs: Vec<&str> = pins.iter().map(String::as_str).collect();
        pin_match(&refs, SPKI_PINS_SHA256_B64)
    }

    #[test]
    fn live_relay_chain_matches_through_the_verified_path() {
        let leaf = CertificateDer::from(&include_bytes!("testdata/relay-leaf.der")[..]);
        let sent = [
            CertificateDer::from(&include_bytes!("testdata/relay-we1.der")[..]),
            CertificateDer::from(&include_bytes!("testdata/relay-r4-cross.der")[..]),
        ];
        let pins =
            verified_path_pins(&store(), &leaf, &sent, at_capture()).expect("relay chain verifies");
        assert!(pins.iter().any(|p| p == GTS_WE1), "{pins:?}");
        assert!(
            pins.iter().any(|p| p == GTS_ROOT_R4),
            "anchor missing: {pins:?}"
        );
        assert!(matches(&pins));
    }

    #[test]
    fn an_unused_pinned_cert_appended_to_another_cas_chain_does_not_pass() {
        // Regression (pin bypass): a valid chain from a CA that is not pinned
        // (DigiCert), with the public GTS WE1 certificate appended as an unused
        // extra. Hashing every presented certificate found WE1 and passed.
        let leaf = CertificateDer::from(&include_bytes!("testdata/digicert-leaf.der")[..]);
        let sent = [
            CertificateDer::from(&include_bytes!("testdata/digicert-ev-g2.der")[..]),
            CertificateDer::from(&include_bytes!("testdata/relay-we1.der")[..]),
        ];
        let presented: Vec<String> = std::iter::once(&leaf)
            .chain(sent.iter())
            .filter_map(|c| spki_sha256_b64(c.as_ref()))
            .collect();
        assert!(matches(&presented), "the old check passes this chain");
        let pins = verified_path_pins(&store(), &leaf, &sent, at_capture())
            .expect("DigiCert chain verifies");
        assert!(
            !pins.iter().any(|p| p == GTS_WE1),
            "WE1 is not on the verified path"
        );
        assert!(!matches(&pins), "verified path must not match: {pins:?}");
    }

    #[test]
    fn anchor_spki_rewrap_matches_the_published_root_pin() {
        let r4 = webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .find(|a| a.subject.as_ref().windows(11).any(|w| w == b"GTS Root R4"))
            .expect("GTS Root R4 in webpki-roots");
        assert_eq!(anchor_spki_sha256_b64(r4), GTS_ROOT_R4);
    }

    #[test]
    fn unverifiable_chain_yields_no_pins() {
        let leaf = CertificateDer::from(&include_bytes!("testdata/relay-leaf.der")[..]);
        assert!(verified_path_pins(&store(), &leaf, &[], at_capture()).is_none());
    }
}
