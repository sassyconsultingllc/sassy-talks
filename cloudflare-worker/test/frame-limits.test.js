// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
import { describe, it, expect } from "vitest";
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
  r.ctx = {
    getWebSockets: () => sockets,
    storage: { setAlarm: async () => {} },
    waitUntil: () => {},
  };
  return r;
}

describe("inbound frame size cap", () => {
  it("closes a socket that sends an oversized binary frame and does not fan it out", async () => {
    const sender = sock({ peer: "a", lastSeenMs: Date.now() });
    const other = sock({ peer: "b", lastSeenMs: Date.now() });
    const r = room([sender, other]);
    await r.webSocketMessage(sender, new Uint8Array(64 * 1024 + 1).buffer);
    expect(sender.closeCode).toBe(1009);
    expect(sender.att.closing).toBe(true);
    expect(other.sent.length).toBe(0);
    expect(r.sessions.has(sender)).toBe(false);
  });

  it("closes a socket that sends an oversized text frame before parsing it", async () => {
    const sender = sock({ peer: "a", lastSeenMs: Date.now() });
    const r = room([sender]);
    await r.webSocketMessage(sender, "x".repeat(64 * 1024 + 1));
    expect(sender.closeCode).toBe(1009);
  });
});

describe("markClosing", () => {
  it("flags the attachment so the wake path stops counting the peer as attached", () => {
    const ws = sock({ peer: "a", lastSeenMs: Date.now() });
    const r = room([ws]);
    r.markClosing(ws);
    expect(ws.att.closing).toBe(true);
    expect(ws.att.peer).toBe("a");
  });
});
