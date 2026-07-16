// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-ZHMSLM6YQ2OS
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
  },
  resolve: {
    alias: {
      // Stub out cloudflare:workers — the tests only exercise pure JS exports
      "cloudflare:workers": new URL("./test/__mocks__/cloudflare-workers.js", import.meta.url).pathname,
    },
  },
});
