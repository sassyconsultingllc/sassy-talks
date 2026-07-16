// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-XY5S3HJSKJBC
//! wire — the audio DATA-plane frame shared by every SassyTalkie endpoint.
//!
//! This is the multicast/relay audio frame (distinct from the control-plane
//! opcodes in [`crate::protocol`]). It carries one Opus frame plus the minimal
//! routing/identity header every receiver needs to attribute and mix audio.
//!
//! It lives here, in `sassytalkie-core`, for the SAME reason `protocol` does:
//! the format was previously defined only in `android-native`
//! (`audio_pipeline::pack_wire_frame`), so the iOS and desktop consumers had no
//! shared definition to build against and could silently drift out of byte
//! compatibility. Every platform now packs/unpacks through this one function,
//! so an iOS frame and an Android frame are byte-identical on the wire.
//!
//! # Frame layout
//!
//! ```text
//! [channel:u8]
//! [subchannel:u8]                 0=Main, 1=A, 2=B
//! [sender_id_len:u8]              <= MAX_SENDER_ID_LEN
//! [sender_id: sender_id_len]      UTF-8, the stable per-install id
//! [device_name_len:u8]           <= MAX_DEVICE_NAME_LEN
//! [device_name: device_name_len] UTF-8, human-readable
//! [timestamp:u64 LE]             ms since unix epoch (capture time)
//! [compressed_audio: rest]       one Opus frame
//! ```
//!
//! The WHOLE frame is then sealed by the transport's `CryptoSession`
//! (`nonce||ct||tag`, AES-256-GCM) before it touches the socket — identity and
//! routing header included, so the relay/observer sees only ciphertext. Keep
//! this in lock-step across platforms: changing a field is a breaking wire
//! change (see the `protocol` module preamble).

/// Maximum sender id length on the wire (bytes). Longer ids are truncated by
/// [`pack_wire_frame`]; [`unpack_wire_frame`] rejects frames claiming more.
pub const MAX_SENDER_ID_LEN: usize = 32;

/// Maximum device-name length on the wire (bytes). Same truncate-on-pack /
/// reject-on-unpack contract as [`MAX_SENDER_ID_LEN`].
pub const MAX_DEVICE_NAME_LEN: usize = 64;

/// Minimum decodable frame: channel + subchannel + id_len + name_len +
/// timestamp, with zero-length id and name = 12 bytes.
const MIN_FRAME_LEN: usize = 12;

/// Subchannel: main/default.
pub const SUBCH_MAIN: u8 = 0;
/// Subchannel A.
pub const SUBCH_A: u8 = 1;
/// Subchannel B.
pub const SUBCH_B: u8 = 2;

/// Current time in milliseconds since the unix epoch (the `timestamp` field).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Encapsulate one Opus frame for transport over the wire.
///
/// `sender_id` / `device_name` are truncated to their max lengths (on a UTF-8
/// boundary-safe byte prefix; over-long inputs are clipped to whole bytes,
/// matching the historical Android behaviour). The returned buffer is the
/// cleartext frame — the transport seals it before sending.
pub fn pack_wire_frame(
    channel: u8,
    subchannel: u8,
    sender_id: &str,
    device_name: &str,
    timestamp: u64,
    compressed: &[u8],
) -> Vec<u8> {
    let id_bytes = sender_id.as_bytes();
    let id_len = id_bytes.len().min(MAX_SENDER_ID_LEN);
    let name_bytes = device_name.as_bytes();
    let name_len = name_bytes.len().min(MAX_DEVICE_NAME_LEN);

    let mut packet = Vec::with_capacity(2 + 1 + id_len + 1 + name_len + 8 + compressed.len());
    packet.push(channel);
    packet.push(subchannel);
    packet.push(id_len as u8);
    packet.extend_from_slice(&id_bytes[..id_len]);
    packet.push(name_len as u8);
    packet.extend_from_slice(&name_bytes[..name_len]);
    packet.extend_from_slice(&timestamp.to_le_bytes());
    packet.extend_from_slice(compressed);
    packet
}

/// Parse a wire frame back into its components.
///
/// Returns `(channel, subchannel, sender_id, device_name, timestamp,
/// compressed_audio)` or an error string describing the malformation. Hostile
/// length fields are bounds-checked against `MAX_*` and the buffer length, so a
/// crafted frame cannot index out of bounds.
pub fn unpack_wire_frame(data: &[u8]) -> Result<(u8, u8, String, String, u64, Vec<u8>), String> {
    if data.len() < MIN_FRAME_LEN {
        return Err(format!("Wire frame too short: {} bytes", data.len()));
    }

    let channel = data[0];
    let subchannel = data[1];
    let id_len = data[2] as usize;

    if id_len > MAX_SENDER_ID_LEN || data.len() < 3 + id_len + 1 {
        return Err(format!("Invalid sender_id length: {}", id_len));
    }

    let sender_id = String::from_utf8_lossy(&data[3..3 + id_len]).to_string();

    let name_len_offset = 3 + id_len;
    let name_len = data[name_len_offset] as usize;

    if name_len > MAX_DEVICE_NAME_LEN || data.len() < name_len_offset + 1 + name_len + 8 {
        return Err(format!("Invalid device_name length: {}", name_len));
    }

    let name_start = name_len_offset + 1;
    let device_name = String::from_utf8_lossy(&data[name_start..name_start + name_len]).to_string();

    let ts_offset = name_start + name_len;
    let timestamp = u64::from_le_bytes([
        data[ts_offset], data[ts_offset + 1], data[ts_offset + 2], data[ts_offset + 3],
        data[ts_offset + 4], data[ts_offset + 5], data[ts_offset + 6], data[ts_offset + 7],
    ]);

    let audio_offset = ts_offset + 8;
    let compressed = data[audio_offset..].to_vec();

    Ok((channel, subchannel, sender_id, device_name, timestamp, compressed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_all_fields() {
        let audio = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let frame = pack_wire_frame(3, SUBCH_A, "ios-deadbeef", "iPhone 99", 1_700_000_000_123, &audio);
        let (ch, sub, id, name, ts, got) = unpack_wire_frame(&frame).unwrap();
        assert_eq!(ch, 3);
        assert_eq!(sub, SUBCH_A);
        assert_eq!(id, "ios-deadbeef");
        assert_eq!(name, "iPhone 99");
        assert_eq!(ts, 1_700_000_000_123);
        assert_eq!(got, audio);
    }

    #[test]
    fn empty_id_and_name_still_round_trip() {
        let audio = vec![9u8; 40];
        let frame = pack_wire_frame(1, SUBCH_MAIN, "", "", 0, &audio);
        let (ch, sub, id, name, ts, got) = unpack_wire_frame(&frame).unwrap();
        assert_eq!((ch, sub), (1, 0));
        assert!(id.is_empty() && name.is_empty());
        assert_eq!(ts, 0);
        assert_eq!(got, audio);
    }

    #[test]
    fn byte_layout_is_exact() {
        // Locks the on-wire byte order so a refactor can't silently shift it and
        // break iOS<->Android interop. ch=2, sub=0, id="ab", name="c", ts=1.
        let frame = pack_wire_frame(2, 0, "ab", "c", 1, &[0xEE]);
        assert_eq!(
            frame,
            vec![
                2,            // channel
                0,            // subchannel
                2,            // id_len
                b'a', b'b',   // sender_id
                1,            // name_len
                b'c',         // device_name
                1, 0, 0, 0, 0, 0, 0, 0, // timestamp u64 LE
                0xEE,         // audio
            ]
        );
    }

    #[test]
    fn rejects_oversized_sender_id_len() {
        let mut data = vec![0u8; 20];
        data[2] = 200; // sender_id_len > MAX_SENDER_ID_LEN
        assert!(unpack_wire_frame(&data).is_err());
    }

    #[test]
    fn rejects_oversized_device_name_len() {
        let mut data = vec![0u8; 20];
        data[2] = 0;   // sender_id_len = 0
        data[3] = 200; // name_len > MAX_DEVICE_NAME_LEN
        assert!(unpack_wire_frame(&data).is_err());
    }

    #[test]
    fn rejects_too_short() {
        assert!(unpack_wire_frame(&[0u8; 4]).is_err());
    }
}
