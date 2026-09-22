// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-PTWK2AK5UCHA
#pragma once

#include "ggml.h"
#include "ggml-backend.h"

#ifdef  __cplusplus
extern "C" {
#endif

#define GGML_WEBGPU_NAME "WebGPU"

// Needed for examples in ggml
GGML_BACKEND_API ggml_backend_t ggml_backend_webgpu_init(void);

GGML_BACKEND_API ggml_backend_reg_t ggml_backend_webgpu_reg(void);

#ifdef  __cplusplus
}
#endif
