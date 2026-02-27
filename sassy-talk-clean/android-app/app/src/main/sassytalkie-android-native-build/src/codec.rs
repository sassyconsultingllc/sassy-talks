#[allow(dead_code)]
pub const CODEC_SAMPLE_RATE: i32 = 48000;
#[allow(dead_code)]
pub const CODEC_CHANNELS: i32 = 1;
pub const CODEC_FRAME_SIZE: usize = 960;
pub const HEADER_SIZE: usize = 4;
const COMPRESSED_FRAME_SIZE: usize = HEADER_SIZE + CODEC_FRAME_SIZE / 2;

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31,
    34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143,
    157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658,
    724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024,
    3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899,
    15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_TABLE: [i32; 16] = [-1,-1,-1,-1,2,4,6,8,-1,-1,-1,-1,2,4,6,8];

fn clamp_step_index(i: i32) -> i32 { i.max(0).min(88) }
fn clamp_sample(s: i32) -> i32 { s.max(-32768).min(32767) }

fn encode_sample(sample: i16, prev: &mut i32, idx: &mut i32) -> u8 {
    let step = STEP_TABLE[*idx as usize];
    let diff = sample as i32 - *prev;
    let mut nibble: u8 = 0;
    let mut delta = step >> 3;
    if diff < 0 { nibble |= 8; }
    let abs_diff = diff.abs();
    if abs_diff >= step { nibble |= 4; delta += step; }
    if abs_diff >= (delta + (step >> 1)) { nibble |= 2; delta += step >> 1; }
    if abs_diff >= (delta + (step >> 2)) { nibble |= 1; delta += step >> 2; }
    if nibble & 8 != 0 { *prev -= delta; } else { *prev += delta; }
    *prev = clamp_sample(*prev);
    *idx = clamp_step_index(*idx + INDEX_TABLE[nibble as usize]);
    nibble
}

fn decode_sample(nibble: u8, prev: &mut i32, idx: &mut i32) -> i16 {
    let step = STEP_TABLE[*idx as usize];
    let mut delta = step >> 3;
    if nibble & 4 != 0 { delta += step; }
    if nibble & 2 != 0 { delta += step >> 1; }
    if nibble & 1 != 0 { delta += step >> 2; }
    if nibble & 8 != 0 { *prev -= delta; } else { *prev += delta; }
    *prev = clamp_sample(*prev);
    *idx = clamp_step_index(*idx + INDEX_TABLE[nibble as usize]);
    *prev as i16
}

pub struct VoiceEncoder { prev_sample: i32, step_index: i32 }
impl VoiceEncoder {
    pub fn new() -> Self { Self { prev_sample: 0, step_index: 0 } }
    #[allow(dead_code)]
    #[allow(dead_code)]
    pub fn reset(&mut self) { self.prev_sample = 0; self.step_index = 0; }
    pub fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
        assert_eq!(pcm.len(), CODEC_FRAME_SIZE);
        let mut out = Vec::with_capacity(COMPRESSED_FRAME_SIZE);
        out.extend_from_slice(&(self.prev_sample as i16).to_le_bytes());
        out.push(self.step_index as u8);
        out.push(0);
        for pair in pcm.chunks_exact(2) {
            let lo = encode_sample(pair[0], &mut self.prev_sample, &mut self.step_index);
            let hi = encode_sample(pair[1], &mut self.prev_sample, &mut self.step_index);
            out.push(lo | (hi << 4));
        }
        out
    }
}
impl Default for VoiceEncoder { fn default() -> Self { Self::new() } }

pub struct VoiceDecoder { prev_sample: i32, step_index: i32 }
impl VoiceDecoder {
    pub fn new() -> Self { Self { prev_sample: 0, step_index: 0 } }
    #[allow(dead_code)]
    pub fn reset(&mut self) { self.prev_sample = 0; self.step_index = 0; }
    pub fn decode(&mut self, compressed: &[u8]) -> Vec<i16> {
        assert_eq!(compressed.len(), COMPRESSED_FRAME_SIZE);
        self.prev_sample = i16::from_le_bytes([compressed[0], compressed[1]]) as i32;
        self.step_index = clamp_step_index(compressed[2] as i32);
        let mut out = Vec::with_capacity(CODEC_FRAME_SIZE);
        for &byte in &compressed[HEADER_SIZE..] {
            out.push(decode_sample(byte & 0x0F, &mut self.prev_sample, &mut self.step_index));
            out.push(decode_sample((byte >> 4) & 0x0F, &mut self.prev_sample, &mut self.step_index));
        }
        out
    }
}
impl Default for VoiceDecoder { fn default() -> Self { Self::new() } }
