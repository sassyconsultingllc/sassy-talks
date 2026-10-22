// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-RLYREPLAYT8M
import { describe, it, expect } from "vitest";
import {
  looksLikeTrigger,
  isControlFrameShape,
  selectReplayFrames,
  selectSubprotocol,
  replayCutoff,
  appendToRing,
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

describe("appendToRing", () => {
  const t0 = 1_700_000_000_000;
  const fill = (retainAt) => {
    // 20 s of 20 ms frames, retention decided per append.
    const ring = [];
    let held = 0;
    for (let i = 0; i < 1000; i++) {
      const now = t0 + i * 20;
      held = appendToRing(ring, held, { ts: now, frame: audio(i), from: "a" }, now, retainAt(now));
    }
    return { ring, held, end: t0 + 999 * 20 };
  };

  it("keeps the last 15 s with no catch-up window open (silent drop)", () => {
    // Regression: with no close or wake, nothing was buffered, so a peer that
    // dropped silently and reconnected with since= got an empty replay.
    const { ring, end } = fill(() => 15_000);
    expect(ring.length).toBe(750); // ts in (end - 15 s, end]
    expect(ring[0].ts).toBeGreaterThan(end - 15_000);
    const missed = selectReplayFrames(ring, end - 3_000, end, "me");
    expect(missed.length).toBe(150);
  });

  it("keeps up to the 30 s TTL while a window is open", () => {
    const { ring } = fill(() => 30_000);
    expect(ring.length).toBe(1000);
  });

  it("tracks bytes and enforces the byte cap", () => {
    const ring = [];
    let held = 0;
    const big = new Uint8Array(900 * 1024);
    for (let i = 0; i < 4; i++) {
      held = appendToRing(ring, held, { ts: t0 + i, frame: big, from: "a" }, t0 + i, 15_000);
    }
    expect(held).toBe(ring.reduce((n, e) => n + e.frame.length, 0));
    expect(held).toBeLessThanOrEqual(2 * 1024 * 1024);
    expect(ring.length).toBe(2);
  });
});

describe("replayCutoff", () => {
  const q = (s) => new URLSearchParams(s);

  it("ignores a bare catchup=1 from 3.2.7 clients, which cannot unwrap replays", () => {
    // These clients send catchup=1 on every reconnect; the replay reached
    // their AEAD as garbage and was logged as a key mismatch.
    expect(replayCutoff(q("catchup=1"))).toBe(null);
    expect(replayCutoff(q("room=r&peer=p&catchup=1"))).toBe(null);
  });

  it("replays from a since= cursor", () => {
    expect(replayCutoff(q("since=1700000095000"))).toBe(1700000095000);
    expect(replayCutoff(q("catchup=1&since=1700000095000"))).toBe(1700000095000);
  });

  it("treats an empty or junk since= as no replay, not the whole window", () => {
    expect(replayCutoff(q("since="))).toBe(null);
    expect(replayCutoff(q("since=%20"))).toBe(null);
    expect(replayCutoff(q("since=abc"))).toBe(null);
    expect(replayCutoff(q("since=0"))).toBe(null);
    expect(replayCutoff(q("since=-5"))).toBe(null);
    expect(replayCutoff(q(""))).toBe(null);
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
