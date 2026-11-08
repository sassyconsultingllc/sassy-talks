// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RLYREPLAYT8M
import { describe, it, expect } from "vitest";
import {
  looksLikeTrigger,
  isControlFrameShape,
  selectReplayFrames,
  selectSubprotocol,
} from "../src/ptt-relay.js";

const tlv = (op, len) => {
  const out = new Uint8Array(3 + len);
  out[0] = op;
  out[1] = len & 0xff;
  out[2] = (len >> 8) & 0xff;
  return out;
};
// Sealed audio: random-looking nonce first, not TLV-exact.
const audio = (seed) => Uint8Array.from({ length: 60 }, (_, i) => (seed * 31 + i * 7 + 0x41) & 0xff);

describe("looksLikeTrigger", () => {
  it("wakes on both the legacy 12-byte and the emergency-aware 13-byte PTT_START_V2", () => {
    expect(looksLikeTrigger(tlv(0x15, 12))).toBe(true);
    expect(looksLikeTrigger(tlv(0x15, 13))).toBe(true);
    expect(looksLikeTrigger(tlv(0x15, 14))).toBe(false);
    expect(looksLikeTrigger(tlv(0x17, 16))).toBe(true);
    expect(looksLikeTrigger(tlv(0x17, 13))).toBe(false);
  });
});

describe("isControlFrameShape", () => {
  it("matches exact-length TLV control and rejects audio", () => {
    expect(isControlFrameShape(tlv(0x18, 40))).toBe(true);
    expect(isControlFrameShape(tlv(0x10, 24))).toBe(true);
    expect(isControlFrameShape(audio(1))).toBe(false);
    expect(isControlFrameShape(new Uint8Array([0x10, 0]))).toBe(false);
  });
});

describe("selectReplayFrames", () => {
  const now = 1_700_000_100_000;
  const buf = [
    { ts: now - 40_000, frame: audio(1), from: "a" }, // older than TTL
    { ts: now - 5_000, frame: audio(2), from: "a" },
    { ts: now - 4_000, frame: tlv(0x18, 40), from: "a" }, // sealed control
    { ts: now - 3_000, frame: audio(3), from: "me" }, // requester's own
    { ts: now - 1_000, frame: audio(4), from: "b" },
  ];

  it("replays only the missed audio from other peers", () => {
    const got = selectReplayFrames(buf, now - 10_000, now, "me");
    expect(got.map((e) => e.frame)).toEqual([audio(2), audio(4)]);
  });

  it("honours a since= cursor and clamps catchup=1 to the TTL", () => {
    expect(selectReplayFrames(buf, now - 2_000, now, "me").map((e) => e.frame)).toEqual([audio(4)]);
    expect(selectReplayFrames(buf, 0, now, "").length).toBe(3);
  });
});

describe("selectSubprotocol", () => {
  it("echoes the offered sassytalk protocol so RFC 6455 clients complete the handshake", () => {
    expect(selectSubprotocol(null)).toBe(null);
    expect(selectSubprotocol("chat, superchat")).toBe(null);
    expect(selectSubprotocol("sassytalk.abc123.def")).toBe("sassytalk.abc123.def");
    expect(selectSubprotocol("chat, sassytalk")).toBe("sassytalk");
  });
});
