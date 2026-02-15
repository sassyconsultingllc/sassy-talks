/// Codec Module - IMA ADPCM Audio Compression
///
/// Compresses 16-bit PCM audio frames for low-bandwidth transport.
/// Each 960-sample frame (1920 bytes) compresses to 484 bytes (4:1 ratio).
///
/// Header format (4 bytes):
///   [prev_sample: 2 bytes LE] [step_index: 1 byte] [reserved: 1 byte]
///
/// Uses standard IMA ADPCM with 4-bit encoding per sample.

/// Codec sample rate in Hz.
pub const CODEC_SAMPLE_RATE: i32 = 48000;

/// Number of audio channels (mono).
pub const CODEC_CHANNELS: i32 = 1;

/// Samples per frame (20ms at 48kHz).
pub const CODEC_FRAME_SIZE: usize = 960;

/// Size of the ADPCM frame header in bytes.
pub const HEADER_SIZE: usize = 4;

/// Expected compressed frame size: 4-byte header + 480 bytes of nibble data.
const COMPRESSED_FRAME_SIZE: usize = HEADER_SIZE + CODEC_FRAME_SIZE / 2;

/// Standard IMA ADPCM step size table (89 entries).
const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14,
    16, 17, 19, 21, 23, 25, 28, 31,
    34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143,
    157, 173, 190, 209, 230, 253, 279, 307,
    337, 371, 408, 449, 494, 544, 598, 658,
    724, 796, 876, 963, 1060, 1166, 1282, 1411,
    1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024,
    3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484,
    7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899,
    15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];

/// Standard IMA ADPCM index adjustment table (16 entries).
/// Indices 0-3 reduce the step, indices 4-7 increase it.
/// The table is symmetric: entries 8-15 mirror entries 0-7.
const INDEX_TABLE: [i32; 16] = [
    -1, -1, -1, -1, 2, 4, 6, 8,
    -1, -1, -1, -1, 2, 4, 6, 8,
];

/// Clamp a step index to the valid range [0, 88].
fn clamp_step_index(index: i32) -> i32 {
    if index < 0 {
        0
    } else if index > 88 {
        88
    } else {
        index
    }
}

/// Clamp a sample value to the valid i16 range.
fn clamp_sample(sample: i32) -> i32 {
    if sample < -32768 {
        -32768
    } else if sample > 32767 {
        32767
    } else {
        sample
    }
}

/// Encode a single PCM sample to a 4-bit ADPCM nibble.
/// Updates `prev_sample` and `step_index` in place.
fn encode_sample(sample: i16, prev_sample: &mut i32, step_index: &mut i32) -> u8 {
    let step = STEP_TABLE[*step_index as usize];
    let diff = sample as i32 - *prev_sample;

    let mut nibble: u8 = 0;
    let mut delta = step >> 3;

    if diff < 0 {
        nibble |= 8;
    }

    let abs_diff = diff.abs();

    if abs_diff >= step {
        nibble |= 4;
        delta += step;
    }
    if abs_diff >= (delta + (step >> 1)) {
        nibble |= 2;
        delta += step >> 1;
    }
    if abs_diff >= (delta + (step >> 2)) {
        nibble |= 1;
        delta += step >> 2;
    }

    if nibble & 8 != 0 {
        *prev_sample -= delta;
    } else {
        *prev_sample += delta;
    }
    *prev_sample = clamp_sample(*prev_sample);

    *step_index = clamp_step_index(*step_index + INDEX_TABLE[nibble as usize]);

    nibble
}

/// Decode a 4-bit ADPCM nibble back to a PCM sample.
/// Updates `prev_sample` and `step_index` in place.
fn decode_sample(nibble: u8, prev_sample: &mut i32, step_index: &mut i32) -> i16 {
    let step = STEP_TABLE[*step_index as usize];

    let mut delta = step >> 3;
    if nibble & 4 != 0 {
        delta += step;
    }
    if nibble & 2 != 0 {
        delta += step >> 1;
    }
    if nibble & 1 != 0 {
        delta += step >> 2;
    }

    if nibble & 8 != 0 {
        *prev_sample -= delta;
    } else {
        *prev_sample += delta;
    }
    *prev_sample = clamp_sample(*prev_sample);

    *step_index = clamp_step_index(*step_index + INDEX_TABLE[nibble as usize]);

    *prev_sample as i16
}

/// IMA ADPCM voice encoder.
///
/// Compresses 960-sample PCM frames into 484-byte ADPCM packets.
pub struct VoiceEncoder {
    prev_sample: i32,
    step_index: i32,
}

impl VoiceEncoder {
    /// Create a new encoder with zeroed state.
    pub fn new() -> Self {
        Self {
            prev_sample: 0,
            step_index: 0,
        }
    }

    /// Reset encoder state to initial values.
    pub fn reset(&mut self) {
        self.prev_sample = 0;
        self.step_index = 0;
    }

    /// Encode a PCM frame into an ADPCM packet.
    ///
    /// Input must be exactly `CODEC_FRAME_SIZE` samples (960).
    /// Returns a 484-byte vector: 4-byte header followed by 480 bytes of packed nibbles.
    pub fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
        assert_eq!(
            pcm.len(),
            CODEC_FRAME_SIZE,
            "encode expects exactly {} samples, got {}",
            CODEC_FRAME_SIZE,
            pcm.len()
        );

        let mut output = Vec::with_capacity(COMPRESSED_FRAME_SIZE);

        // Write header: prev_sample (2 bytes LE), step_index (1 byte), reserved (1 byte)
        output.extend_from_slice(&(self.prev_sample as i16).to_le_bytes());
        output.push(self.step_index as u8);
        output.push(0); // reserved

        // Encode samples, packing two nibbles per byte (low nibble first)
        for pair in pcm.chunks_exact(2) {
            let lo = encode_sample(pair[0], &mut self.prev_sample, &mut self.step_index);
            let hi = encode_sample(pair[1], &mut self.prev_sample, &mut self.step_index);
            output.push(lo | (hi << 4));
        }

        output
    }
}

impl Default for VoiceEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// IMA ADPCM voice decoder.
///
/// Decompresses 484-byte ADPCM packets back into 960-sample PCM frames.
pub struct VoiceDecoder {
    prev_sample: i32,
    step_index: i32,
}

impl VoiceDecoder {
    /// Create a new decoder with zeroed state.
    pub fn new() -> Self {
        Self {
            prev_sample: 0,
            step_index: 0,
        }
    }

    /// Reset decoder state to initial values.
    pub fn reset(&mut self) {
        self.prev_sample = 0;
        self.step_index = 0;
    }

    /// Decode an ADPCM packet into a PCM frame.
    ///
    /// Input must be exactly `COMPRESSED_FRAME_SIZE` bytes (484).
    /// Returns a vector of `CODEC_FRAME_SIZE` samples (960).
    pub fn decode(&mut self, compressed: &[u8]) -> Vec<i16> {
        assert_eq!(
            compressed.len(),
            COMPRESSED_FRAME_SIZE,
            "decode expects exactly {} bytes, got {}",
            COMPRESSED_FRAME_SIZE,
            compressed.len()
        );

        // Read header
        self.prev_sample = i16::from_le_bytes([compressed[0], compressed[1]]) as i32;
        self.step_index = clamp_step_index(compressed[2] as i32);
        // compressed[3] is reserved

        let mut output = Vec::with_capacity(CODEC_FRAME_SIZE);

        // Decode packed nibbles (low nibble first)
        for &byte in &compressed[HEADER_SIZE..] {
            let lo = byte & 0x0F;
            let hi = (byte >> 4) & 0x0F;
            output.push(decode_sample(lo, &mut self.prev_sample, &mut self.step_index));
            output.push(decode_sample(hi, &mut self.prev_sample, &mut self.step_index));
        }

        output
    }
}

impl Default for VoiceDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compressed_size() {
        let mut encoder = VoiceEncoder::new();
        let pcm = vec![0i16; CODEC_FRAME_SIZE];
        let compressed = encoder.encode(&pcm);

        assert_eq!(compressed.len(), COMPRESSED_FRAME_SIZE);
        assert_eq!(compressed.len(), 484);
    }

    #[test]
    fn test_roundtrip_silence() {
        let mut encoder = VoiceEncoder::new();
        let mut decoder = VoiceDecoder::new();

        let pcm = vec![0i16; CODEC_FRAME_SIZE];
        let compressed = encoder.encode(&pcm);
        let decoded = decoder.decode(&compressed);

        assert_eq!(decoded.len(), CODEC_FRAME_SIZE);

        // Silence should roundtrip perfectly (all zeros)
        for (i, &sample) in decoded.iter().enumerate() {
            assert_eq!(sample, 0, "sample {} should be 0, got {}", i, sample);
        }
    }

    #[test]
    fn test_roundtrip_sine_wave() {
        let mut encoder = VoiceEncoder::new();
        let mut decoder = VoiceDecoder::new();

        // Generate a 1kHz sine wave at half amplitude
        let amplitude = 16000.0_f64;
        let freq = 1000.0;
        let pcm: Vec<i16> = (0..CODEC_FRAME_SIZE)
            .map(|i| {
                let t = i as f64 / CODEC_SAMPLE_RATE as f64;
                (amplitude * (2.0 * std::f64::consts::PI * freq * t).sin()) as i16
            })
            .collect();

        let compressed = encoder.encode(&pcm);
        let decoded = decoder.decode(&compressed);

        assert_eq!(decoded.len(), CODEC_FRAME_SIZE);

        // Compute signal-to-noise ratio
        let mut signal_power = 0.0_f64;
        let mut noise_power = 0.0_f64;
        for i in 0..CODEC_FRAME_SIZE {
            let original = pcm[i] as f64;
            let reconstructed = decoded[i] as f64;
            signal_power += original * original;
            noise_power += (original - reconstructed) * (original - reconstructed);
        }

        // IMA ADPCM on a clean sine wave should achieve at least 20 dB SNR
        let snr_db = 10.0 * (signal_power / noise_power).log10();
        assert!(
            snr_db > 20.0,
            "SNR too low: {:.1} dB (expected > 20 dB)",
            snr_db
        );
    }

    #[test]
    fn test_roundtrip_max_amplitude() {
        let mut encoder = VoiceEncoder::new();
        let mut decoder = VoiceDecoder::new();

        // Alternating min/max values (worst case for ADPCM)
        let pcm: Vec<i16> = (0..CODEC_FRAME_SIZE)
            .map(|i| if i % 2 == 0 { i16::MAX } else { i16::MIN })
            .collect();

        let compressed = encoder.encode(&pcm);
        let decoded = decoder.decode(&compressed);

        assert_eq!(decoded.len(), CODEC_FRAME_SIZE);

        // Values should still be within i16 range
        for &sample in &decoded {
            assert!(sample >= i16::MIN);
            assert!(sample <= i16::MAX);
        }
    }

    #[test]
    fn test_header_format() {
        let mut encoder = VoiceEncoder::new();

        // Feed a known first sample to set predictor state
        let mut pcm = vec![0i16; CODEC_FRAME_SIZE];
        pcm[0] = 1000;

        let compressed = encoder.encode(&pcm);

        // Header bytes 0-1: prev_sample as i16 LE (initial state was 0)
        let header_sample = i16::from_le_bytes([compressed[0], compressed[1]]);
        assert_eq!(header_sample, 0, "header prev_sample should reflect state before encoding");

        // Header byte 2: step_index (initial state was 0)
        assert_eq!(compressed[2], 0, "header step_index should reflect state before encoding");

        // Header byte 3: reserved
        assert_eq!(compressed[3], 0, "reserved byte should be 0");
    }

    #[test]
    fn test_encoder_reset() {
        let mut encoder = VoiceEncoder::new();

        // Encode a loud frame to move the predictor state
        let loud: Vec<i16> = (0..CODEC_FRAME_SIZE)
            .map(|i| ((i as f64 * 0.1).sin() * 30000.0) as i16)
            .collect();
        let first_loud = encoder.encode(&loud);

        // Reset and encode the same frame again
        encoder.reset();
        let second_loud = encoder.encode(&loud);

        // After reset, output should be identical to the first encoding
        assert_eq!(first_loud, second_loud, "reset encoder should produce identical output");
    }

    #[test]
    fn test_decoder_reset() {
        let mut encoder = VoiceEncoder::new();
        let mut decoder = VoiceDecoder::new();

        let pcm: Vec<i16> = (0..CODEC_FRAME_SIZE)
            .map(|i| ((i as f64 * 0.05).sin() * 20000.0) as i16)
            .collect();

        let compressed = encoder.encode(&pcm);

        // Decode once
        let first_decode = decoder.decode(&compressed);

        // Reset decoder and decode the same data
        decoder.reset();
        let second_decode = decoder.decode(&compressed);

        // Decoder reads state from header, so both decodes should be identical
        assert_eq!(first_decode, second_decode, "reset decoder should produce identical output");
    }

    #[test]
    fn test_multiple_frames_continuity() {
        let mut encoder = VoiceEncoder::new();
        let mut decoder = VoiceDecoder::new();

        // Encode and decode three consecutive frames of a continuous sine wave
        let amplitude = 10000.0_f64;
        let freq = 440.0;

        let mut all_original = Vec::new();
        let mut all_decoded = Vec::new();

        for frame_idx in 0..3 {
            let offset = frame_idx * CODEC_FRAME_SIZE;
            let pcm: Vec<i16> = (0..CODEC_FRAME_SIZE)
                .map(|i| {
                    let t = (offset + i) as f64 / CODEC_SAMPLE_RATE as f64;
                    (amplitude * (2.0 * std::f64::consts::PI * freq * t).sin()) as i16
                })
                .collect();

            let compressed = encoder.encode(&pcm);
            let decoded = decoder.decode(&compressed);

            all_original.extend_from_slice(&pcm);
            all_decoded.extend_from_slice(&decoded);
        }

        // Compute overall SNR across all three frames
        let mut signal_power = 0.0_f64;
        let mut noise_power = 0.0_f64;
        for i in 0..all_original.len() {
            let o = all_original[i] as f64;
            let d = all_decoded[i] as f64;
            signal_power += o * o;
            noise_power += (o - d) * (o - d);
        }

        let snr_db = 10.0 * (signal_power / noise_power).log10();
        assert!(
            snr_db > 20.0,
            "multi-frame SNR too low: {:.1} dB (expected > 20 dB)",
            snr_db
        );
    }

    #[test]
    fn test_constants() {
        assert_eq!(CODEC_SAMPLE_RATE, 48000);
        assert_eq!(CODEC_CHANNELS, 1);
        assert_eq!(CODEC_FRAME_SIZE, 960);
        assert_eq!(HEADER_SIZE, 4);
        assert_eq!(COMPRESSED_FRAME_SIZE, 484);
    }

    #[test]
    fn test_step_table_boundaries() {
        assert_eq!(STEP_TABLE[0], 7);
        assert_eq!(STEP_TABLE[88], 32767);
        assert_eq!(STEP_TABLE.len(), 89);
    }

    #[test]
    fn test_index_table_symmetry() {
        // Entries 0-7 should mirror entries 8-15
        for i in 0..8 {
            assert_eq!(
                INDEX_TABLE[i], INDEX_TABLE[i + 8],
                "index table not symmetric at position {}",
                i
            );
        }
    }

    #[test]
    fn test_clamp_functions() {
        assert_eq!(clamp_step_index(-5), 0);
        assert_eq!(clamp_step_index(0), 0);
        assert_eq!(clamp_step_index(44), 44);
        assert_eq!(clamp_step_index(88), 88);
        assert_eq!(clamp_step_index(100), 88);

        assert_eq!(clamp_sample(-40000), -32768);
        assert_eq!(clamp_sample(-32768), -32768);
        assert_eq!(clamp_sample(0), 0);
        assert_eq!(clamp_sample(32767), 32767);
        assert_eq!(clamp_sample(40000), 32767);
    }

    #[test]
    #[should_panic(expected = "encode expects exactly 960 samples")]
    fn test_encode_wrong_size() {
        let mut encoder = VoiceEncoder::new();
        let short = vec![0i16; 100];
        encoder.encode(&short);
    }

    #[test]
    #[should_panic(expected = "decode expects exactly 484 bytes")]
    fn test_decode_wrong_size() {
        let mut decoder = VoiceDecoder::new();
        let short = vec![0u8; 100];
        decoder.decode(&short);
    }

    #[test]
    fn test_default_trait() {
        let encoder = VoiceEncoder::default();
        let decoder = VoiceDecoder::default();

        assert_eq!(encoder.prev_sample, 0);
        assert_eq!(encoder.step_index, 0);
        assert_eq!(decoder.prev_sample, 0);
        assert_eq!(decoder.step_index, 0);
    }
}
