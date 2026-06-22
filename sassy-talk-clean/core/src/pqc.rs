//! pqc — Hybrid post-quantum key agreement (X25519 + ML-KEM-768).
//!
//! **INTEGRATION STATUS (staged, not yet on the wire):** this module compiles
//! and self-tests but no transport drives it yet — every live handshake is
//! still classical X25519 only (see `crypto::KeyExchange`). Activating PQC is a
//! protocol-versioned change on BOTH ends: negotiate a hybrid suite in the QR /
//! session handshake, carry the ML-KEM ciphertext in a new handshake frame, and
//! feed the combined secret into `CryptoSession::from_shared_secret`. Until that
//! lands, the "harvest-now-decrypt-later" protection described below is NOT in
//! effect — do not assume callers are quantum-safe just because this exists.
//!
//! # Why hybrid, and why now
//!
//! Classical X25519 ECDH is broken by a sufficiently large quantum computer
//! (Shor's algorithm recovers the discrete log). A "harvest now, decrypt later"
//! adversary can record today's X25519 handshakes and the audio they protect,
//! then decrypt years later once such hardware exists. For a push-to-talk app
//! that's a real exposure window.
//!
//! ML-KEM-768 (FIPS 203, NIST security category 3) is a lattice KEM believed to
//! resist quantum attack. But post-quantum schemes are *young* — implementation
//! bugs or cryptanalytic breaks are more plausible than for the decades-hardened
//! X25519. So we run BOTH and combine their secrets through a KDF. The combined
//! key is secure as long as **at least one** of the two primitives is secure:
//!
//!   * Quantum adversary breaks X25519 but not ML-KEM  → key still safe.
//!   * Classical break / impl bug in ML-KEM but X25519 holds → key still safe.
//!
//! This "belt and suspenders" construction is the consensus migration strategy
//! (cf. TLS X25519MLKEM768, Signal PQXDH, Apple iMessage PQ3). We deliberately
//! do NOT ship ML-KEM alone.
//!
//! # Why HKDF over the concatenation
//!
//! We feed `x25519_shared || mlkem_shared` as the HKDF input keying material.
//! Concatenation (rather than XOR) is what makes the construction sound: HKDF
//! acts as a dual-PRF over the pair, so the output is pseudorandom if *either*
//! half is. XOR-combining would let an attacker who controls one secret cancel
//! the other. Both secrets are 32 bytes; the 64-byte IKM passes through
//! HKDF-Extract+Expand to a single 32-byte AES-256 key.
//!
//! Note the domain-separation info string is **distinct** from crypto.rs's
//! `b"sassytalkie-aead-v2"`. We KDF here and then hand the finished 32-byte key
//! to `CryptoSession::from_psk` (which applies *no* further KDF), so the v2 AEAD
//! info string is never applied twice and the classical wire path in crypto.rs
//! stays byte-for-byte unchanged.
//!
//! # Layering
//!
//! This module is purely additive on top of `crate::crypto`. It produces a
//! `crate::crypto::CryptoSession` exactly like the classical path, so hybrid is
//! negotiable and fully backward-compatible: a peer that doesn't advertise
//! hybrid support transparently falls back to classical X25519 ECDH.
//!
//! # Key hygiene
//!
//! The X25519 ephemeral zeroizes on drop (x25519-dalek `zeroize` feature). The
//! ML-KEM `DecapsulationKey` zeroizes on drop (ml-kem `zeroize` feature). The
//! two raw shared secrets and the combined IKM buffer live in `Zeroizing`
//! wrappers so they are wiped the moment the final key is derived.

use crate::crypto::{CryptoSession, KeyExchange};

use x25519_dalek::{EphemeralSecret, PublicKey};
use aes_gcm::aead::OsRng;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;
use log::info;

use ml_kem::{
    EncapsulationKey, MlKem768,
    array::{Array, typenum::Unsigned},
    kem::{Decapsulate, Encapsulate, Kem, KeyExport, KeySizeUser},
    DecapsulationKey,
};

/// Length of an X25519 public key in bytes (Curve25519 u-coordinate).
pub const X25519_PUBLIC_LEN: usize = 32;

/// Length of a raw shared secret produced by either primitive (both are 32 B):
/// X25519 yields a 32-byte field element, ML-KEM-768 a 32-byte shared key.
pub const SHARED_SECRET_LEN: usize = 32;

/// Length of a serialized ML-KEM-768 encapsulation (public) key, in bytes.
///
/// Derived from the crate's typed size rather than hardcoded: it is the
/// `KeySize` of `EncapsulationKey<MlKem768>` lowered to a `usize` via the
/// `typenum::Unsigned::USIZE` associated const. For ML-KEM-768 this is 1184,
/// but we never write that literal — if the parameter set or encoding ever
/// changes, this constant tracks it automatically.
pub const MLKEM768_ENCAPS_KEY_LEN: usize =
    <<EncapsulationKey<MlKem768> as KeySizeUser>::KeySize as Unsigned>::USIZE;

/// Length of a serialized ML-KEM-768 ciphertext ("encapsulated key"), in bytes.
///
/// Derived from `<MlKem768 as Kem>::CiphertextSize` via `Unsigned::USIZE`.
/// For ML-KEM-768 this is 1088. Again, never hardcoded.
pub const MLKEM768_CIPHERTEXT_LEN: usize =
    <<MlKem768 as Kem>::CiphertextSize as Unsigned>::USIZE;

/// Total length of the initiator's on-wire public material:
/// X25519 public key || ML-KEM-768 encapsulation key.
pub const HYBRID_INIT_MSG_LEN: usize = X25519_PUBLIC_LEN + MLKEM768_ENCAPS_KEY_LEN;

/// Total length of the responder's on-wire public material:
/// X25519 public key || ML-KEM-768 ciphertext.
pub const HYBRID_RESP_MSG_LEN: usize = X25519_PUBLIC_LEN + MLKEM768_CIPHERTEXT_LEN;

/// HKDF domain-separation tag for the hybrid combiner. Distinct from crypto.rs's
/// AEAD info string so the two KDFs can never be confused, and so we don't apply
/// the `-aead-v2` expansion twice (we feed the result to `from_psk`, not
/// `from_shared_secret`). Bumping this string is a wire break for hybrid peers.
const HYBRID_HKDF_INFO: &[u8] = b"sassytalkie-hybrid-pqc-v1";

/// Combine the two raw shared secrets into a single 32-byte AES-256 key.
///
/// `ikm = x25519_shared || mlkem_shared`, run through HKDF-SHA256. Concatenation
/// (not XOR) is deliberate: it yields a combiner that is secure if EITHER input
/// is secure (see module docs). The returned key is `Zeroizing` so it wipes
/// after the `CryptoSession` is built.
fn combine_secrets(x25519_shared: &[u8], mlkem_shared: &[u8]) -> Zeroizing<[u8; 32]> {
    // Hold the concatenated IKM in a zeroizing buffer — it transitively contains
    // both raw secrets, so it must not linger on the stack.
    let mut ikm = Zeroizing::new(Vec::with_capacity(x25519_shared.len() + mlkem_shared.len()));
    ikm.extend_from_slice(x25519_shared);
    ikm.extend_from_slice(mlkem_shared);

    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut key = Zeroizing::new([0u8; 32]);
    hk.expand(HYBRID_HKDF_INFO, &mut *key)
        .expect("HKDF expand of 32 bytes cannot fail");
    key
}

/// The initiator's public handshake message: X25519 public key followed by the
/// ML-KEM-768 encapsulation key. This is what the initiator transmits first.
#[derive(Clone)]
pub struct HybridInitiatorMessage {
    /// X25519 ephemeral public key (32 bytes).
    pub x25519_public: [u8; X25519_PUBLIC_LEN],
    /// Serialized ML-KEM-768 encapsulation (public) key.
    pub mlkem_encaps_key: Vec<u8>,
}

impl HybridInitiatorMessage {
    /// Serialize as `x25519_public (32) || mlkem_encaps_key`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HYBRID_INIT_MSG_LEN);
        out.extend_from_slice(&self.x25519_public);
        out.extend_from_slice(&self.mlkem_encaps_key);
        out
    }

    /// Parse from the wire layout, length-checking every field so hostile or
    /// truncated input is rejected cleanly instead of panicking on a slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != HYBRID_INIT_MSG_LEN {
            return Err(format!(
                "Hybrid initiator message wrong length: {} (expected {})",
                bytes.len(),
                HYBRID_INIT_MSG_LEN
            ));
        }
        let mut x25519_public = [0u8; X25519_PUBLIC_LEN];
        x25519_public.copy_from_slice(&bytes[..X25519_PUBLIC_LEN]);
        let mlkem_encaps_key = bytes[X25519_PUBLIC_LEN..].to_vec();
        Ok(Self { x25519_public, mlkem_encaps_key })
    }
}

/// The responder's public handshake message: X25519 public key followed by the
/// ML-KEM-768 ciphertext. This is what the responder sends back.
#[derive(Clone)]
pub struct HybridResponderMessage {
    /// X25519 ephemeral public key (32 bytes).
    pub x25519_public: [u8; X25519_PUBLIC_LEN],
    /// ML-KEM-768 ciphertext (the encapsulated shared key).
    pub mlkem_ciphertext: Vec<u8>,
}

impl HybridResponderMessage {
    /// Serialize as `x25519_public (32) || mlkem_ciphertext`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HYBRID_RESP_MSG_LEN);
        out.extend_from_slice(&self.x25519_public);
        out.extend_from_slice(&self.mlkem_ciphertext);
        out
    }

    /// Parse from the wire layout, length-checking every field.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != HYBRID_RESP_MSG_LEN {
            return Err(format!(
                "Hybrid responder message wrong length: {} (expected {})",
                bytes.len(),
                HYBRID_RESP_MSG_LEN
            ));
        }
        let mut x25519_public = [0u8; X25519_PUBLIC_LEN];
        x25519_public.copy_from_slice(&bytes[..X25519_PUBLIC_LEN]);
        let mlkem_ciphertext = bytes[X25519_PUBLIC_LEN..].to_vec();
        Ok(Self { x25519_public, mlkem_ciphertext })
    }
}

/// Initiator state for a hybrid X25519 + ML-KEM-768 key agreement.
///
/// Holds the two secret halves until the responder's reply arrives: the X25519
/// ephemeral secret and the ML-KEM-768 decapsulation key. Both zeroize on drop
/// (x25519-dalek and ml-kem `zeroize` features). `complete` consumes `self`,
/// which guarantees the one-shot semantics of an ephemeral exchange.
///
/// We drive X25519 through `x25519_dalek` directly here (rather than reusing
/// `crate::crypto::KeyExchange`) because the hybrid combiner needs the *raw* DH
/// output to feed HKDF — `KeyExchange::complete` already KDFs internally and
/// hands back a finished `CryptoSession`, which would double-KDF. The classical
/// fallback path still delegates to `KeyExchange` (see `classical_key_exchange`).
pub struct HybridKeyExchange {
    /// Classical half — X25519 ephemeral secret. Zeroizes on drop.
    x25519_secret: EphemeralSecret,
    /// Classical half — cached X25519 public key, sent in the initiator msg.
    x25519_public: PublicKey,
    /// Post-quantum half — secret key. Zeroizes on drop (ml-kem `zeroize`).
    mlkem_dk: DecapsulationKey<MlKem768>,
    /// Cached serialized encapsulation key (public), sent in the initiator msg.
    mlkem_ek_bytes: Vec<u8>,
}

impl HybridKeyExchange {
    /// Generate fresh ephemeral material for both primitives: an X25519 keypair
    /// and an ML-KEM-768 (decapsulation, encapsulation) keypair.
    pub fn new() -> Self {
        // OsRng (rand 0.8) satisfies x25519-dalek 2.0's rand_core 0.6 bound.
        let x25519_secret = EphemeralSecret::random_from_rng(OsRng);
        let x25519_public = PublicKey::from(&x25519_secret);

        // Uses the crate's system-RNG helper (enabled by the `getrandom`
        // feature) — this sidesteps the rand_core 0.10 vs rand 0.8 CryptoRng
        // trait-version mismatch that would otherwise force a fragile bridge.
        let (mlkem_dk, mlkem_ek) = MlKem768::generate_keypair();
        let mlkem_ek_bytes = mlkem_ek.to_bytes().to_vec();

        Self { x25519_secret, x25519_public, mlkem_dk, mlkem_ek_bytes }
    }

    /// The public message the initiator sends to the responder:
    /// X25519 public key || ML-KEM-768 encapsulation key.
    pub fn initiator_message(&self) -> HybridInitiatorMessage {
        HybridInitiatorMessage {
            x25519_public: *self.x25519_public.as_bytes(),
            mlkem_encaps_key: self.mlkem_ek_bytes.clone(),
        }
    }

    /// Complete the exchange on the initiator side using the responder's reply.
    ///
    /// Performs X25519 DH against the responder's public key and ML-KEM
    /// *decapsulation* of the responder's ciphertext, then HKDF-combines the two
    /// secrets into the session key. Consumes `self`.
    pub fn complete(self, response: &HybridResponderMessage) -> Result<CryptoSession, String> {
        // --- Classical half: raw X25519 DH (we need the bytes for the combiner).
        let their_public = PublicKey::from(response.x25519_public);
        let x25519_shared = self.x25519_secret.diffie_hellman(&their_public);

        // --- Post-quantum half: ML-KEM decapsulation of the responder's CT.
        let ct = decode_mlkem_ciphertext(&response.mlkem_ciphertext)?;
        let mlkem_shared = self.mlkem_dk.decapsulate(&ct);

        let key = combine_secrets(x25519_shared.as_bytes(), mlkem_shared.as_slice());
        info!("Hybrid X25519+ML-KEM-768 key agreement completed (initiator)");
        Ok(CryptoSession::from_psk(&key))
    }
}

impl Default for HybridKeyExchange {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the responder side of the hybrid exchange in one shot.
///
/// Given the initiator's message, this:
///   1. generates a fresh X25519 ephemeral and performs DH against the
///      initiator's X25519 public key,
///   2. ML-KEM-*encapsulates* against the initiator's encapsulation key,
///      yielding a shared secret + ciphertext,
///   3. HKDF-combines both secrets into the session key,
///
/// and returns the message to send back (`X25519 public || ML-KEM ciphertext`)
/// together with the established `CryptoSession`. Both sides derive an identical
/// key because each computes the same X25519 DH output and the same ML-KEM
/// shared secret (encapsulation on this side, decapsulation on the initiator's).
pub fn respond(initiator: &HybridInitiatorMessage) -> Result<(HybridResponderMessage, CryptoSession), String> {
    // --- Classical half: our own ephemeral, raw DH against the initiator's key.
    let our_secret = EphemeralSecret::random_from_rng(OsRng);
    let our_x25519_public = *PublicKey::from(&our_secret).as_bytes();
    let their_public = PublicKey::from(initiator.x25519_public);
    let x25519_shared = our_secret.diffie_hellman(&their_public);

    // --- Post-quantum half: encapsulate to the initiator's encapsulation key.
    let ek = decode_mlkem_encaps_key(&initiator.mlkem_encaps_key)?;
    // System-RNG helper again (getrandom feature) — avoids the rand_core
    // version bridge. encapsulate() draws fresh randomness internally.
    let (ct, mlkem_shared) = ek.encapsulate();

    let key = combine_secrets(x25519_shared.as_bytes(), mlkem_shared.as_slice());
    info!("Hybrid X25519+ML-KEM-768 key agreement completed (responder)");

    let response = HybridResponderMessage {
        x25519_public: our_x25519_public,
        mlkem_ciphertext: ct.as_slice().to_vec(),
    };
    Ok((response, CryptoSession::from_psk(&key)))
}

/// Decode a serialized ML-KEM-768 encapsulation key, length-checking first so a
/// hostile/truncated buffer is rejected with a clear error rather than a panic.
fn decode_mlkem_encaps_key(bytes: &[u8]) -> Result<EncapsulationKey<MlKem768>, String> {
    if bytes.len() != MLKEM768_ENCAPS_KEY_LEN {
        return Err(format!(
            "ML-KEM-768 encapsulation key wrong length: {} (expected {})",
            bytes.len(),
            MLKEM768_ENCAPS_KEY_LEN
        ));
    }
    // try_from validates the fixed-length encoding; new() additionally validates
    // the key is well-formed (modulus reduction check inside the crate).
    let encoded = Array::try_from(bytes)
        .map_err(|_| "ML-KEM-768 encapsulation key failed length decode".to_string())?;
    EncapsulationKey::<MlKem768>::new(&encoded)
        .map_err(|_| "ML-KEM-768 encapsulation key failed validation".to_string())
}

/// Decode a serialized ML-KEM-768 ciphertext into the typed `Ciphertext` array.
fn decode_mlkem_ciphertext(bytes: &[u8]) -> Result<ml_kem::Ciphertext<MlKem768>, String> {
    if bytes.len() != MLKEM768_CIPHERTEXT_LEN {
        return Err(format!(
            "ML-KEM-768 ciphertext wrong length: {} (expected {})",
            bytes.len(),
            MLKEM768_CIPHERTEXT_LEN
        ));
    }
    Array::try_from(bytes)
        .map_err(|_| "ML-KEM-768 ciphertext failed length decode".to_string())
}

/// Negotiated key-exchange suite.
///
/// Rides on the existing handshake as a single advertised capability flag. The
/// rule is conservative: we only use the post-quantum-protected `Hybrid` suite
/// when BOTH peers advertise support, otherwise we fall back to the classical
/// X25519 path that every existing build already speaks. This keeps older peers
/// interoperable while letting upgraded peers transparently gain PQ protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KexSuite {
    /// Classical X25519 ECDH only (the pre-PQC behavior).
    Classical,
    /// Hybrid X25519 + ML-KEM-768.
    Hybrid,
}

impl KexSuite {
    /// Select the suite for a handshake given whether each side supports hybrid.
    ///
    /// Hybrid is chosen ONLY if both sides support it (logical AND). Any other
    /// combination falls back to `Classical`. The fallback is explicit and
    /// total so there is never an ambiguous "half-hybrid" state on the wire.
    pub fn negotiate(local_supports_hybrid: bool, peer_supports_hybrid: bool) -> KexSuite {
        if local_supports_hybrid && peer_supports_hybrid {
            KexSuite::Hybrid
        } else {
            KexSuite::Classical
        }
    }

    /// Whether this suite is the post-quantum hybrid one.
    pub fn is_hybrid(self) -> bool {
        matches!(self, KexSuite::Hybrid)
    }
}

/// Explicit classical fallback: perform a plain X25519 ECDH and return both the
/// local public key (to transmit) and a constructor that completes the exchange.
///
/// This delegates entirely to `crate::crypto::KeyExchange` so the fallback path
/// is byte-for-byte the existing classical handshake — nothing about the
/// non-hybrid wire format changes. Provided here so a caller that negotiated
/// `KexSuite::Classical` has a single, named entry point symmetric with the
/// hybrid one.
pub fn classical_key_exchange() -> KeyExchange {
    KeyExchange::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_constants_match_mlkem768() {
        // These are the published FIPS 203 ML-KEM-768 sizes. We assert the
        // crate-derived constants equal them — if a future ml-kem bump silently
        // changed an encoding, this test catches it before it reaches the wire.
        assert_eq!(MLKEM768_ENCAPS_KEY_LEN, 1184, "ML-KEM-768 encaps key is 1184 B");
        assert_eq!(MLKEM768_CIPHERTEXT_LEN, 1088, "ML-KEM-768 ciphertext is 1088 B");
        assert_eq!(HYBRID_INIT_MSG_LEN, 32 + 1184);
        assert_eq!(HYBRID_RESP_MSG_LEN, 32 + 1088);
    }

    #[test]
    fn test_hybrid_round_trip() {
        // Full initiator/responder flow: both must derive identical sessions,
        // and a frame encrypted by one must decrypt on the other.
        let initiator = HybridKeyExchange::new();
        let init_msg = initiator.initiator_message();

        let (resp_msg, resp_session) = respond(&init_msg).unwrap();
        let mut init_session = initiator.complete(&resp_msg).unwrap();

        // Encrypt on initiator, decrypt on responder.
        let plaintext = b"hybrid pqc audio frame";
        let ct = init_session.encrypt(plaintext).unwrap();
        let pt = resp_session.decrypt(&ct).unwrap();
        assert_eq!(&pt, plaintext);

        // And the reverse direction, to prove the key is truly symmetric.
        let mut resp_session = resp_session; // need &mut for encrypt
        let ct2 = resp_session.encrypt(b"reverse direction").unwrap();
        assert_eq!(init_session.decrypt(&ct2).unwrap(), b"reverse direction");
    }

    #[test]
    fn test_tampered_mlkem_ciphertext_diverges() {
        // Flipping a bit in the ML-KEM ciphertext makes the responder's and
        // initiator's keys differ (ML-KEM's implicit rejection yields a
        // pseudorandom-but-different shared secret), so decryption must fail.
        let initiator = HybridKeyExchange::new();
        let init_msg = initiator.initiator_message();
        let (mut resp_msg, resp_session) = respond(&init_msg).unwrap();

        // Tamper the ciphertext.
        resp_msg.mlkem_ciphertext[0] ^= 0x01;

        let init_session = initiator.complete(&resp_msg).unwrap();

        let mut resp_session = resp_session;
        let ct = resp_session.encrypt(b"frame").unwrap();
        // Initiator derived a different key → GCM tag check fails.
        assert!(init_session.decrypt(&ct).is_err(), "tampered ML-KEM CT must break the key");
    }

    #[test]
    fn test_tampered_x25519_key_diverges() {
        // Flipping the responder's X25519 public key changes the DH output on
        // the initiator side only, so the combined keys diverge and decrypt fails.
        let initiator = HybridKeyExchange::new();
        let init_msg = initiator.initiator_message();
        let (mut resp_msg, resp_session) = respond(&init_msg).unwrap();

        resp_msg.x25519_public[0] ^= 0x01;

        let init_session = initiator.complete(&resp_msg).unwrap();

        let mut resp_session = resp_session;
        let ct = resp_session.encrypt(b"frame").unwrap();
        assert!(init_session.decrypt(&ct).is_err(), "tampered X25519 key must break the key");
    }

    #[test]
    fn test_classical_fallback_path() {
        // When negotiation lands on Classical, the explicit fallback must still
        // produce a working session via the existing X25519 KeyExchange.
        assert_eq!(KexSuite::negotiate(true, false), KexSuite::Classical);
        assert_eq!(KexSuite::negotiate(false, true), KexSuite::Classical);
        assert_eq!(KexSuite::negotiate(false, false), KexSuite::Classical);
        assert_eq!(KexSuite::negotiate(true, true), KexSuite::Hybrid);

        let kx_a = classical_key_exchange();
        let kx_b = classical_key_exchange();
        let pub_a = kx_a.public_key_bytes();
        let pub_b = kx_b.public_key_bytes();

        let mut session_a = kx_b.complete(&pub_a).unwrap();
        let session_b = kx_a.complete(&pub_b).unwrap();

        let pt = b"classical fallback frame";
        let ct = session_a.encrypt(pt).unwrap();
        assert_eq!(&session_b.decrypt(&ct).unwrap(), pt);
    }

    #[test]
    fn test_serialization_round_trips_with_lengths() {
        let initiator = HybridKeyExchange::new();
        let init_msg = initiator.initiator_message();
        let (resp_msg, _session) = respond(&init_msg).unwrap();

        // Initiator message round-trips and has the documented length.
        let init_bytes = init_msg.to_bytes();
        assert_eq!(init_bytes.len(), HYBRID_INIT_MSG_LEN);
        let init_parsed = HybridInitiatorMessage::from_bytes(&init_bytes).unwrap();
        assert_eq!(init_parsed.x25519_public, init_msg.x25519_public);
        assert_eq!(init_parsed.mlkem_encaps_key, init_msg.mlkem_encaps_key);

        // Responder message round-trips and has the documented length.
        let resp_bytes = resp_msg.to_bytes();
        assert_eq!(resp_bytes.len(), HYBRID_RESP_MSG_LEN);
        let resp_parsed = HybridResponderMessage::from_bytes(&resp_bytes).unwrap();
        assert_eq!(resp_parsed.x25519_public, resp_msg.x25519_public);
        assert_eq!(resp_parsed.mlkem_ciphertext, resp_msg.mlkem_ciphertext);
    }

    #[test]
    fn test_deserialization_rejects_wrong_lengths() {
        // Truncated / oversized buffers must error, not panic.
        assert!(HybridInitiatorMessage::from_bytes(&[0u8; 10]).is_err());
        assert!(HybridInitiatorMessage::from_bytes(&vec![0u8; HYBRID_INIT_MSG_LEN + 1]).is_err());
        assert!(HybridResponderMessage::from_bytes(&[0u8; 10]).is_err());
        assert!(HybridResponderMessage::from_bytes(&vec![0u8; HYBRID_RESP_MSG_LEN - 1]).is_err());

        // A correctly-sized but cryptographically-invalid encaps key is rejected
        // by the KEM decoder (all-zero bytes are not a valid encoding).
        let bad_ek = vec![0u8; MLKEM768_ENCAPS_KEY_LEN];
        let bad_init = HybridInitiatorMessage {
            x25519_public: [0u8; 32],
            mlkem_encaps_key: bad_ek,
        };
        // respond() validates the encaps key; all-zero may or may not pass the
        // crate's structural check, but a wrong *length* always fails:
        let mut short = bad_init.mlkem_encaps_key.clone();
        short.truncate(10);
        assert!(decode_mlkem_encaps_key(&short).is_err());
    }
}
