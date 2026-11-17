// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
import { describe, it, expect } from "vitest";
import worker from "../src/ptt-relay-worker.js";
import { PttRoom, replyCloseCode } from "../src/ptt-relay.js";
import { hmacSha256Hex } from "../src/relay-auth.js";

function sock(att, { closeThrows = false } = {}) {
  return {
    att,
    sent: [],
    closeCalls: [],
    deserializeAttachment() { return this.att; },
    serializeAttachment(a) { this.att = a; },
    close(code, reason) {
      this.closeCalls.push([code, reason]);
      if (closeThrows) throw new TypeError("WebSocket already closed");
    },
  };
}

function room(sockets) {
  const r = Object.create(PttRoom.prototype);
  r.sessions = new Map();
  r.ctx = { getWebSockets: () => sockets, storage: { setAlarm: async () => {} } };
  r.safeSend = (peer, frame) => { peer.sent.push(frame); return true; };
  return r;
}

describe("replyCloseCode", () => {
  it("echoes an ordinary close code", () => {
    expect(replyCloseCode(1000)).toBe(1000);
    expect(replyCloseCode(1001)).toBe(1001);
    expect(replyCloseCode(4001)).toBe(4001);
  });

  it("answers reserved and out-of-range codes with 1000, which close() accepts", () => {
    for (const code of [1004, 1005, 1006, 1015, 0, 999, 5000, undefined, NaN, 1000.5]) {
      expect(replyCloseCode(code)).toBe(1000);
    }
  });
});

describe("close handshake", () => {
  it("answers a client's Close frame so the socket does not linger", async () => {
    const leaving = sock({ peer: "a", lastSeenMs: Date.now() });
    const staying = sock({ peer: "b", lastSeenMs: Date.now() });
    const r = room([leaving, staying]);
    await r.webSocketClose(leaving, 1000, "user disconnect", true);
    expect(leaving.closeCalls.length).toBe(1);
    expect(leaving.closeCalls[0][0]).toBe(1000);
    // The departure is still announced to the peer that stayed.
    expect(staying.sent.length).toBe(1);
    expect(staying.sent[0][0]).toBe(0x14);
  });

  it("does not hand close() the reserved code of an abnormal drop", async () => {
    const dropped = sock({ peer: "a", lastSeenMs: Date.now() });
    const r = room([dropped]);
    await r.webSocketClose(dropped, 1006, "", false);
    expect(dropped.closeCalls[0][0]).toBe(1000);
  });

  it("still announces the departure when the socket is already closed", async () => {
    const gone = sock({ peer: "a", lastSeenMs: Date.now() }, { closeThrows: true });
    const staying = sock({ peer: "b", lastSeenMs: Date.now() });
    const r = room([gone, staying]);
    await r.webSocketClose(gone, 1001, "Heartbeat stale", true);
    expect(staying.sent.length).toBe(1);
  });
});

describe("/ws front door with a legacy token", () => {
  const SECRET = "test-only-auth-secret-with-enough-entropy";
  const ROOM = "room-test-0001";

  async function open(peer) {
    let forwarded = null;
    const env = {
      AUTH_SECRET: SECRET,
      PTT_RELAY: {
        idFromName: (name) => name,
        get: () => ({
          fetch: async (req) => { forwarded = req; return new Response("ok"); },
        }),
      },
    };
    const exp = Math.floor(Date.now() / 1000) + 300;
    const sig = await hmacSha256Hex(`${ROOM}.${exp}`, SECRET);
    const url = `https://relay.test/ws?room=${ROOM}&token=${exp}.${sig}` +
      `&peer=${encodeURIComponent(peer)}`;
    const res = await worker.fetch(new Request(url, { headers: { Upgrade: "websocket" } }), env);
    return { res, forwarded };
  }

  it("forwards a well-formed peer id to the room", async () => {
    const { res, forwarded } = await open("6f1c2a5e-0d3b-4c7a-9e21-abcdef012345");
    expect(res.status).toBe(200);
    expect(forwarded.headers.get("X-Sassy-Verified-Peer"))
      .toBe("6f1c2a5e-0d3b-4c7a-9e21-abcdef012345");
    expect(forwarded.headers.get("X-Sassy-Verified-Auth-Class")).toBe("legacy");
  });

  it("does not fail the upgrade on a peer id that cannot be a header value", async () => {
    const { res, forwarded } = await open("péer-\u{1F600}");
    expect(res.status).toBe(200);
    expect(forwarded.headers.get("X-Sassy-Verified-Peer")).toBe("");
  });
});
