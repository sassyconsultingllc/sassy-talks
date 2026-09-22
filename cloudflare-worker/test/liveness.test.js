// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-PNM7QL45MWMC
import { describe, it, expect } from "vitest";
import { buildPartnerOfflineFrame, buildReplayFrame } from "../src/ptt-relay.js";

describe("buildPartnerOfflineFrame", () => {
  it("encodes opcode 0x14 with TLV format", () => {
    const f = buildPartnerOfflineFrame("alice");
    expect(f[0]).toBe(0x14); // OP_PARTNER_OFFLINE
    // Payload = [peer_id_len:1] + "alice" = 6 bytes
    const payloadLen = f[1] | (f[2] << 8);
    expect(payloadLen).toBe(6);
    expect(f[3]).toBe(5); // "alice".length
    const decoded = new TextDecoder().decode(f.slice(4, 4 + 5));
    expect(decoded).toBe("alice");
  });

  it("handles empty peerId", () => {
    const f = buildPartnerOfflineFrame("");
    expect(f[0]).toBe(0x14);
    expect(f[3]).toBe(0);
  });

  it("handles unicode peerId", () => {
    const f = buildPartnerOfflineFrame("user-\u00e9");
    expect(f[0]).toBe(0x14);
    // Should not throw
    expect(f.length).toBeGreaterThan(4);
  });
});

describe("buildReplayFrame", () => {
  it("wraps audio with empty peer_id by default", () => {
    const audio = new Uint8Array([0xAA, 0xBB, 0xCC]);
    const f = buildReplayFrame(audio);
    expect(f[0]).toBe(0x19);
    expect(f[1] | (f[2] << 8)).toBe(0); // peer_id_len
    expect(Array.from(f.slice(3))).toEqual([0xAA, 0xBB, 0xCC]);
  });
});
