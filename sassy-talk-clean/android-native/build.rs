/// build.rs — compile libopus from source using the cc crate.
///
/// This bypasses audiopus_sys's cmake build (which is unreliable for
/// Android cross-compilation on Windows). The cc crate works out of the
/// box with cargo-ndk because cargo-ndk sets CC, AR, CFLAGS, etc.
///
/// The opus source is taken from the audiopus_sys crate's bundled copy
/// in the cargo registry. Set OPUS_SRC_DIR to override the lookup.

use std::env;
use std::path::PathBuf;

fn find_opus_src() -> PathBuf {
    // Explicit override wins
    if let Ok(p) = env::var("OPUS_SRC_DIR") {
        let path = PathBuf::from(&p);
        if path.exists() {
            return path;
        }
        panic!("OPUS_SRC_DIR={} does not exist", p);
    }

    // Search the cargo registry for audiopus_sys's bundled opus sources
    let cargo_home = env::var("CARGO_HOME")
        .or_else(|_| env::var("USERPROFILE").map(|p| format!("{}/.cargo", p)))
        .or_else(|_| env::var("HOME").map(|p| format!("{}/.cargo", p)))
        .expect("Cannot determine CARGO_HOME");

    let registry_src = PathBuf::from(&cargo_home).join("registry").join("src");
    if !registry_src.exists() {
        panic!("Registry src not found at {:?}", registry_src);
    }

    // Iterate all registry index directories (one per source URL)
    for entry in std::fs::read_dir(&registry_src).expect("read registry/src") {
        let e = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let candidate = e.path().join("audiopus_sys-0.2.2").join("opus");
        if candidate.join("include").join("opus.h").exists() {
            return candidate;
        }
    }

    panic!(
        "Cannot find libopus source from audiopus_sys-0.2.2 in {:?}. \
         Add audiopus_sys = \"0.2.2\" as a dependency or set OPUS_SRC_DIR.",
        registry_src
    )
}

fn main() {
    let target = env::var("TARGET").unwrap_or_default();

    // Only compile libopus when targeting Android or Linux.
    // On Windows host builds (e.g. unit tests that run natively) we skip this.
    let is_android = target.contains("android");
    let is_linux   = target.contains("linux");
    if !is_android && !is_linux {
        return;
    }

    let opus = find_opus_src();

    // ── Source file lists (float-point build, base C only — no NEON/SSE) ──

    let celt_sources = [
        "celt/bands.c",
        "celt/celt.c",
        "celt/celt_encoder.c",
        "celt/celt_decoder.c",
        "celt/cwrs.c",
        "celt/entcode.c",
        "celt/entdec.c",
        "celt/entenc.c",
        "celt/kiss_fft.c",
        "celt/laplace.c",
        "celt/mathops.c",
        "celt/mdct.c",
        "celt/modes.c",
        "celt/pitch.c",
        "celt/celt_lpc.c",
        "celt/quant_bands.c",
        "celt/rate.c",
        "celt/vq.c",
    ];

    let silk_sources = [
        "silk/CNG.c",
        "silk/code_signs.c",
        "silk/init_decoder.c",
        "silk/decode_core.c",
        "silk/decode_frame.c",
        "silk/decode_parameters.c",
        "silk/decode_indices.c",
        "silk/decode_pulses.c",
        "silk/decoder_set_fs.c",
        "silk/dec_API.c",
        "silk/enc_API.c",
        "silk/encode_indices.c",
        "silk/encode_pulses.c",
        "silk/gain_quant.c",
        "silk/interpolate.c",
        "silk/LP_variable_cutoff.c",
        "silk/NLSF_decode.c",
        "silk/NSQ.c",
        "silk/NSQ_del_dec.c",
        "silk/PLC.c",
        "silk/shell_coder.c",
        "silk/tables_gain.c",
        "silk/tables_LTP.c",
        "silk/tables_NLSF_CB_NB_MB.c",
        "silk/tables_NLSF_CB_WB.c",
        "silk/tables_other.c",
        "silk/tables_pitch_lag.c",
        "silk/tables_pulses_per_block.c",
        "silk/VAD.c",
        "silk/control_audio_bandwidth.c",
        "silk/quant_LTP_gains.c",
        "silk/VQ_WMat_EC.c",
        "silk/HP_variable_cutoff.c",
        "silk/NLSF_encode.c",
        "silk/NLSF_VQ.c",
        "silk/NLSF_unpack.c",
        "silk/NLSF_del_dec_quant.c",
        "silk/process_NLSFs.c",
        "silk/stereo_LR_to_MS.c",
        "silk/stereo_MS_to_LR.c",
        "silk/check_control_input.c",
        "silk/control_SNR.c",
        "silk/init_encoder.c",
        "silk/control_codec.c",
        "silk/A2NLSF.c",
        "silk/ana_filt_bank_1.c",
        "silk/biquad_alt.c",
        "silk/bwexpander_32.c",
        "silk/bwexpander.c",
        "silk/debug.c",
        "silk/decode_pitch.c",
        "silk/inner_prod_aligned.c",
        "silk/lin2log.c",
        "silk/log2lin.c",
        "silk/LPC_analysis_filter.c",
        "silk/LPC_inv_pred_gain.c",
        "silk/table_LSF_cos.c",
        "silk/NLSF2A.c",
        "silk/NLSF_stabilize.c",
        "silk/NLSF_VQ_weights_laroia.c",
        "silk/pitch_est_tables.c",
        "silk/resampler.c",
        "silk/resampler_down2_3.c",
        "silk/resampler_down2.c",
        "silk/resampler_private_AR2.c",
        "silk/resampler_private_down_FIR.c",
        "silk/resampler_private_IIR_FIR.c",
        "silk/resampler_private_up2_HQ.c",
        "silk/resampler_rom.c",
        "silk/sigm_Q15.c",
        "silk/sort.c",
        "silk/sum_sqr_shift.c",
        "silk/stereo_decode_pred.c",
        "silk/stereo_encode_pred.c",
        "silk/stereo_find_predictor.c",
        "silk/stereo_quant_pred.c",
        "silk/LPC_fit.c",
    ];

    let silk_float_sources = [
        "silk/float/apply_sine_window_FLP.c",
        "silk/float/corrMatrix_FLP.c",
        "silk/float/encode_frame_FLP.c",
        "silk/float/find_LPC_FLP.c",
        "silk/float/find_LTP_FLP.c",
        "silk/float/find_pitch_lags_FLP.c",
        "silk/float/find_pred_coefs_FLP.c",
        "silk/float/LPC_analysis_filter_FLP.c",
        "silk/float/LTP_analysis_filter_FLP.c",
        "silk/float/LTP_scale_ctrl_FLP.c",
        "silk/float/noise_shape_analysis_FLP.c",
        "silk/float/process_gains_FLP.c",
        "silk/float/regularize_correlations_FLP.c",
        "silk/float/residual_energy_FLP.c",
        "silk/float/warped_autocorrelation_FLP.c",
        "silk/float/wrappers_FLP.c",
        "silk/float/autocorrelation_FLP.c",
        "silk/float/burg_modified_FLP.c",
        "silk/float/bwexpander_FLP.c",
        "silk/float/energy_FLP.c",
        "silk/float/inner_product_FLP.c",
        "silk/float/k2a_FLP.c",
        "silk/float/LPC_inv_pred_gain_FLP.c",
        "silk/float/pitch_analysis_core_FLP.c",
        "silk/float/scale_copy_vector_FLP.c",
        "silk/float/scale_vector_FLP.c",
        "silk/float/schur_FLP.c",
        "silk/float/sort_FLP.c",
    ];

    let opus_sources = [
        "src/opus.c",
        "src/opus_decoder.c",
        "src/opus_encoder.c",
        "src/opus_multistream.c",
        "src/opus_multistream_encoder.c",
        "src/opus_multistream_decoder.c",
        "src/repacketizer.c",
        "src/opus_projection_encoder.c",
        "src/opus_projection_decoder.c",
        "src/mapping_matrix.c",
        // Float analysis (improves quality; only meaningful for float builds)
        "src/analysis.c",
        "src/mlp.c",
        "src/mlp_data.c",
    ];

    // ── Build ──

    let mut build = cc::Build::new();

    build
        .include(opus.join("include"))
        .include(opus.join("celt"))
        .include(opus.join("silk"))
        .include(opus.join("silk").join("float"))
        // Core defines for a float-point build
        .define("OPUS_BUILD", None)
        .define("USE_ALLOCA", None)
        .define("HAVE_LRINT", None)
        .define("HAVE_LRINTF", None)
        .define("HAVE_STDINT_H", None)
        .define("HAVE_INTTYPES_H", None)
        .define("HAVE_ALLOCA_H", None)
        // Disable optional features that need extra headers / platform work
        .define("ENABLE_HARDENING", "0")
        // Silence all warnings so the build log stays clean
        .flag("-w");

    // Architecture-specific ARM NEON handling.
    // IMPORTANT: `defined(OPUS_ARM_PRESUME_AARCH64_NEON_INTR)` in silk/macros.h
    // is a presence check — defining it as "0" still makes `defined(...)` true,
    // which causes arm_neon.h to be pulled in for x86_64 builds (→ build failure).
    // Solution: only DEFINE it for aarch64; for all other arches, undefine ARM
    // detection macros so silk/macros.h never includes silk/arm/macros_arm64.h.
    if target.contains("aarch64") {
        build.define("OPUS_ARM_PRESUME_AARCH64_NEON_INTR", "1");
    } else {
        // Undefine ARM architecture macros that could trigger NEON includes
        build.flag("-U__ARM_ARCH_ISA_A64");
        build.flag("-U__aarch64__");
        build.flag("-U__ARM_NEON__");
        build.flag("-U__ARM_NEON");
    }

    for src in celt_sources.iter()
        .chain(silk_sources.iter())
        .chain(silk_float_sources.iter())
        .chain(opus_sources.iter())
    {
        build.file(opus.join(src));
    }

    build.compile("opus");

    // Tell cargo where to find the compiled library (handled by cc::Build::compile)
    println!("cargo:rustc-link-lib=static=opus");

    // Whisper.cpp libraries removed — transcription stripped to slim down binary size.
}
