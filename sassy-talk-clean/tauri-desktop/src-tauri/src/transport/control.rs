use rand::RngCore;

pub const OP_PTT_START: u8      = 0x01;
pub const OP_PTT_STOP: u8       = 0x02;
pub const OP_READY_ACK: u8      = 0x03;
pub const OP_PING: u8           = 0x04;
pub const OP_CHANNEL_SYNC: u8   = 0x05;
pub const OP_HEARTBEAT: u8      = 0x10;
pub const OP_RECV_ACK: u8       = 0x11;
pub const OP_EOT_ACK: u8        = 0x12;
pub const OP_CAPABILITIES: u8   = 0x13;
pub const OP_PARTNER_OFFLINE: u8= 0x14;
pub const OP_PTT_START_V2: u8   = 0x15;
pub const OP_PTT_STOP_V2: u8    = 0x16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PresenceState {
    Idle = 0,
    Listening = 1,
    Speaking = 2,
    Muted = 3,
    Away = 4,
    Backgrounded = 5,
    Dnd = 6,
}

impl PresenceState {
    pub fn from_byte(b: u8) -> Self {
        match b {
            1 => Self::Listening,
            2 => Self::Speaking,
            3 => Self::Muted,
            4 => Self::Away,
            5 => Self::Backgrounded,
            6 => Self::Dnd,
            _ => Self::Idle,
        }
    }
    pub fn as_byte(self) -> u8 { self as u8 }
}

#[derive(Debug)]
pub struct Decoded {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct Heartbeat {
    pub epoch: u64,
    pub seq: u32,
    pub ts_ms: u64,
    pub state: PresenceState,
    pub rtt_ms: u16,
}

#[derive(Debug)]
pub struct RecvAck {
    pub epoch: u64,
    pub last_seq: u32,
    pub ts_ms: u64,
}

pub fn encode_legacy(op: u8) -> Vec<u8> {
    vec![op]
}

pub fn encode_tlv(op: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= 0xFFFF, "payload too big");
    let mut out = Vec::with_capacity(3 + payload.len());
    out.push(op);
    out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode(bytes: &[u8]) -> Decoded {
    if bytes.is_empty() {
        return Decoded { opcode: 0, payload: vec![] };
    }
    let op = bytes[0];
    if op < 0x10 || bytes.len() < 3 {
        return Decoded { opcode: op, payload: vec![] };
    }
    let len = u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
    let end = (3 + len).min(bytes.len());
    Decoded { opcode: op, payload: bytes[3..end].to_vec() }
}

pub fn encode_heartbeat(epoch: u64, seq: u32, ts_ms: u64,
                        state: PresenceState, rtt_ms: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(23);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&ts_ms.to_le_bytes());
    p.push(state.as_byte());
    p.extend_from_slice(&rtt_ms.to_le_bytes());
    encode_tlv(OP_HEARTBEAT, &p)
}

pub fn parse_heartbeat(payload: &[u8]) -> Option<Heartbeat> {
    if payload.len() < 23 { return None; }
    Some(Heartbeat {
        epoch:  u64::from_le_bytes(payload[0..8].try_into().ok()?),
        seq:    u32::from_le_bytes(payload[8..12].try_into().ok()?),
        ts_ms:  u64::from_le_bytes(payload[12..20].try_into().ok()?),
        state:  PresenceState::from_byte(payload[20]),
        rtt_ms: u16::from_le_bytes(payload[21..23].try_into().ok()?),
    })
}

pub fn encode_recv_ack(epoch: u64, last_seq: u32, ts_ms: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(20);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&last_seq.to_le_bytes());
    p.extend_from_slice(&ts_ms.to_le_bytes());
    encode_tlv(OP_RECV_ACK, &p)
}

pub fn parse_recv_ack(payload: &[u8]) -> Option<RecvAck> {
    if payload.len() < 20 { return None; }
    Some(RecvAck {
        epoch: u64::from_le_bytes(payload[0..8].try_into().ok()?),
        last_seq: u32::from_le_bytes(payload[8..12].try_into().ok()?),
        ts_ms: u64::from_le_bytes(payload[12..20].try_into().ok()?),
    })
}

pub fn encode_eot_ack(epoch: u64, up_to_seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&up_to_seq.to_le_bytes());
    encode_tlv(OP_EOT_ACK, &p)
}

pub fn encode_partner_offline(peer_id: &str) -> Vec<u8> {
    let id_bytes = peer_id.as_bytes();
    let mut p = Vec::with_capacity(1 + id_bytes.len());
    p.push(id_bytes.len() as u8);
    p.extend_from_slice(id_bytes);
    encode_tlv(OP_PARTNER_OFFLINE, &p)
}

pub fn encode_ptt_start_v2(epoch: u64, start_seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&start_seq.to_le_bytes());
    encode_tlv(OP_PTT_START_V2, &p)
}

pub fn encode_ptt_stop_v2(epoch: u64, end_seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&epoch.to_le_bytes());
    p.extend_from_slice(&end_seq.to_le_bytes());
    encode_tlv(OP_PTT_STOP_V2, &p)
}

pub fn new_session_epoch() -> u64 {
    let mut rng = rand::thread_rng();
    loop {
        let v = rng.next_u64();
        if v != 0 { return v; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_byte_round_trips() {
        let bytes = encode_legacy(OP_PTT_START);
        assert_eq!(bytes, vec![0x01u8]);
        let decoded = decode(&bytes);
        assert_eq!(decoded.opcode, OP_PTT_START);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn heartbeat_round_trips() {
        let hb = encode_heartbeat(0xCAFEBABEDEADBEEF, 42, 1_700_000_000_000,
                                  PresenceState::Listening, 18);
        assert_eq!(hb[0], 0x10);
        let decoded = decode(&hb);
        assert_eq!(decoded.opcode, OP_HEARTBEAT);
        let p = parse_heartbeat(&decoded.payload).unwrap();
        assert_eq!(p.seq, 42);
        assert_eq!(p.state, PresenceState::Listening);
        assert_eq!(p.rtt_ms, 18);
        assert_eq!(p.epoch, 0xCAFEBABEDEADBEEF);
    }

    #[test]
    fn recv_ack_round_trips() {
        let bytes = encode_recv_ack(999, 77, 5000);
        let decoded = decode(&bytes);
        assert_eq!(decoded.opcode, OP_RECV_ACK);
        let p = parse_recv_ack(&decoded.payload).unwrap();
        assert_eq!(p.epoch, 999);
        assert_eq!(p.last_seq, 77);
        assert_eq!(p.ts_ms, 5000);
    }

    #[test]
    fn partner_offline_round_trips() {
        let bytes = encode_partner_offline("alice-uuid");
        let decoded = decode(&bytes);
        assert_eq!(decoded.opcode, OP_PARTNER_OFFLINE);
        let len = decoded.payload[0] as usize;
        let id = std::str::from_utf8(&decoded.payload[1..1+len]).unwrap();
        assert_eq!(id, "alice-uuid");
    }

    #[test]
    fn epoch_is_nonzero_and_unique() {
        let a = new_session_epoch();
        let b = new_session_epoch();
        assert_ne!(a, b);
        assert_ne!(a, 0);
    }

    #[test]
    fn empty_bytes_decode_safely() {
        let d = decode(&[]);
        assert_eq!(d.opcode, 0);
        assert!(d.payload.is_empty());
    }
}
