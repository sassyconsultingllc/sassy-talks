// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
//! Desktop TLS pin-set for the Cloudflare relay. Uses GTS intermediate SPKI
//! pins from `sassytalkie_core::tls_pins` (primary + backups). Enabled by
//! default when the pin-set is complete; `SASSYTALKIE_TLS_PINNING=0` disables.
//! Mismatch is fail-closed. Rotation: `docs/TLS_PINNING.md`.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme};
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

pub fn chain_matches_pins(certs: &[&[u8]]) -> bool {
    let presented: Vec<String> = certs.iter().filter_map(|c| spki_sha256_b64(c)).collect();
    let refs: Vec<&str> = presented.iter().map(|s| s.as_str()).collect();
    pin_match(&refs, SPKI_PINS_SHA256_B64)
}

#[derive(Debug)]
struct PinningVerifier {
    inner: Arc<WebPkiServerVerifier>,
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
        let mut ders: Vec<&[u8]> = Vec::with_capacity(1 + intermediates.len());
        ders.push(end_entity.as_ref());
        for c in intermediates {
            ders.push(c.as_ref());
        }
        if chain_matches_pins(&ders) {
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
    let inner = WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|e| format!("tls verifier: {e}"))?;
    let pinning = pinning_active();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let verifier = Arc::new(PinningVerifier { inner, pinning });
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
        assert!(!chain_matches_pins(&[]));
        assert!(spki_der_from_cert(&[0x30, 0x00]).is_none());
    }
}
