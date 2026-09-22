#![allow(non_camel_case_types)]
// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-UBRHRDMGV2I5

/// Minimal FFI bindings to the whisper_wrapper.c shim.
///
/// The wrapper hides the complex whisper_full_params struct behind a
/// simple C function interface. Pre-built static libraries (libwhisper.a,
/// libggml*.a) live in whisper-libs/{arm64-v8a,x86_64}/ and are linked
/// by build.rs.

use std::os::raw::{c_char, c_float, c_int, c_void};

extern "C" {
    /// Load a whisper model from disk. Returns an opaque context pointer,
    /// or null on failure. CPU-only, no GPU.
    pub fn whisper_wrapper_init(model_path: *const c_char) -> *mut c_void;

    /// Free a context returned by whisper_wrapper_init.
    pub fn whisper_wrapper_free(ctx: *mut c_void);

    /// Run greedy-decode inference on 16 kHz mono f32 PCM.
    ///
    /// * `ctx`         – context from whisper_wrapper_init
    /// * `samples`     – 16 kHz float PCM [-1.0, 1.0]
    /// * `n_samples`   – number of samples
    /// * `language`    – e.g. "en" (or "auto")
    /// * `n_threads`   – CPU threads for inference
    /// * `result`      – output buffer for UTF-8 text
    /// * `result_size` – capacity of result buffer
    ///
    /// Returns number of segments decoded, or -1 on error.
    pub fn whisper_wrapper_transcribe(
        ctx: *mut c_void,
        samples: *const c_float,
        n_samples: c_int,
        language: *const c_char,
        n_threads: c_int,
        result: *mut c_char,
        result_size: c_int,
    ) -> c_int;
}
