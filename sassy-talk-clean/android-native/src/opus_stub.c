// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-3HC6AOF3FXSR
/* opus_stub.c — host-build stubs for the libopus C ABI.
 *
 * Compiled by build.rs ONLY on non-Android/non-Linux hosts (typically
 * Windows MSVC for `cargo test`), where the real libopus sources are
 * not built. The stubs resolve the linker's external references so the
 * test binary can be produced.
 *
 * Calling any of these at runtime is a programming error — the codec
 * module is only exercised on Android targets, and host-side unit
 * tests must not invoke it. If a stub is hit, abort with a clear
 * diagnostic rather than corrupting state.
 */

#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>

static void opus_stub_abort(const char* fn) {
    fprintf(stderr, "FATAL: libopus stub %s called on a non-Android host build.\n", fn);
    fprintf(stderr, "       The codec module is not supported on this target.\n");
    abort();
}

int opus_encoder_get_size(int channels) {
    (void)channels;
    opus_stub_abort("opus_encoder_get_size");
    return -1;
}

void* opus_encoder_create(int Fs, int channels, int application, int* error) {
    (void)Fs; (void)channels; (void)application;
    if (error) *error = -1;
    opus_stub_abort("opus_encoder_create");
    return NULL;
}

int opus_encoder_ctl(void* st, int request, ...) {
    (void)st; (void)request;
    opus_stub_abort("opus_encoder_ctl");
    return -1;
}

void opus_encoder_destroy(void* st) {
    (void)st;
    opus_stub_abort("opus_encoder_destroy");
}

int opus_encode(void* st, const int16_t* pcm, int frame_size,
                uint8_t* data, int max_data_bytes) {
    (void)st; (void)pcm; (void)frame_size; (void)data; (void)max_data_bytes;
    opus_stub_abort("opus_encode");
    return -1;
}

void* opus_decoder_create(int Fs, int channels, int* error) {
    (void)Fs; (void)channels;
    if (error) *error = -1;
    opus_stub_abort("opus_decoder_create");
    return NULL;
}

void opus_decoder_destroy(void* st) {
    (void)st;
    opus_stub_abort("opus_decoder_destroy");
}

int opus_decode(void* st, const uint8_t* data, int len, int16_t* pcm,
                int frame_size, int decode_fec) {
    (void)st; (void)data; (void)len; (void)pcm; (void)frame_size; (void)decode_fec;
    opus_stub_abort("opus_decode");
    return -1;
}

const char* opus_strerror(int error) {
    (void)error;
    return "opus_stub";
}
