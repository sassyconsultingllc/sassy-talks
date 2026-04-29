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
