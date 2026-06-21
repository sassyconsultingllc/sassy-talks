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

// FCM wake-push throttling. A push only fires when an OP_WAKE (0x17) or
// OP_PTT_START_V2 (0x15) frame is broadcast AND a registered peer has no
// active WS in this DO. Per-peer cooldown so a chatty operator can't turn
// the relay into an FCM spam pump.
const FCM_PUSH_COOLDOWN_MS = 10_000;
// Re-fetch presence list at most this often. Reads are cheap but a busy
// room with 50 audio frames/sec doesn't need fresh presence every frame.
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

// Per-socket rate limit. Normal PTT traffic is ~50 frames/sec (20 ms Opus
// frames — see codec.rs CODEC_FRAME_SIZE=960 @ 48 kHz). 120 messages/sec is
// ~2.4x headroom for bursts (e.g. recovery after a brief network stall) but
// cuts off a runaway client well below the rate that could meaningfully spike
// DO billing.
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

// decodeURIComponent throws URIError on malformed percent-sequences (e.g.
// "%GG"). Query params are attacker-influenced (device/peer come from the
// client), so a bad value would otherwise 500 the DO fetch handler.
function safeDecode(s, fallback) {
  try { return decodeURIComponent(s); } catch { return fallback; }
}

export class PttRoom extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    // Map<WebSocket, { id: string, device: string, peer: string, joinedAt: number, lastSeenMs: number }>
    // Restored from serialized attachments on wake-up
    this.sessions = new Map();
    // In-memory shadow of "is the sweeper alarm armed?". Previously we
    // called `ctx.storage.getAlarm()` on every binary frame (~50 reads/sec
    // per active speaker) which is a hot-path storage round-trip both in
    // latency and in billed-op cost. Tracking this in RAM means we only
    // touch storage on (re-)arm and on the alarm callback itself.
    this.alarmArmed = false;
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
    const device = safeDecode(rawDevice, "Unknown").substring(0, MAX_DEVICE_NAME_LEN);
    // Stable per-install peer ID (provided by the app — see
    // SassyTalkNative.getStablePeerId on the client). Used to:
    //   1. Match WS sessions against /presence FCM registrations so we know
    //      which registered peers are currently offline.
    //   2. Persist across app restarts so a returning user doesn't accumulate
    //      stale presence rows.
    const rawPeer = url.searchParams.get("peer") || "";
    const peer = safeDecode(rawPeer, "").substring(0, MAX_PEER_ID_LEN);
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
      // sweeper at the 8-second mark. Audio frames at ~50fps trivially keep
      // a talking peer alive; control frames keep an idle-but-attached peer
      // alive. The 0x10 specific check is kept below as a sanity guard on
      // payload size, but its lastSeenMs touch is now redundant.
      const sessRefresh = this.sessions.get(ws);
      if (sessRefresh) {
        sessRefresh.lastSeenMs = Date.now();
        // serializeAttachment is what survives hibernation; cheap, but avoid
        // doing it more than once per second to keep storage writes bounded
        // when a peer is streaming audio at 50 fps.
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
      }
    } catch {
      // Ignore malformed JSON
    }
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
