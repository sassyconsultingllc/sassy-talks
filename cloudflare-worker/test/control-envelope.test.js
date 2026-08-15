// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
import { describe, expect, it } from "vitest";
import { looksLikeTrigger } from "../src/ptt-relay.js";

function envelope(hint, payloadLength = 30) {
  const bytes = new Uint8Array(3 + payloadLength);
  bytes[0] = 0x18;
  bytes[1] = payloadLength & 0xff;
  bytes[2] = payloadLength >> 8;
  bytes[3] = 1;
  bytes[4] = hint;
  return bytes;
}

describe("authenticated control wake hints", () => {
  it("wakes only for authenticated-envelope PTT start and wake hints", () => {
    expect(looksLikeTrigger(envelope(0x15))).toBe(true);
    expect(looksLikeTrigger(envelope(0x17))).toBe(true);
    expect(looksLikeTrigger(envelope(0x10))).toBe(false);
  });

  it("rejects malformed envelope lengths and versions", () => {
    const wrongLength = envelope(0x15);
    wrongLength[1]--;
    expect(looksLikeTrigger(wrongLength)).toBe(false);
    const wrongVersion = envelope(0x15);
    wrongVersion[3] = 2;
    expect(looksLikeTrigger(wrongVersion)).toBe(false);
  });
});
