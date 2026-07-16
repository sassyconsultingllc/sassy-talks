// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-IH5YXNEALMVH
#pragma once

#include "ggml.h"
#include "ggml-backend.h"


#ifdef  __cplusplus
extern "C" {
#endif

// backend API
GGML_BACKEND_API ggml_backend_t ggml_backend_blas_init(void);

GGML_BACKEND_API bool ggml_backend_is_blas(ggml_backend_t backend);

// number of threads used for conversion to float
// for openblas and blis, this will also set the number of threads used for blas operations
GGML_BACKEND_API void ggml_backend_blas_set_n_threads(ggml_backend_t backend_blas, int n_threads);

GGML_BACKEND_API ggml_backend_reg_t ggml_backend_blas_reg(void);


#ifdef  __cplusplus
}
#endif
