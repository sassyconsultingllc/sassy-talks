/// Codec Module - Opus Audio Compression
///
/// Compresses 16-bit PCM audio using the Opus codec (compiled from source via
/// build.rs / cc crate). Each 960-sample frame (20ms at 48kHz) typically
/// compresses to 20–80 bytes at 24kbps VoIP quality.
///
/// libopus is compiled from the audiopus_sys-bundled sources in build.rs;
/// the FFI declarations live in opus_ffi.rs.

use crate::opus_ffi as ffi;

/// Codec sample rate in Hz.
pub const CODEC_SAMPLE_RATE: i32 = 48000;

/// Number of audio channels (mono).
pub const CODEC_CHANNELS: i32 = 1;

/// Samples per frame (20ms at 48kHz). Standard Opus frame size.
pub const CODEC_FRAME_SIZE: usize = 960;

/// Maximum Opus packet size for allocation. Real voice frames: 20-80 bytes.
pub const COMPRESSED_FRAME_SIZE: usize = 256;

/// Voice bitrate for Opus (bps). 32kbps gives better quality margin under
/// network stress while staying low-bandwidth for relay transport.
const VOICE_BITRATE: i32 = 32000;

// ── Helper ──

fn check_opus_error(code: i32, context: &str) {
    if code < ffi::OPUS_OK {
        let msg = unsafe {
            let ptr = ffi::opus_strerror(code);
            if ptr.is_null() { "unknown error".to_string() }
            else { std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned() }
        };
        log::warn!("Opus {} error {}: {}", context, code, msg);
    }
}

// ── Encoder ──

/// Opus voice encoder.
///
/// Encodes 960-sample PCM frames into variable-length Opus packets.
pub struct VoiceEncoder {
    enc: *mut ffi::OpusEncoder,
}

// Safety: OpusEncoder is not thread-safe; we always access it under a Mutex in the pipeline.
unsafe impl Send for VoiceEncoder {}

impl VoiceEncoder {
    /// Create a new encoder configured for VoIP at 24kbps.
    pub fn new() -> Self {
        let mut err = 0i32;
        let enc = unsafe {
            ffi::opus_encoder_create(CODEC_SAMPLE_RATE, CODEC_CHANNELS, ffi::OPUS_APPLICATION_VOIP, &mut err)
        };
        if enc.is_null() || err != ffi::OPUS_OK {
            panic!("opus_encoder_create failed: {}", err);
        }
        // Set 24kbps bitrate
        let ret = unsafe { ffi::opus_encoder_ctl(enc, ffi::OPUS_SET_BITRATE_REQUEST, VOICE_BITRATE) };
        check_opus_error(ret, "encoder set_bitrate");
        // Complexity 5 (balanced quality vs CPU)
        let ret = unsafe { ffi::opus_encoder_ctl(enc, ffi::OPUS_SET_COMPLEXITY_REQUEST, 5i32) };
        check_opus_error(ret, "encoder set_complexity");
        // Enable in-band FEC so the decoder can recover lost packets
        let ret = unsafe { ffi::opus_encoder_ctl(enc, ffi::OPUS_SET_INBAND_FEC_REQUEST, 1i32) };
        check_opus_error(ret, "encoder set_inband_fec");
        // Tell encoder to expect ~10% packet loss (tunes FEC overhead)
        let ret = unsafe { ffi::opus_encoder_ctl(enc, ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, 10i32) };
        check_opus_error(ret, "encoder set_packet_loss_perc");
        Self { enc }
    }

    /// Reset encoder by reinitialising (called on PTT release to start clean).
    pub fn reset(&mut self) {
        let mut err = 0i32;
        let new_enc = unsafe {
            ffi::opus_encoder_create(CODEC_SAMPLE_RATE, CODEC_CHANNELS, ffi::OPUS_APPLICATION_VOIP, &mut err)
        };
        if !new_enc.is_null() && err == ffi::OPUS_OK {
            unsafe { ffi::opus_encoder_destroy(self.enc) };
            self.enc = new_enc;
            unsafe { ffi::opus_encoder_ctl(new_enc, ffi::OPUS_SET_BITRATE_REQUEST, VOICE_BITRATE); }
            unsafe { ffi::opus_encoder_ctl(new_enc, ffi::OPUS_SET_COMPLEXITY_REQUEST, 5i32); }
            unsafe { ffi::opus_encoder_ctl(new_enc, ffi::OPUS_SET_INBAND_FEC_REQUEST, 1i32); }
            unsafe { ffi::opus_encoder_ctl(new_enc, ffi::OPUS_SET_PACKET_LOSS_PERC_REQUEST, 10i32); }
        }
    }

    /// Encode a PCM frame into an Opus packet.
    ///
    /// `pcm` must be exactly `CODEC_FRAME_SIZE` samples (960).
    /// Returns a variable-length Vec (typically 20–80 bytes for voice).
    /// Returns empty Vec on encoding failure.
    pub fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
        debug_assert_eq!(pcm.len(), CODEC_FRAME_SIZE);
        let mut buf = vec![0u8; COMPRESSED_FRAME_SIZE];
        let n = unsafe {
            ffi::opus_encode(
                self.enc,
                pcm.as_ptr(),
                CODEC_FRAME_SIZE as i32,
                buf.as_mut_ptr(),
                COMPRESSED_FRAME_SIZE as i32,
            )
        };
        if n <= 0 {
            check_opus_error(n, "encode");
            return Vec::new();
        }
        buf.truncate(n as usize);
        buf
    }
}

impl Default for VoiceEncoder {
    fn default() -> Self { Self::new() }
}

impl Drop for VoiceEncoder {
    fn drop(&mut self) {
        if !self.enc.is_null() {
            unsafe { ffi::opus_encoder_destroy(self.enc) };
        }
    }
}

// ── Decoder ──

/// Opus voice decoder.
///
/// Decodes variable-length Opus packets back into 960-sample PCM frames.
pub struct VoiceDecoder {
    dec: *mut ffi::OpusDecoder,
}

unsafe impl Send for VoiceDecoder {}

impl VoiceDecoder {
    /// Create a new decoder.
    pub fn new() -> Self {
        let mut err = 0i32;
        let dec = unsafe {
            ffi::opus_decoder_create(CODEC_SAMPLE_RATE, CODEC_CHANNELS, &mut err)
        };
        if dec.is_null() || err != ffi::OPUS_OK {
            panic!("opus_decoder_create failed: {}", err);
        }
        Self { dec }
    }

    /// Reset decoder state.
    pub fn reset(&mut self) {
        let mut err = 0i32;
        let new_dec = unsafe { ffi::opus_decoder_create(CODEC_SAMPLE_RATE, CODEC_CHANNELS, &mut err) };
        if !new_dec.is_null() && err == ffi::OPUS_OK {
            unsafe { ffi::opus_decoder_destroy(self.dec) };
            self.dec = new_dec;
        }
    }

    /// Decode an Opus packet into PCM (normal decode, no FEC).
    ///
    /// `decode_fec=1` is intentionally NOT set here: that flag tells the
    /// decoder to emit the FEC-reconstructed *previous* frame instead of
    /// the current frame, which makes sense only when the previous frame
    /// was actually lost — not as a routine option on every packet. The
    /// previous unconditional `decode_fec=1` was an API misuse that
    /// produced unexpected pitch/timing artifacts on lossy paths. See
    /// `decode_fec_from_next` for the correct lost-frame repair path.
    pub fn decode(&mut self, compressed: &[u8]) -> Vec<i16> {
        if compressed.is_empty() {
            log::warn!("Opus decode: empty packet, returning silence frame");
            return vec![0i16; CODEC_FRAME_SIZE];
        }
        let mut buf = vec![0i16; CODEC_FRAME_SIZE];
        let n = unsafe {
            ffi::opus_decode(
                self.dec,
                compressed.as_ptr(),
                compressed.len() as i32,
                buf.as_mut_ptr(),
                CODEC_FRAME_SIZE as i32,
                0, // normal decode — FEC repair is via decode_fec_from_next
            )
        };
        if n <= 0 {
            log::error!("Opus decode FAILED (code={}, {} input bytes) — returning silence", n, compressed.len());
            check_opus_error(n, "decode");
            return vec![0i16; CODEC_FRAME_SIZE];
        }
        buf.truncate(n as usize);
        buf
    }

    /// Reconstruct a single missing previous frame from the in-band FEC
    /// data embedded in `next_packet`. Call this BEFORE decoding the
    /// successor packet, once per detected sequence-number gap.
    ///
    /// Per Opus spec, the encoder's `setInbandFec(true)` mode places a
    /// down-quality replica of frame N-1 inside frame N's bitstream;
    /// `opus_decode(decoder, next_packet, len, out, frame_size, 1)`
    /// extracts that replica. We never call FEC for a frame whose
    /// successor hasn't arrived — if both N-1 and N are lost, we use
    /// `decode_plc` instead.
    pub fn decode_fec_from_next(&mut self, next_packet: &[u8]) -> Vec<i16> {
        if next_packet.is_empty() {
            return self.decode_plc();
        }
        let mut buf = vec![0i16; CODEC_FRAME_SIZE];
        let n = unsafe {
            ffi::opus_decode(
                self.dec,
                next_packet.as_ptr(),
                next_packet.len() as i32,
                buf.as_mut_ptr(),
                CODEC_FRAME_SIZE as i32,
                1, // request FEC reconstruction of the prior frame
            )
        };
        if n <= 0 {
            log::warn!("Opus decode_fec: code={} from {}-byte packet — falling back to PLC", n, next_packet.len());
            return self.decode_plc();
        }
        buf.truncate(n as usize);
        buf
    }

    /// Pure packet-loss concealment — generate a plausible replacement
    /// frame when there is neither a real packet nor a successor with
    /// FEC. `opus_decode(NULL, 0, ...)` runs Opus's built-in PLC, which
    /// extrapolates from the decoder's recent history and fades to silence
    /// over a few frames.
    pub fn decode_plc(&mut self) -> Vec<i16> {
        let mut buf = vec![0i16; CODEC_FRAME_SIZE];
        let n = unsafe {
            ffi::opus_decode(
                self.dec,
                std::ptr::null(),
                0,
                buf.as_mut_ptr(),
                CODEC_FRAME_SIZE as i32,
                0,
            )
        };
        if n <= 0 {
            return vec![0i16; CODEC_FRAME_SIZE];
        }
        buf.truncate(n as usize);
        buf
    }
}

impl Default for VoiceDecoder {
    fn default() -> Self { Self::new() }
}

impl Drop for VoiceDecoder {
    fn drop(&mut self) {
        if !self.dec.is_null() {
            unsafe { ffi::opus_decoder_destroy(self.dec) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(CODEC_SAMPLE_RATE, 48000);
        assert_eq!(CODEC_CHANNELS, 1);
        assert_eq!(CODEC_FRAME_SIZE, 960);
    }
}
