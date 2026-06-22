//! sealed — Metadata-resistant blinded room/peer handles (pure logic).
//!
//! Derives ROTATING, UNLINKABLE room/peer tokens from the shared session
//! secret (the 32-byte key every room member already holds via QR/PSK or the
//! PQC handshake), so the Cloudflare relay — a blind audio forwarder — only
//! ever sees opaque values it cannot correlate to identity or across time.
//!
//! # The problem
//!
//! Today the relay still receives correlatable connection metadata as plaintext
//! WebSocket query params (`?room=...&peer=...&device=...`). Even though the
//! audio payload is end-to-end encrypted, these let the relay (or anyone
//! observing it) link who talks to whom and track a specific peer install
//! across sessions and over time. That is the metadata-resistance / "sealed
//! sender" gap this module closes.
//!
//! # The construction
//!
//! All handles are HKDF-SHA256 expansions keyed by the shared session key, with
//! the rotation epoch bound into the `info` string. Because HKDF's underlying
//! HMAC-SHA256 is a PRF, its output is computationally indistinguishable from
//! random to anyone who does not hold the key. Concretely:
//!
//! * **Unlinkable to the relay.** The relay never holds the session key, so it
//!   cannot derive the room id itself, cannot recognise a room across epochs
//!   (each epoch is a fresh, independent-looking token), and cannot reverse a
//!   sealed peer handle back to a stable peer id. To the relay every token is
//!   just random bytes.
//! * **Agreed by all members.** The derivation is deterministic in
//!   `(key, epoch)`, so every device in a room computes the *same* blinded room
//!   id and lands in the same relay room without any extra coordination.
//! * **Rotating.** The epoch — `floor(now / epoch_secs)` — is mixed in, so the
//!   room id and every peer handle change every epoch window. The relay cannot
//!   stitch one logical room's traffic together across windows.
//! * **Per-peer pseudonyms.** A sealed peer handle additionally binds the
//!   stable peer id, yielding a fresh pseudonym each epoch that is unlinkable to
//!   the install identity and to the same install's handle in any other epoch.
//!
//! # Epoch-rotation trade-off
//!
//! Shorter epochs give better unlinkability (a narrower correlation window) at
//! the cost of more rejoin churn at each boundary. We default to **15 minutes**
//! ([`DEFAULT_EPOCH_SECS`]) as a balance, and provide a seamless handoff helper
//! ([`room_ids_for_handoff`] / [`seconds_until_next_epoch`]) so a client can
//! pre-join the next epoch's room before the current one rolls over, avoiding
//! an audio gap at the boundary.
//!
//! # Scope
//!
//! This is **defense-in-depth ON TOP of** the E2E audio encryption in
//! `crypto.rs`, not a replacement for it. It hides *who is talking to whom*
//! (connection metadata) from the relay; the audio contents are already
//! protected by AES-256-GCM. Peer identity that the app still needs (e.g. which
//! member sent a frame) travels inside the E2E-encrypted audio frame header,
//! which the relay cannot read — so it never needs to appear in the clear.

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Default rotation window: 15 minutes.
///
/// Chosen as the unlinkability-vs-churn balance documented at the module level:
/// short enough that the relay's correlation window across a logical room is
/// only a quarter hour, long enough that members are not constantly rejoining.
pub const DEFAULT_EPOCH_SECS: u64 = 15 * 60;

/// Length, in URL-safe-base64 characters, of the blinded room id.
///
/// 24 base64url chars encode 18 bytes (144 bits) of HKDF output. This sits
/// comfortably inside the relay's accepted room-id length window
/// ([`ROOM_ID_MIN_LEN`]..=[`ROOM_ID_MAX_LEN`], i.e. 8..=64) and is plenty of
/// entropy to make blind guessing of an active room infeasible.
const ROOM_ID_B64_LEN: usize = 24;
/// Bytes of HKDF output backing a room id (`ROOM_ID_B64_LEN` base64url chars).
const ROOM_ID_BYTES: usize = 18;

/// Length, in URL-safe-base64 characters, of a sealed peer handle.
///
/// 22 base64url chars encode 16 bytes (128 bits). Well under the relay's 64-char
/// peer-id cap and unlinkable.
const PEER_HANDLE_B64_LEN: usize = 22;
/// Bytes of HKDF output backing a peer handle (`PEER_HANDLE_B64_LEN` chars).
const PEER_HANDLE_BYTES: usize = 16;

/// Relay's accepted room-id length window (inclusive). Mirrors the worker's
/// `isValidRoomId`, which requires `id.length >= 8 && id.length <= 64`.
pub const ROOM_ID_MIN_LEN: usize = 8;
/// Upper bound of the relay's accepted room-id length window (inclusive).
pub const ROOM_ID_MAX_LEN: usize = 64;

/// Domain-separation prefix for the blinded room id derivation. Bound together
/// with the epoch so room and peer derivations never collide and so a future
/// format change is a clean break.
const ROOM_INFO_PREFIX: &[u8] = b"sassytalkie-sealed-room-v1";
/// Domain-separation prefix for the sealed peer handle derivation.
const PEER_INFO_PREFIX: &[u8] = b"sassytalkie-sealed-peer-v1";

/// Placeholder the client sends in the `device=` WebSocket query param when
/// sealed handles are in use.
///
/// With sealed handles the device name MUST stay off the relay entirely — it is
/// a stable, often human-readable identifier that would defeat the whole
/// exercise. The client sends this constant instead; any per-peer identity the
/// app needs is conveyed inside the E2E-encrypted audio frame header, which the
/// relay cannot read. An empty string would also work; a short constant is used
/// so logs/dashboards show an intentional sentinel rather than a blank field.
pub const SEALED_DEVICE_PLACEHOLDER: &str = "sealed";

/// Compute the rotation epoch for a wall-clock time.
///
/// `floor(now_ms / 1000 / epoch_secs)` — the index of the `epoch_secs`-wide
/// window that `now_ms` falls in. Every device that shares a clock (to within
/// `epoch_secs`) computes the same epoch, and therefore the same handles.
///
/// `epoch_secs` of 0 is treated as 1 second to avoid a divide-by-zero; callers
/// should pass [`DEFAULT_EPOCH_SECS`].
pub fn current_epoch(now_ms: u64, epoch_secs: u64) -> u64 {
    let epoch_secs = epoch_secs.max(1);
    (now_ms / 1000) / epoch_secs
}

/// Seconds remaining until the current epoch rolls over to the next one.
///
/// Returns a value in `1..=epoch_secs` (never 0): exactly on a boundary the
/// full window remains. A client uses this to schedule the pre-join of the next
/// epoch's room (see [`room_ids_for_handoff`]) so audio does not drop when the
/// room id rotates.
pub fn seconds_until_next_epoch(now_ms: u64, epoch_secs: u64) -> u64 {
    let epoch_secs = epoch_secs.max(1);
    let now_s = now_ms / 1000;
    let elapsed_in_epoch = now_s % epoch_secs;
    epoch_secs - elapsed_in_epoch
}

/// Derive the blinded room id for a given `(session_key, epoch)`.
///
/// Deterministic: every device holding the same 32-byte session key produces
/// the identical room id for the same epoch, so they all converge on the same
/// relay room. The relay, lacking the key, sees only `ROOM_ID_B64_LEN`
/// URL-safe-base64 characters of PRF output — random-looking and uncorrelatable
/// either to the underlying session or across epochs.
///
/// Output is `ROOM_ID_B64_LEN` (24) URL-safe base64 chars, inside the relay's
/// 8..=64 window and URL-safe with no padding so it needs no extra escaping.
pub fn blinded_room_id(session_key: &[u8; 32], epoch: u64) -> String {
    let info = room_info(epoch);
    let mut okm = Zeroizing::new([0u8; ROOM_ID_BYTES]);
    expand(session_key, &info, &mut *okm);
    let s = URL_SAFE_NO_PAD.encode(&*okm);
    debug_assert_eq!(s.len(), ROOM_ID_B64_LEN);
    s
}

/// Derive a sealed per-peer handle for a given `(session_key, stable_peer_id,
/// epoch)`.
///
/// The result is a fresh pseudonym for this peer in this epoch:
///
/// * **Not reversible** to `stable_peer_id` without the session key (it is PRF
///   output over the key).
/// * **Unlinkable across epochs** — the same peer gets an unrelated-looking
///   handle every epoch.
/// * **Distinct per peer** — different stable peer ids under the same
///   `(key, epoch)` yield different handles.
///
/// Output is `PEER_HANDLE_B64_LEN` (22) URL-safe base64 chars, `<= 64` so it
/// fits the relay's peer-id cap, and URL-safe so it needs no extra escaping.
pub fn sealed_peer_handle(session_key: &[u8; 32], stable_peer_id: &str, epoch: u64) -> String {
    let info = peer_info(stable_peer_id, epoch);
    let mut okm = Zeroizing::new([0u8; PEER_HANDLE_BYTES]);
    expand(session_key, &info, &mut *okm);
    let s = URL_SAFE_NO_PAD.encode(&*okm);
    debug_assert_eq!(s.len(), PEER_HANDLE_B64_LEN);
    s
}

/// Current and next epochs' blinded room ids, for seamless epoch-boundary
/// handoff.
///
/// Returns `(current, next)`. A client connects to `current`, and shortly
/// before the boundary (see [`seconds_until_next_epoch`]) also opens `next`, so
/// when the epoch rolls over there is no window in which it is in no room and
/// audio is not dropped. Once the boundary passes it can drop the old room.
pub fn room_ids_for_handoff(
    session_key: &[u8; 32],
    now_ms: u64,
    epoch_secs: u64,
) -> (String, String) {
    let epoch = current_epoch(now_ms, epoch_secs);
    let current = blinded_room_id(session_key, epoch);
    let next = blinded_room_id(session_key, epoch.wrapping_add(1));
    (current, next)
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// HKDF-SHA256 `info` for a room derivation: domain-separation prefix followed
/// by the epoch as 8 LE bytes. Binding the epoch as bytes (not formatted text)
/// keeps the input unambiguous.
fn room_info(epoch: u64) -> Vec<u8> {
    let mut info = Vec::with_capacity(ROOM_INFO_PREFIX.len() + 8);
    info.extend_from_slice(ROOM_INFO_PREFIX);
    info.extend_from_slice(&epoch.to_le_bytes());
    info
}

/// HKDF-SHA256 `info` for a peer derivation: domain-separation prefix, the
/// epoch as 8 LE bytes, then the stable peer id bytes. A length prefix on the
/// peer id removes any ambiguity between the fixed-width epoch field and the
/// variable-width peer id, so no two distinct inputs can serialise identically.
fn peer_info(stable_peer_id: &str, epoch: u64) -> Vec<u8> {
    let id = stable_peer_id.as_bytes();
    let mut info = Vec::with_capacity(PEER_INFO_PREFIX.len() + 8 + 8 + id.len());
    info.extend_from_slice(PEER_INFO_PREFIX);
    info.extend_from_slice(&epoch.to_le_bytes());
    info.extend_from_slice(&(id.len() as u64).to_le_bytes());
    info.extend_from_slice(id);
    info
}

/// Expand the session key into `out` bytes via HKDF-SHA256 with the given
/// `info`. The session key is the input keying material; no salt is used (the
/// key is already high-entropy and the per-purpose `info` provides domain
/// separation). Expanding `out.len()` bytes well under 255*32 cannot fail.
fn expand(session_key: &[u8; 32], info: &[u8], out: &mut [u8]) {
    let hk = Hkdf::<Sha256>::new(None, session_key);
    hk.expand(info, out)
        .expect("HKDF expand of a handle-sized output cannot fail");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_a() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    fn key_b() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = (255 - i) as u8;
        }
        k
    }

    #[test]
    fn room_id_is_deterministic_for_key_and_epoch() {
        let k = key_a();
        let a = blinded_room_id(&k, 42);
        let b = blinded_room_id(&k, 42);
        assert_eq!(a, b, "same (key, epoch) must yield identical room id");
    }

    #[test]
    fn peer_handle_is_deterministic_for_key_peer_and_epoch() {
        let k = key_a();
        let a = sealed_peer_handle(&k, "peer-install-1", 7);
        let b = sealed_peer_handle(&k, "peer-install-1", 7);
        assert_eq!(a, b, "same (key, peer, epoch) must yield identical handle");
    }

    #[test]
    fn room_id_rotates_across_epochs() {
        let k = key_a();
        assert_ne!(
            blinded_room_id(&k, 100),
            blinded_room_id(&k, 101),
            "different epochs must produce different room ids"
        );
    }

    #[test]
    fn peer_handle_rotates_across_epochs() {
        let k = key_a();
        let id = "stable-peer";
        assert_ne!(
            sealed_peer_handle(&k, id, 100),
            sealed_peer_handle(&k, id, 101),
            "different epochs must produce different peer handles"
        );
    }

    #[test]
    fn different_peers_get_different_handles_same_epoch() {
        let k = key_a();
        assert_ne!(
            sealed_peer_handle(&k, "peer-1", 5),
            sealed_peer_handle(&k, "peer-2", 5),
            "different stable peer ids must produce different handles"
        );
    }

    #[test]
    fn same_peer_different_keys_get_different_handles() {
        let id = "peer-1";
        assert_ne!(
            sealed_peer_handle(&key_a(), id, 5),
            sealed_peer_handle(&key_b(), id, 5),
            "same peer under different session keys must differ (unlinkable across rooms)"
        );
    }

    #[test]
    fn different_keys_get_different_room_ids() {
        assert_ne!(
            blinded_room_id(&key_a(), 5),
            blinded_room_id(&key_b(), 5),
            "different session keys must land in different relay rooms"
        );
    }

    #[test]
    fn room_id_length_is_within_relay_bounds() {
        let k = key_a();
        for epoch in [0u64, 1, 42, u64::MAX] {
            let id = blinded_room_id(&k, epoch);
            assert!(
                id.len() >= ROOM_ID_MIN_LEN && id.len() <= ROOM_ID_MAX_LEN,
                "room id len {} out of relay 8..=64 window",
                id.len()
            );
            assert_eq!(id.len(), ROOM_ID_B64_LEN);
        }
    }

    #[test]
    fn peer_handle_length_is_bounded() {
        let k = key_a();
        // Include a long stable id to prove the handle length is independent of
        // the input length (it is a fixed-width hash output).
        let long_id = "x".repeat(4096);
        for id in ["", "peer", long_id.as_str()] {
            let h = sealed_peer_handle(&k, id, 9);
            assert!(h.len() <= 64, "peer handle len {} exceeds 64", h.len());
            assert_eq!(h.len(), PEER_HANDLE_B64_LEN);
        }
    }

    #[test]
    fn room_id_is_url_safe() {
        // URL-safe base64 (no padding) uses only [A-Za-z0-9_-]; nothing here
        // needs percent-encoding, so it slots straight into the WS query.
        let id = blinded_room_id(&key_a(), 1234);
        assert!(
            id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "room id '{}' must be url-safe",
            id
        );
    }

    #[test]
    fn current_epoch_floors_correctly() {
        // epoch_secs = 900 (15 min). 900_000 ms == 1 epoch exactly.
        assert_eq!(current_epoch(0, 900), 0);
        assert_eq!(current_epoch(899_999, 900), 0, "just before the boundary");
        assert_eq!(current_epoch(900_000, 900), 1, "exactly on the boundary");
        assert_eq!(current_epoch(900_001, 900), 1, "just after the boundary");
        assert_eq!(current_epoch(1_799_999, 900), 1);
        assert_eq!(current_epoch(1_800_000, 900), 2);
    }

    #[test]
    fn current_epoch_handles_zero_epoch_secs() {
        // Guarded against divide-by-zero: treated as 1-second epochs.
        let _ = current_epoch(12_345, 0);
    }

    #[test]
    fn seconds_until_next_epoch_boundary_math() {
        // epoch_secs = 900. At t=0 the full window remains.
        assert_eq!(seconds_until_next_epoch(0, 900), 900);
        // 1 s in → 899 left.
        assert_eq!(seconds_until_next_epoch(1_000, 900), 899);
        // 899 s in → 1 s left.
        assert_eq!(seconds_until_next_epoch(899_000, 900), 1);
        // Exactly on the next boundary → full window again, never 0.
        assert_eq!(seconds_until_next_epoch(900_000, 900), 900);
        // Sub-second progress within a second still counts that second elapsed
        // only at the 1000 ms mark (integer seconds).
        assert_eq!(seconds_until_next_epoch(900_500, 900), 900);
    }

    #[test]
    fn handoff_pairs_current_and_next_epoch() {
        let k = key_a();
        // Pick a time mid-epoch.
        let now_ms = 450_000; // 450 s into epoch 0 of a 900 s window
        let (current, next) = room_ids_for_handoff(&k, now_ms, 900);
        let epoch = current_epoch(now_ms, 900);
        assert_eq!(current, blinded_room_id(&k, epoch));
        assert_eq!(next, blinded_room_id(&k, epoch + 1));
        assert_ne!(current, next, "handoff rooms must differ across the boundary");
    }

    #[test]
    fn handoff_next_becomes_current_after_boundary() {
        // The room a client pre-joins for handoff must be exactly the room it
        // computes as `current` once the epoch actually rolls over — otherwise
        // the pre-join would be useless.
        let k = key_a();
        let epoch_secs = 900;
        let before = 899_000; // 1 s before boundary, epoch 0
        let after = 901_000; // 1 s after boundary, epoch 1
        let (_, next_before) = room_ids_for_handoff(&k, before, epoch_secs);
        let (current_after, _) = room_ids_for_handoff(&k, after, epoch_secs);
        assert_eq!(
            next_before, current_after,
            "pre-joined next room must equal current room after rollover"
        );
    }

    #[test]
    fn empty_peer_id_is_handled() {
        let k = key_a();
        let h = sealed_peer_handle(&k, "", 1);
        assert_eq!(h.len(), PEER_HANDLE_B64_LEN);
    }
}
