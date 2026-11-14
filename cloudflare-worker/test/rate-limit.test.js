// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
import { describe, it, expect, vi, afterEach } from "vitest";
import { PttRoom } from "../src/ptt-relay.js";

function sock(att) {
  return {
    att,
    sent: [],
    closeCode: null,
    deserializeAttachment() { return this.att; },
    serializeAttachment(a) { this.att = a; },
    close(code) { this.closeCode = code; },
    send(f) { this.sent.push(f); },
  };
}

function room(sockets) {
  const r = Object.create(PttRoom.prototype);
  r.sessions = new Map();
  r.sendFailCounts = new Map();
  r.droppedRateLimited = 0;
  r.droppedFanout = 0;
  r.audioBuffer = [];
  r.audioBufferBytes = 0;
  r.alarmArmed = true;
  r.ctx = {
    getWebSockets: () => sockets,
    storage: { setAlarm: async () => {} },
    waitUntil: () => {},
  };
  return r;
}

// A sealed audio frame is opaque to the relay; any small binary blob will do.
const frame = () => new Uint8Array(200).buffer;

afterEach(() => vi.useRealTimers());

describe("per-socket rate limit", () => {
  it("delivers a post-stall flush of 3 s of audio in one burst without dropping", async () => {
    vi.useFakeTimers();
    const sender = sock({ peer: "cell", lastSeenMs: Date.now() });
    const other = sock({ peer: "wifi", lastSeenMs: Date.now() });
    const r = room([sender, other]);
    // 150 frames inside one millisecond: what a cellular uplink does after
    // stalling for three seconds. The old fixed window kept only 120.
    for (let i = 0; i < 150; i++) await r.webSocketMessage(sender, frame());
    expect(other.sent.length).toBe(150);
    expect(r.droppedRateLimited).toBe(0);
    expect(sender.closeCode).toBe(null);
  });

  it("still soft-drops a sender that sustains more than the allowed rate", async () => {
    vi.useFakeTimers();
    const sender = sock({ peer: "a", lastSeenMs: Date.now() });
    const other = sock({ peer: "b", lastSeenMs: Date.now() });
    const r = room([sender, other]);
    // 700 frames with no time passing: 600 burst tokens, then drops.
    for (let i = 0; i < 700; i++) await r.webSocketMessage(sender, frame());
    expect(other.sent.length).toBe(600);
    expect(r.droppedRateLimited).toBe(100);
    expect(sender.closeCode).toBe(null);
  });

  it("refills at the sustained rate", async () => {
    vi.useFakeTimers();
    const sender = sock({ peer: "a", lastSeenMs: Date.now() });
    const other = sock({ peer: "b", lastSeenMs: Date.now() });
    const r = room([sender, other]);
    for (let i = 0; i < 600; i++) await r.webSocketMessage(sender, frame());
    vi.advanceTimersByTime(1000); // +120 tokens
    for (let i = 0; i < 200; i++) await r.webSocketMessage(sender, frame());
    expect(other.sent.length).toBe(720);
  });

  it("hard-closes a flood that keeps going with the bucket empty", async () => {
    vi.useFakeTimers();
    const sender = sock({ peer: "a", lastSeenMs: Date.now() });
    const r = room([sender, sock({ peer: "b", lastSeenMs: Date.now() })]);
    for (let i = 0; i < 1000 && sender.closeCode === null; i++) {
      await r.webSocketMessage(sender, frame());
    }
    expect(sender.closeCode).toBe(1008);
  });
});
