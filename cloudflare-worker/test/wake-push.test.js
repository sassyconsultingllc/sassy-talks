// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-WAKEPUSHTEST1
import { describe, it, expect, vi, afterEach } from "vitest";
import { routeWakePush, normalizeWakePlatform } from "../src/wake-push.js";
import { sendApnsWakePush, apnsConfigGaps } from "../src/apns.js";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("normalizeWakePlatform", () => {
  it("defaults unknown / omitted to fcm", () => {
    expect(normalizeWakePlatform(undefined)).toBe("fcm");
    expect(normalizeWakePlatform("fcm")).toBe("fcm");
    expect(normalizeWakePlatform("android")).toBe("fcm");
    expect(normalizeWakePlatform("apns")).toBe("apns");
  });
});

describe("routeWakePush", () => {
  it("does not send an apns token to FCM", async () => {
    const fetches = [];
    vi.stubGlobal("fetch", async (url) => {
      fetches.push(String(url));
      return new Response("{}", { status: 200 });
    });

    // FCM is configured so a mis-route would hit Google; APNs secrets absent
    // so the APNs path skips without calling Apple either.
    const env = {
      FCM_SERVICE_ACCOUNT_JSON: JSON.stringify({
        project_id: "test-proj",
        client_email: "sa@test.iam.gserviceaccount.com",
        private_key: "-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----\n",
      }),
    };

    const r = await routeWakePush(
      env,
      { token: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", platform: "apns" },
      "room-apns-1",
    );

    expect(r.ok).toBe(false);
    expect(r.error).toMatch(/APNs not configured/);
    expect(fetches.some((u) => u.includes("fcm.googleapis.com"))).toBe(false);
    expect(fetches.some((u) => u.includes("oauth2.googleapis.com"))).toBe(false);
  });
});

describe("sendApnsWakePush", () => {
  it("skips when APNs secrets are missing", async () => {
    const fetches = [];
    vi.stubGlobal("fetch", async (url) => {
      fetches.push(String(url));
      return new Response("{}", { status: 200 });
    });

    const env = { APNS_KEY_ID: "ABC123", APNS_TEAM_ID: "TEAMID1" }; // missing key + bundle
    const gaps = apnsConfigGaps(env);
    expect(gaps).toContain("APNS_PRIVATE_KEY");
    expect(gaps).toContain("APNS_BUNDLE_ID");

    const r = await sendApnsWakePush(env, "deadbeef", "room-1");
    expect(r.ok).toBe(false);
    expect(r.stale).toBe(false);
    expect(r.error).toBe(
      "APNs not configured: missing APNS_PRIVATE_KEY, APNS_BUNDLE_ID",
    );
    expect(fetches).toEqual([]);
  });
});
