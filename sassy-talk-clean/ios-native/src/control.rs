//! control — iOS control-plane frames (heartbeat) for the relay / liveness path.
//!
//! Reuses the shared opcodes + TLV framing from `sassytalkie_core::protocol`, so
//! the wire format is byte-identical to android-native (`ControlFrame.kt`) and
//! tauri-desktop (`transport/control.rs`). iOS previously emitted no control
//! frames at all, so it never appeared in peers' liveness rosters and would be
//! swept by the relay's idle-staleness timer; the heartbeat below fixes that on
//! the relay path.

use sassytalkie_core::protocol::{encode_tlv, OP_HEARTBEAT};

/// Presence state byte (matches desktop `PresenceState` / Android).
pub const PRESENCE_IDLE: u8 = 0;
pub const PRESENCE_SPEAKING: u8 = 2;

/// Encode an `OP_HEARTBEAT` TLV. Payload (23 bytes, little-endian):
///   `[epoch:u64][seq:u32][ts_ms:u64][state:u8][rtt_ms:u16]`
///
/// No trailing capabilities byte: iOS does not (yet) run the PQC handshake, so
/// like desktop it advertises caps = 0 by omission. Receivers tolerate a 23- or
/// 24-byte payload, so this stays forward-compatible.
pub fn encode_heartbeat(epoch: u64, seq: u32, ts_ms: u64, state: u8, rtt_ms: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(23);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&ts_ms.to_le_bytes());
    p.push(state);
    p.extend_from_slice(&rtt_ms.to_le_bytes());
    encode_tlv(OP_HEARTBEAT, &p)
}

/// Wall-clock milliseconds since the unix epoch (heartbeat `ts_ms`).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_is_tlv_with_23_byte_payload() {
        let hb = encode_heartbeat(0xDEAD_BEEF_CAFE_F00D, 7, 1_700_000_000_000, PRESENCE_IDLE, 0);
        // [op][len:u16 LE][payload]
        assert_eq!(hb[0], OP_HEARTBEAT);
        let len = u16::from_le_bytes([hb[1], hb[2]]) as usize;
        assert_eq!(len, 23);
        assert_eq!(hb.len(), 3 + 23);
    }
}
