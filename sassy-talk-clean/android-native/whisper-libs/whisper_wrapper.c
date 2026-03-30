/// Thin C wrapper around whisper.cpp to avoid exposing the complex
/// whisper_full_params struct to Rust FFI. Compiled by build.rs.

#include <string.h>
#include "include/whisper.h"

/// Initialize a whisper context from a model file (CPU-only, no GPU).
void *whisper_wrapper_init(const char *model_path) {
    struct whisper_context_params cparams = whisper_context_default_params();
    cparams.use_gpu = false;
    cparams.flash_attn = false;
    return (void *)whisper_init_from_file_with_params(model_path, cparams);
}

/// Free a whisper context.
void whisper_wrapper_free(void *ctx) {
    if (ctx) whisper_free((struct whisper_context *)ctx);
}

/// Run inference on 16 kHz float PCM and write the concatenated transcript
/// into `result` (up to `result_size - 1` chars, always NUL-terminated).
///
/// Returns the number of segments decoded, or -1 on error.
int whisper_wrapper_transcribe(
    void       *ctx,
    const float *samples,
    int          n_samples,
    const char  *language,
    int          n_threads,
    char        *result,
    int          result_size
) {
    if (!ctx || !samples || n_samples <= 0 || !result || result_size <= 0)
        return -1;

    struct whisper_full_params params =
        whisper_full_default_params(WHISPER_SAMPLING_GREEDY);

    params.n_threads        = n_threads;
    params.language         = language;
    params.no_timestamps    = true;
    params.single_segment   = false;
    params.print_progress   = false;
    params.print_special    = false;
    params.print_realtime   = false;
    params.print_timestamps = false;
    params.translate        = false;
    params.no_context       = true;
    params.suppress_blank   = true;
    params.suppress_nst     = true;
    params.greedy.best_of   = 1;        /* fastest */

    if (whisper_full((struct whisper_context *)ctx, params, samples, n_samples) != 0)
        return -1;

    int n_segments = whisper_full_n_segments((struct whisper_context *)ctx);
    int offset = 0;
    result[0] = '\0';

    for (int i = 0; i < n_segments; i++) {
        const char *text =
            whisper_full_get_segment_text((struct whisper_context *)ctx, i);
        if (!text) continue;
        int len = (int)strlen(text);
        if (offset + len >= result_size - 1) break;
        memcpy(result + offset, text, len);
        offset += len;
    }
    result[offset] = '\0';
    return n_segments;
}
