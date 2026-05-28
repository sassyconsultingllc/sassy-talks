/**
 * PTT Relay — Durable Object WebSocket server for SassyTalkie cellular transport.
 *
 * Each "room" is a separate Durable Object instance (keyed by session_id).
 * All WebSocket connections within the same room receive each other's binary
 * audio frames. The relay is a blind forwarder — it never decrypts audio.
 *
 * Protocol:
 *   Binary messages → broadcast to all other peers in the room
 *   Text messages   → JSON control (ping/pong, peer_joined, peer_left)
 *
 * Uses WebSocket Hibernation so idle rooms don't consume memory.
 *
 * Security:
 *   - Room capacity capped at MAX_PEERS_PER_ROOM (prevents abuse)
 *   - Dead sockets pruned on send failure
 *   - Device name length capped
 *   - All audio is end-to-end encrypted (AES-256-GCM) — relay never sees plaintext
 */

import { DurableObject } from "cloudflare:workers";
import { listPresence, dropPresence, getPresenceVersion } from "./presence.js";
import { sendWakePush } from "./fcm.js";

const MAX_PEERS_PER_ROOM = 16;
const MAX_DEVICE_NAME_LEN = 100;
const MAX_PEER_ID_LEN = 64;
const HEARTBEAT_STALE_MS = 8_000;
const SWEEP_INTERVAL_MS  = 2_000;

// ── In-memory replay ring buffer ────────────────────────────────────────────
// Holds the most-recent audio frames per peer in DO RAM (not storage). Used
// for two recovery cases:
//   1. Late join — a peer joining mid-utterance gets the last ~RING_BUFFER_MS
//      of audio from each currently-active sender, so they don't catch a half
//      sentence.
//   2. Brief disconnect — a peer that reconnects within RING_BUFFER_MS can
//      request frames newer than ts=X and get a contiguous catch-up slice.
// Bounded by RING_BUFFER_FRAMES_PER_PEER × MAX_PEERS_PER_ROOM × frame size
// (~12 KB encrypted at 24 kbps × 250 frames × 16 peers ≈ 48 MB worst case;
// realistic rooms with 2-4 active speakers ≈ 2-4 MB).
const RING_BUFFER_FRAMES_PER_PEER = 250;   // ~10 s at 25 fps
const RING_BUFFER_MS              = 10_000;
const REPLAY_BURST_MAX_FRAMES     = 250 * MAX_PEERS_PER_ROOM; // hard ceiling per request

// Per-peer replay_since rate limit. A single peer can request at most
// REPLAY_REQUESTS_PER_WINDOW catch-up dumps within REPLAY_REQUEST_WINDOW_MS.
// Typical legitimate use: 1 request on reconnect = ~1 per minute. Cap is
// generous (5/30s) to allow rapid reconnects on flaky cellular but blocks
// a peer hammering the DO with full ring dumps.
const REPLAY_REQUESTS_PER_WINDOW = 5;
const REPLAY_REQUEST_WINDOW_MS   = 30_000;

// Control opcodes added by this feature. Out of band of audio nonces (audio
// frames are 12-byte nonce || ciphertext — they never have a TLV opcode at
// byte 0). The text-control protocol stays JSON for human debuggability.
//   REPLAY_REQUEST  — text JSON  { type: "replay_since", ts: <unix_ms> }
//   REPLAY_FRAME    — binary, prefix byte 0x19 marking a replayed audio frame
const OP_REPLAY_FRAME = 0x19;

// FCM wake-push throttling. A push only fires when an OP_WAKE (0x17) or
// OP_PTT_START_V2 (0x15) frame is broadcast AND a registered peer has no
// active WS in this DO. Per-peer cooldown so a chatty operator can't turn
// the relay into an FCM spam pump.
const FCM_PUSH_COOLDOWN_MS = 10_000;
// Re-fetch presence list at most this often. Reads are cheap but a busy
// room with 25 audio frames/sec doesn't need fresh presence every frame.
const PRESENCE_CACHE_TTL_MS = 5_000;

// Opcodes that should trigger an FCM wake fan-out to offline peers.
// Each entry maps opcode → expected exact payload length (in bytes, the u16
// at TLV positions 1..2). The byte-precise length check disambiguates real
// control frames from audio nonce bytes that happen to start with the same
// opcode value.
const PUSH_TRIGGER_FRAME_SHAPES = {
  0x15: 12,  // OP_PTT_START_V2 — [epoch:u64][startSeq:u32]
  0x17: 16,  // OP_WAKE         — [epoch:u64][senderTsMs:u64]
};
function looksLikeTrigger(bytes) {
  if (bytes.length < 3) return false;
  const expected = PUSH_TRIGGER_FRAME_SHAPES[bytes[0]];
  if (expected === undefined) return false;
  const payloadLen = bytes[1] | (bytes[2] << 8);
  if (payloadLen !== expected) return false;
  if (bytes.length < 3 + expected) return false;
  return true;
}

// All known control-frame opcodes. To recognise one, the byte 0 opcode AND
// the inline length field AND the actual frame length must all agree. Audio
// frames start with a 12-byte random nonce — ~1/256 of them will coincidentally
// have a first byte that matches a control opcode, so opcode alone is NOT
// enough — without the length cross-check, ~1.5 % of audio frames would be
// misclassified as control and silently dropped from the ring buffer, creating
// gaps in replay.
//
// For variable-length opcodes we still validate: the payload length field
// at bytes[1..3] (LE u16) must equal `total_frame_len - 3` AND be within a
// plausible range [min, max]. This catches all coincidental matches in
// practice (audio frames have a uniformly random nonce, vanishingly unlikely
// to also have a matching length field).
const CONTROL_FRAME_SHAPES = {
  // opcode → { fixed: u16 | null, minLen: usize (for variable), maxLen }
  0x14: { fixed: null, minLen: 2,  maxLen: 65 },  // OP_PARTNER_OFFLINE: 1 + peer_id (peer_id ≤ 64)
  0x15: { fixed: 12,   minLen: 12, maxLen: 12 },  // OP_PTT_START_V2
  0x17: { fixed: 16,   minLen: 16, maxLen: 16 },  // OP_WAKE
  0x19: { fixed: null, minLen: 3,  maxLen: 70 },  // OP_REPLAY_FRAME: peer_id_len(2) + peer_id(≤64) + ...
};
function isControlFrame(bytes) {
  if (bytes.length < 3) return false;
  const shape = CONTROL_FRAME_SHAPES[bytes[0]];
  if (shape === undefined) return false;
  const payloadLen = bytes[1] | (bytes[2] << 8);
  // For fixed-length opcodes, the inline length must match exactly.
  if (shape.fixed !== null && payloadLen !== shape.fixed) return false;
  // For variable-length opcodes, length must be in the plausible range
  // AND consistent with the actual frame size.
  if (payloadLen < shape.minLen || payloadLen > shape.maxLen) return false;
  if (bytes.length < 3 + payloadLen) return false;
  return true;
}

// Per-socket rate limit. Normal PTT traffic is ~25 frames/sec (40 ms Opus
// frames). 120 messages/sec is ~5x headroom for bursts (e.g. recovery after
// brief network stall) but cuts off a runaway client well below the rate that
// could meaningfully spike DO billing.
const MAX_MESSAGES_PER_SEC = 120;
const RATE_WINDOW_MS = 1_000;

/**
 * Build a PARTNER_OFFLINE TLV binary frame for the given peerId.
 * Frame layout:
 *   [0]      opcode: u8  = 0x14
 *   [1..2]   payload_len: u16 LE  = 1 + peer_id_bytes.length
 *   [3]      peer_id_len: u8
 *   [4..]    peer_id bytes (UTF-8)
 */
export function buildPartnerOfflineFrame(peerId) {
  const idBytes = new TextEncoder().encode(peerId);
  const len = 1 + idBytes.length; // peer_id_len:u8 + peer_id bytes
  const out = new Uint8Array(3 + len);
  out[0] = 0x14; // OP_PARTNER_OFFLINE
  out[1] = len & 0xFF;
  out[2] = (len >> 8) & 0xFF; // u16 LE payload length
  out[3] = idBytes.length; // peer_id_len: u8
  out.set(idBytes, 4);
  return out;
}

export class PttRoom extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    // Map<WebSocket, { id: string, device: string, peer: string, joinedAt: number, lastSeenMs: number }>
    // Restored from serialized attachments on wake-up
    this.sessions = new Map();
    // Per-peer ring buffer of recent audio frames. peerId -> Array<{ts, data}>
    // where `data` is the raw (still-encrypted) wire bytes. Bounded by
    // RING_BUFFER_FRAMES_PER_PEER per peer. DO-local memory; lost on hibernate.
    // Hibernate-loss is acceptable: any peer that goes idle for long enough to
    // hibernate the DO has already missed the live audio they'd want replayed.
    this.ringBuffer = new Map();
    // In-memory shadow of "is the sweeper alarm armed?". Previously we
    // called `ctx.storage.getAlarm()` on every binary frame (~25 reads/sec
    // per active speaker) which is a hot-path storage round-trip both in
    // latency and in billed-op cost. Tracking this in RAM means we only
    // touch storage on (re-)arm and on the alarm callback itself.
    this.alarmArmed = false;
    // Per-socket replay_since rate limit. peerId -> array of timestamps.
    // Bounded to REPLAY_REQUEST_WINDOW_MS by trim-on-check. Prevents a
    // single peer from hammering the DO with full ring-buffer dumps.
    this.replayRequestTimes = new Map();
    // FCM-fanout state — DO-local, not persisted. Worst case after hibernation
    // is a single extra push per peer. Audio loss is the dominant cost; over-
    // delivery of wake pushes is benign.
    this.lastFcmPushMs = new Map();      // peerId -> ms
    this.presenceCache = null;           // { list: [{peer, token}], fetchedMs: number }
    // Cache the room name for FCM data payload. Restored from DO storage on
    // wake — without persistence, a wake-message arriving on a hibernated DO
    // would find roomId=null and silently skip FCM fan-out for offline peers.
    this.roomId = null;
    this.ctx.blockConcurrencyWhile(async () => {
      this.roomId = (await this.ctx.storage.get("roomId")) || null;
    });
  }

  /**
   * Handle incoming HTTP request — must be a WebSocket upgrade.
   * Query params: ?device=NAME&client_id=UUID
   */
  async fetch(request) {
    const url = new URL(request.url);
    const upgradeHeader = request.headers.get("Upgrade");

    if (!upgradeHeader || upgradeHeader !== "websocket") {
      return new Response("Expected WebSocket upgrade", { status: 426 });
    }

    // Enforce room capacity
    const currentPeers = this.ctx.getWebSockets().length;
    if (currentPeers >= MAX_PEERS_PER_ROOM) {
      return new Response("Room full", { status: 429 });
    }

    // Create the WebSocket pair
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    // Accept with hibernation — the DO can sleep between messages
    this.ctx.acceptWebSocket(server);

    const clientId = crypto.randomUUID();
    const rawDevice = url.searchParams.get("device") || "Unknown";
    const device = decodeURIComponent(rawDevice).substring(0, MAX_DEVICE_NAME_LEN);
    // Stable per-install peer ID (provided by the app — see
    // SassyTalkNative.getStablePeerId on the client). Used to:
    //   1. Match WS sessions against /presence FCM registrations so we know
    //      which registered peers are currently offline.
    //   2. Persist across app restarts so a returning user doesn't accumulate
    //      stale presence rows.
    const rawPeer = url.searchParams.get("peer") || "";
    const peer = decodeURIComponent(rawPeer).substring(0, MAX_PEER_ID_LEN);
    // Also remember which room this DO is — needed when firing FCM pushes.
    const roomParam = url.searchParams.get("room") || "";
    if (!this.roomId && roomParam) {
      this.roomId = roomParam;
      await this.ctx.storage.put("roomId", this.roomId);
    }

    const session = {
      id: clientId,
      device,
      peer,
      joinedAt: Date.now(),
      lastSeenMs: Date.now(),
      // Per-socket sliding-window rate limit state. Re-initialised on every
      // restore from hibernation attachment (window has clearly elapsed if we
      // were hibernating), so we don't bother persisting these fields.
      windowStartMs: Date.now(),
      msgCount: 0,
    };

    // Persist session info so it survives hibernation
    server.serializeAttachment(session);
    this.sessions.set(server, session);

    // Notify all existing peers about the new connection
    const joinMsg = JSON.stringify({
      type: "peer_joined",
      client_id: clientId,
      device: session.device,
      peers: this.sessions.size,
    });
    this.broadcast(server, joinMsg);

    // Send welcome to the new client
    server.send(JSON.stringify({
      type: "welcome",
      client_id: clientId,
      peers: this.sessions.size,
    }));

    // Start sweeper alarm if not already running. In-memory flag avoids a
    // storage round-trip per connection.
    if (!this.alarmArmed) {
      await this.ctx.storage.setAlarm(Date.now() + SWEEP_INTERVAL_MS);
      this.alarmArmed = true;
    }

    return new Response(null, { status: 101, webSocket: client });
  }

  /**
   * Called when a WebSocket receives a message (binary or text).
   * Binary = encrypted audio frame → broadcast to all other peers.
   * Text = control message (ping → pong).
   */
  async webSocketMessage(ws, message) {
    // Restore session from attachment if we just woke from hibernation
    if (!this.sessions.has(ws)) {
      const attachment = ws.deserializeAttachment();
      if (attachment) {
        // Reset rate-limit window on wake — by definition we've been idle.
        attachment.windowStartMs = Date.now();
        attachment.msgCount = 0;
        this.sessions.set(ws, attachment);
      }
    }

    // Per-socket rate limit. Anyone exceeding MAX_MESSAGES_PER_SEC is closed
    // immediately; a healthy client never approaches this rate even during
    // recovery bursts.
    const rlSession = this.sessions.get(ws);
    if (rlSession) {
      const now = Date.now();
      if (now - rlSession.windowStartMs >= RATE_WINDOW_MS) {
        rlSession.windowStartMs = now;
        rlSession.msgCount = 0;
      }
      rlSession.msgCount++;
      if (rlSession.msgCount > MAX_MESSAGES_PER_SEC) {
        try { ws.close(1008, "Rate limit exceeded"); } catch {}
        this.sessions.delete(ws);
        return;
      }
    }

    if (typeof message !== "string") {
      // Binary message
      const bytes = message instanceof ArrayBuffer
        ? new Uint8Array(message)
        : new Uint8Array(message.buffer, message.byteOffset, message.byteLength);

      // Liveness refresh — ANY inbound binary frame counts as proof of life,
      // not just OP_HEARTBEAT. Otherwise a 1-peer room (or any client that
      // emits other control frames like OP_WAKE / OP_PTT_START_V2 but doesn't
      // get scheduled HBs out yet) gets falsely closed by the staleness
      // sweeper at the 8-second mark. Audio frames at ~25fps trivially keep
      // a talking peer alive; control frames keep an idle-but-attached peer
      // alive. The 0x10 specific check is kept below as a sanity guard on
      // payload size, but its lastSeenMs touch is now redundant.
      const sessRefresh = this.sessions.get(ws);
      if (sessRefresh) {
        sessRefresh.lastSeenMs = Date.now();
        // serializeAttachment is what survives hibernation; cheap, but avoid
        // doing it more than once per second to keep storage writes bounded
        // when a peer is streaming audio at 25 fps.
        const nowMs = sessRefresh.lastSeenMs;
        if (!sessRefresh.lastAttachWriteMs || (nowMs - sessRefresh.lastAttachWriteMs) > 1000) {
          sessRefresh.lastAttachWriteMs = nowMs;
          ws.serializeAttachment(sessRefresh);
        }
      }

      // Defensive: ensure the sweeper alarm is always armed while sockets
      // are active. In-memory flag — no per-frame storage read. Worst case
      // after hibernation: alarm doesn't fire for one extra SWEEP_INTERVAL_MS
      // until the next message; acceptable.
      if (!this.alarmArmed) {
        await this.ctx.storage.setAlarm(Date.now() + SWEEP_INTERVAL_MS);
        this.alarmArmed = true;
      }

      // Binary broadcast to all other peers
      // This is the hot path — zero parsing, zero copying, just fan-out.
      const sockets = this.ctx.getWebSockets();
      for (const peer of sockets) {
        if (peer !== ws) {
          try {
            peer.send(message);
          } catch {
            // Send failed — peer is dead. Force-close so it gets cleaned up.
            try { peer.close(1011, "Send failed"); } catch { /* already closed */ }
            this.sessions.delete(peer);
          }
        }
      }

      // Ring-buffer append. Off the hot path conceptually (no network) but
      // still O(1). Only audio-shaped frames are stored — control frames
      // (OP_PARTNER_OFFLINE, OP_WAKE, OP_PTT_START_V2, OP_REPLAY_FRAME) are
      // skipped since replaying them has no value and would confuse clients.
      const senderInfo = this.sessions.get(ws);
      if (senderInfo && senderInfo.peer && !isControlFrame(bytes)) {
        let ring = this.ringBuffer.get(senderInfo.peer);
        if (!ring) {
          ring = [];
          this.ringBuffer.set(senderInfo.peer, ring);
        }
        // Capture `now` once so the just-pushed frame's ts cannot land on
        // the wrong side of the trim cutoff if the two Date.now() calls
        // straddle a millisecond boundary.
        // Also `.slice()` the bytes — `message` is owned by the runtime
        // and the underlying ArrayBuffer may be detached/reused after
        // this event handler returns. The ring needs an OWNED copy or
        // replays will serve corrupt frames.
        const nowTs = Date.now();
        ring.push({ ts: nowTs, data: bytes.slice() });
        // Trim head — keep last RING_BUFFER_FRAMES_PER_PEER, drop anything
        // older than RING_BUFFER_MS regardless of count.
        const cutoffTs = nowTs - RING_BUFFER_MS;
        while (ring.length > RING_BUFFER_FRAMES_PER_PEER || (ring.length && ring[0].ts < cutoffTs)) {
          ring.shift();
        }
      }

      // FCM wake-push fan-out for offline peers. Only fires for OP_WAKE (0x17)
      // or OP_PTT_START_V2 (0x15) — i.e. moments where a sender is actively
      // trying to engage. Runs via waitUntil so it never blocks audio fan-out.
      //
      // Strict TLV-shape check: encrypted audio frames have a 12-byte random
      // nonce as their first bytes, so ~1/128 of them have a first byte of
      // 0x15 or 0x17 and would spuriously trigger FCM fan-out without this
      // guard. Real OP_WAKE has payload length 16 (TLV header at bytes 1..2
      // == 0x10 0x00 LE) and total length 19; real OP_PTT_START_V2 has
      // payload length 12 (0x0C 0x00 LE) and total length 15. Both fit
      // within ~32 bytes — well clear of typical audio frame sizes.
      if (looksLikeTrigger(bytes)) {
        this.ctx.waitUntil(this.firePushesForOfflinePeers(sockets));
      }
      return;
    }

    // Text control message
    try {
      const parsed = JSON.parse(message);
      if (parsed.type === "ping") {
        ws.send(JSON.stringify({ type: "pong", ts: Date.now() }));
      } else if (parsed.type === "replay_since") {
        // Client requests catch-up frames newer than `ts` (unix ms).
        // Require `ts` to be a finite positive number — without this, a
        // missing/zero `ts` would request EVERYTHING in the ring (10s ×
        // 16 peers × 25 fps = ~4000 frames, ~6 MB), which a single misbehaving
        // peer could spam to OOM the DO isolate.
        if (!Number.isFinite(parsed.ts) || parsed.ts <= 0) {
          ws.send(JSON.stringify({ type: "replay_error", reason: "invalid_ts" }));
          return;
        }
        // Per-peer rate limit. Legitimate use is ~1 request per reconnect;
        // 5 per 30 s is generous and catches any spam attempt.
        const sessRl = this.sessions.get(ws);
        const peerKey = (sessRl && sessRl.peer) || (sessRl && sessRl.id) || "anon";
        let times = this.replayRequestTimes.get(peerKey) || [];
        const cutoff = Date.now() - REPLAY_REQUEST_WINDOW_MS;
        times = times.filter(t => t > cutoff);
        if (times.length >= REPLAY_REQUESTS_PER_WINDOW) {
          ws.send(JSON.stringify({ type: "replay_error", reason: "rate_limited" }));
          return;
        }
        times.push(Date.now());
        this.replayRequestTimes.set(peerKey, times);
        await this.handleReplayRequest(ws, parsed);
      }
    } catch {
      // Ignore malformed JSON
    }
  }

  /**
   * Send a catch-up burst of recent frames from the ring buffer.
   * Frames are sent in (ts ASC, peerId) order so the receiver can mix or
   * sequence them client-side without re-sorting.
   *
   * Wire format of each replay frame:
   *   [0]      OP_REPLAY_FRAME (0x19)
   *   [1..2]   peerId_len: u16 LE
   *   [3..]    peerId bytes (UTF-8)
   *   [...]    original encrypted audio frame (nonce || ciphertext || tag)
   */
  async handleReplayRequest(ws, parsed) {
    const sinceTs = Number.isFinite(parsed.ts) ? Number(parsed.ts) : 0;
    const minTs = Math.max(sinceTs, Date.now() - RING_BUFFER_MS);

    // Gather (ts, peerId, data) tuples from every peer's ring, sort by ts ASC.
    const out = [];
    for (const [peerId, ring] of this.ringBuffer) {
      for (const entry of ring) {
        if (entry.ts > minTs) out.push({ ts: entry.ts, peerId, data: entry.data });
      }
    }
    out.sort((a, b) => a.ts - b.ts);
    if (out.length > REPLAY_BURST_MAX_FRAMES) out.length = REPLAY_BURST_MAX_FRAMES;

    // Announce frame count first so the client can show a progress hint and
    // pre-size its catch-up buffer.
    try {
      ws.send(JSON.stringify({
        type: "replay_begin",
        frames: out.length,
        oldest_ts: out.length ? out[0].ts : null,
        newest_ts: out.length ? out[out.length - 1].ts : null,
      }));
    } catch { /* socket closed mid-call */ }

    const enc = new TextEncoder();
    for (const { peerId, data } of out) {
      const idBytes = enc.encode(peerId);
      const frame = new Uint8Array(3 + idBytes.length + data.length);
      frame[0] = OP_REPLAY_FRAME;
      frame[1] = idBytes.length & 0xFF;
      frame[2] = (idBytes.length >> 8) & 0xFF;
      frame.set(idBytes, 3);
      frame.set(data, 3 + idBytes.length);
      try { ws.send(frame); } catch { return; /* socket gone */ }
    }

    try { ws.send(JSON.stringify({ type: "replay_end" })); } catch {}
  }

  /**
   * Alarm-based sweeper: runs every SWEEP_INTERVAL_MS.
   * Checks for stale peers (no heartbeat in HEARTBEAT_STALE_MS) and pushes
   * PARTNER_OFFLINE TLV frames to remaining peers before closing the stale socket.
   */
  async alarm() {
    const now = Date.now();
    let liveCount = 0;

    for (const ws of this.ctx.getWebSockets()) {
      if (!this.sessions.has(ws)) {
        const att = ws.deserializeAttachment();
        if (att) this.sessions.set(ws, att);
      }
      const session = this.sessions.get(ws);
      if (session && session.lastSeenMs && now - session.lastSeenMs > HEARTBEAT_STALE_MS) {
        // Push PARTNER_OFFLINE to other peers
        const frame = buildPartnerOfflineFrame(session.id || "unknown");
        for (const peer of this.ctx.getWebSockets()) {
          if (peer !== ws) {
            try { peer.send(frame); } catch {}
          }
        }
        try { ws.close(1001, "Heartbeat stale"); } catch {}
        this.sessions.delete(ws);
      } else {
        liveCount++;
      }
    }
    // Re-arm based on the AUTHORITATIVE post-loop count. The `liveCount`
    // variable counted sockets seen non-stale during the iteration, but
    // sockets we just `.close()`d may still appear in `getWebSockets()`
    // in the same tick — leading to a leaked alarm in an empty room.
    // Re-query at the end to get the true count after closures settle.
    const remainingSockets = this.ctx.getWebSockets().length;
    if (remainingSockets > 0) {
      await this.ctx.storage.setAlarm(Date.now() + SWEEP_INTERVAL_MS);
      this.alarmArmed = true;
    } else {
      // Room is empty — alarm has already fired (we're inside it) and
      // we're not re-arming, so the flag is now correct.
      this.alarmArmed = false;
    }
  }

  /**
   * Called when a WebSocket connection closes.
   */
  async webSocketClose(ws, code, reason, wasClean) {
    const session = this.sessions.get(ws) || ws.deserializeAttachment() || {};
    this.sessions.delete(ws);
    // Drop this peer's ring buffer + rate-limit history. They're gone, the
    // buffered frames have no future audience, and the rate-limit window
    // resets cleanly on reconnect.
    if (session.peer) {
      this.ringBuffer.delete(session.peer);
      this.replayRequestTimes.delete(session.peer);
    }
    // Note: the runtime already initiated the close (that's why this handler
    // fired). The previous explicit `ws.close()` was a no-op at best — the
    // socket is by-definition closing here.

    const sockets = this.ctx.getWebSockets();

    // Push binary PARTNER_OFFLINE TLV to remaining peers
    const offlineFrame = buildPartnerOfflineFrame(session.id || "unknown");
    for (const peer of sockets) {
      try { peer.send(offlineFrame); } catch {}
    }
  }

  /**
   * Called when a WebSocket encounters an error.
   */
  async webSocketError(ws, error) {
    const session = this.sessions.get(ws) || {};
    console.error(`WebSocket error for ${session.id || "unknown"}: ${error}`);
    this.sessions.delete(ws);
    try { ws.close(1011, "Error"); } catch { /* ignore */ }
  }

  /**
   * Fire FCM wake pushes to peers registered for /presence that are NOT
   * currently attached as a WebSocket. Triggered from the binary fan-out
   * path on OP_WAKE or OP_PTT_START_V2 frames. Per-peer 10s cooldown.
   *
   * Best-effort: runs inside waitUntil(), so failures are logged but never
   * stall audio delivery to peers that ARE connected.
   */
  async firePushesForOfflinePeers(activeSockets) {
    if (!this.env || !this.env.SHARES) return;
    if (!this.roomId) return;
    if (!this.env.FCM_SERVICE_ACCOUNT_JSON) return; // FCM not configured — silent skip

    // Build the set of currently-attached peerIds.
    const attached = new Set();
    for (const ws of activeSockets) {
      const s = this.sessions.get(ws) || ws.deserializeAttachment() || null;
      if (s && s.peer) attached.add(s.peer);
    }

    // Get the room's presence list (cached briefly to keep KV traffic sane
    // when bursts of wake / PTT_START frames arrive close together).
    // Version stamp lets the /presence DELETE handler invalidate this cache
    // without an inter-DO RPC — we just compare.
    const now = Date.now();
    let liveVersion = null;
    try { liveVersion = await getPresenceVersion(this.env, this.roomId); }
    catch { /* fall through; treat as same-version */ }
    const cacheStale = !this.presenceCache ||
      (now - this.presenceCache.fetchedMs) > PRESENCE_CACHE_TTL_MS ||
      (liveVersion !== null && this.presenceCache.version !== liveVersion);
    if (cacheStale) {
      try {
        const list = await listPresence(this.env, this.roomId);
        this.presenceCache = { list, fetchedMs: now, version: liveVersion };
      } catch (e) {
        console.error("presence list failed:", e.message);
        return;
      }
    }

    for (const { peer, token } of this.presenceCache.list) {
      if (attached.has(peer)) continue; // peer is online — no push needed
      const lastMs = this.lastFcmPushMs.get(peer) || 0;
      if (now - lastMs < FCM_PUSH_COOLDOWN_MS) continue; // cooled down
      this.lastFcmPushMs.set(peer, now);

      try {
        const r = await sendWakePush(this.env, token, this.roomId);
        if (!r.ok && r.stale) {
          // FCM said the token is dead; drop the presence row so we don't
          // keep trying. The app re-registers on next launch / token refresh.
          await dropPresence(this.env, this.roomId, peer);
          // Also drop from cache so this iteration doesn't try again.
          this.presenceCache.list = this.presenceCache.list.filter((p) => p.peer !== peer);
          console.warn(`FCM token stale, dropped presence: room=${this.roomId} peer=${peer}`);
        } else if (!r.ok) {
          console.warn(`FCM push failed status=${r.status} room=${this.roomId} peer=${peer}: ${r.error}`);
        }
      } catch (e) {
        console.error(`FCM push exception room=${this.roomId} peer=${peer}: ${e.message}`);
      }
    }
  }

  /**
   * Broadcast a message to all peers except the sender.
   */
  broadcast(sender, message) {
    const sockets = this.ctx.getWebSockets();
    for (const peer of sockets) {
      if (peer !== sender) {
        try {
          peer.send(message);
        } catch {
          // Dead socket — force-close
          try { peer.close(1011, "Broadcast failed"); } catch { /* ignore */ }
          this.sessions.delete(peer);
        }
      }
    }
  }
}
