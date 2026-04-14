#![allow(non_camel_case_types)]

/// Minimal raw FFI bindings to libopus.
///
/// libopus is compiled from source by build.rs (cc crate, no cmake needed).
/// Only the functions actually used by VoiceEncoder/VoiceDecoder are declared.

// Opaque handle types
#[repr(C)]
pub struct OpusEncoder(u8);
#[repr(C)]
pub struct OpusDecoder(u8);

pub const OPUS_OK: i32 = 0;
pub const OPUS_APPLICATION_VOIP: i32 = 2048;
pub const OPUS_SET_BITRATE_REQUEST: i32 = 4002;
pub const OPUS_SET_VBR_REQUEST: i32 = 4006;
pub const OPUS_SET_COMPLEXITY_REQUEST: i32 = 4010;
pub const OPUS_SET_INBAND_FEC_REQUEST: i32 = 4012;
pub const OPUS_SET_PACKET_LOSS_PERC_REQUEST: i32 = 4014;

extern "C" {
    // ── Encoder ──

    pub fn opus_encoder_get_size(channels: i32) -> i32;

    pub fn opus_encoder_create(
        Fs: i32,
        channels: i32,
        application: i32,
        error: *mut i32,
    ) -> *mut OpusEncoder;

    pub fn opus_encoder_ctl(st: *mut OpusEncoder, request: i32, ...) -> i32;

    pub fn opus_encoder_destroy(st: *mut OpusEncoder);

    pub fn opus_encode(
        st: *mut OpusEncoder,
        pcm: *const i16,
        frame_size: i32,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;

    // ── Decoder ──

    pub fn opus_decoder_create(Fs: i32, channels: i32, error: *mut i32) -> *mut OpusDecoder;

    pub fn opus_decoder_destroy(st: *mut OpusDecoder);

    pub fn opus_decode(
        st: *mut OpusDecoder,
        data: *const u8,
        len: i32,
        pcm: *mut i16,
        frame_size: i32,
        decode_fec: i32,
    ) -> i32;

    // ── Error strings (useful for debug logging) ──
    pub fn opus_strerror(error: i32) -> *const std::os::raw::c_char;
}
