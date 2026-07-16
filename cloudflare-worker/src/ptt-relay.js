// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-6NT2F2PAVM7X
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
// Per-ROOM push budget on top of the per-peer cooldown. The per-peer cooldown
// alone lets an authorized peer cycle through N offline peers and wake all of
// them every 10 s on demand (each wake = a presence read + an FCM OAuth/send).
// This bounds total pushes a single room can emit per window regardless of how
// many offline peers exist or how often a sender fires trigger frames.
const FCM_ROOM_PUSH_BUDGET = 30;       // max pushes per room per window
const FCM_ROOM_PUSH_WINDOW_MS = 60_000;
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

// ── Store-and-forward (async voice / catch-up) ────────────────────────────
// A bounded, DO-local in-memory ring buffer of the most recent broadcast
// audio frames. A peer woken by an FCM push (or recovering from a network
// drop) can reconnect with `?catchup=1` (or `?since=<ms>`) and be replayed
// the audio it missed, wrapped as OP_REPLAY_FRAME (0x19) frames.
//
// The buffer is intentionally NOT persisted to ctx.storage — appending on the
// 50 fps audio hot path must add zero storage round-trips. Losing it across
// hibernation is acceptable: catch-up is best-effort voicemail, not the
// primary delivery path (live fan-out + FCM wake already cover that). This
// mirrors `lastFcmPushMs` / `presenceCache`, which are likewise RAM-only.
//
// Bounds are enforced by BOTH count and bytes, whichever trips first, with an
// O(1) shift of the oldest entry. At ~50 fps a 30 s window is ~1500 frames;
// the byte cap protects against larger frames (or higher rates) blowing RAM.
const BUFFER_MAX_FRAMES = 1500;        // ~30 s of audio at 50 fps (20 ms Opus)
const BUFFER_MAX_BYTES  = 2 * 1024 * 1024; // ~2 MB hard cap regardless of count
const BUFFER_TTL_MS     = 30_000;      // drop frames older than this on replay

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

/**
 * Wrap a previously-broadcast audio frame as an OP_REPLAY_FRAME (0x19) for
 * catch-up delivery. NOTE: this is NOT the standard `encode_tlv`
 * `[op][len:u16 LE][payload]` shape — per core/src/protocol.rs the replay
 * frame carries a peer_id LENGTH (not a payload length) right after the
 * opcode, then the original audio appended raw to the end of the message:
 *
 *   [0]      opcode: u8        = 0x19
 *   [1..2]   peer_id_len: u16 LE
 *   [3..]    peer_id bytes (UTF-8)
 *   [...]    original_audio_frame (encrypted: nonce + ciphertext + tag),
 *            copied verbatim — the receiver reads it to end-of-message.
 *
 * The relay forwards blind and never tracks which peer originated a given
 * broadcast frame (the fan-out path does zero parsing), so peer_id is emitted
 * as the EMPTY STRING (peer_id_len = 0). Receivers route replayed audio by the
 * sender epoch embedded inside the encrypted/decoded frame, exactly as they do
 * for live frames — the relay-level peer_id is informational only.
 */
export function buildReplayFrame(audioFrame, peerId = "") {
  const idBytes = new TextEncoder().encode(peerId);
  const out = new Uint8Array(3 + idBytes.length + audioFrame.length);
  out[0] = 0x19; // OP_REPLAY_FRAME
  out[1] = idBytes.length & 0xFF;
  out[2] = (idBytes.length >> 8) & 0xFF; // peer_id_len: u16 LE
  out.set(idBytes, 3);
  out.set(audioFrame, 3 + idBytes.length);
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
    // Per-room FCM push budget window (in-memory, see FCM_ROOM_PUSH_* consts).
    this.fcmRoomWindowStartMs = 0;
    this.fcmRoomPushCount = 0;
    // Store-and-forward ring buffer — DO-local, NOT persisted (see the
    // BUFFER_* consts above). Each entry is { ts: number, frame: Uint8Array }.
    // Oldest at index 0; we push to the end and shift the front when over cap.
    // `audioBufferBytes` shadows the summed `frame.length` so the byte-cap trim
    // stays O(1) (no re-summing the array on the hot path).
    this.audioBuffer = [];
    this.audioBufferBytes = 0;
    // Store-and-forward is only useful when a peer might reconnect and ask to
    // catch up. We open a buffering window (this.bufferUntilMs) after any peer
    // disconnect or FCM wake — the only moments a catch-up reconnect can follow.
    // In the steady state (all peers live-connected, nobody dropping) we skip
    // the per-frame copy entirely instead of buffering ~800 frames/sec/room
    // that no one will ever replay.
    this.bufferUntilMs = 0;
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

    // Store-and-forward catch-up. Only replay when the client explicitly asks
    // (so normal joins don't get a replay blast on top of live audio):
    //   ?catchup=1       → replay the whole retained window
    //   ?since=<ms>      → replay only frames with ts > <ms> (epoch millis)
    // A peer woken by the FCM push reconnects with one of these to hear what
    // it missed. Replay is paced in arrival order, after the welcome, so the
    // client sees its session is live before the catch-up stream starts.
    const wantsCatchup = url.searchParams.get("catchup") === "1";
    const sinceRaw = url.searchParams.get("since");
    const sinceMs = sinceRaw !== null ? Number(sinceRaw) : NaN;
    if (wantsCatchup || Number.isFinite(sinceMs)) {
      // Snapshot the buffer cutoff now; replay runs in waitUntil so it never
      // blocks the 101 handshake response.
      const cutoff = Number.isFinite(sinceMs)
        ? sinceMs
        : Date.now() - BUFFER_TTL_MS;
      this.ctx.waitUntil(this.replayBufferedAudio(server, cutoff, clientId));
    }

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

      // Store-and-forward append — ONLY while a catch-up reconnect is plausible
      // (this.bufferUntilMs, opened on peer disconnect / FCM wake). In the
      // steady state (everyone live, nobody dropping) catch-up is never
      // requested, so we skip the per-frame Uint8Array copy + push + trim
      // entirely — that's ~800 copies/sec saved in a full 16-peer room.
      //
      // Hot-path cost when active: one Uint8Array copy + array push, plus an
      // occasional O(1) front-shift when over cap. NO storage write, NO await.
      // We copy because `bytes` may alias a transient runtime buffer reused
      // after this turn; the copy is also exactly what gets re-emitted later.
      //
      // Control frames (PTT_START/STOP/WAKE/etc.) are tiny and harmless to keep
      // — replaying them is a no-op for receivers — so we buffer every binary
      // frame uniformly rather than branch on opcode in the hot path.
      if (Date.now() <= this.bufferUntilMs) {
        const framed = new Uint8Array(bytes.length);
        framed.set(bytes);
        this.audioBuffer.push({ ts: Date.now(), frame: framed });
        this.audioBufferBytes += framed.length;
        // O(1) trim: shift the oldest until both caps are satisfied. `shift()`
        // on a JS array is amortized cheap and we only ever drop a handful per
        // append at steady state (one in, one out once the window is full).
        while (
          this.audioBuffer.length > BUFFER_MAX_FRAMES ||
          this.audioBufferBytes > BUFFER_MAX_BYTES
        ) {
          const evicted = this.audioBuffer.shift();
          if (!evicted) break;
          this.audioBufferBytes -= evicted.frame.length;
        }
      } else if (this.audioBuffer.length > 0) {
        // Window lapsed and no one is around to catch up — release the retained
        // frames so a quiet room doesn't hold ~2 MB until hibernation.
        this.audioBuffer.length = 0;
        this.audioBufferBytes = 0;
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
        // A stale peer is being dropped — it may reconnect with ?catchup.
        this.bufferUntilMs = now + BUFFER_TTL_MS;
        // Push PARTNER_OFFLINE to other peers, keyed on the stable peer id.
        const frame = buildPartnerOfflineFrame(session.peer || session.id || "unknown");
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

    // A peer just dropped — it may reconnect with ?catchup. Open a buffering
    // window so the store-and-forward ring actually retains what it missed.
    this.bufferUntilMs = Date.now() + BUFFER_TTL_MS;

    const sockets = this.ctx.getWebSockets();

    // Push binary PARTNER_OFFLINE TLV to remaining peers, keyed on the STABLE
    // per-install peer id (not session.id, the per-connection UUID the other
    // side never learned). Receivers track partners by the stable peer id, so
    // keying on session.id meant the offline notice never matched a partner.
    const offlineFrame = buildPartnerOfflineFrame(session.peer || session.id || "unknown");
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

    // Per-room budget: cap total pushes this room can emit per window, on top of
    // the per-peer cooldown. Stops a peer cycling triggers across many offline
    // peers from turning the relay into an FCM/KV pump.
    const winNow = Date.now();
    if (winNow - this.fcmRoomWindowStartMs > FCM_ROOM_PUSH_WINDOW_MS) {
      this.fcmRoomWindowStartMs = winNow;
      this.fcmRoomPushCount = 0;
    }
    if (this.fcmRoomPushCount >= FCM_ROOM_PUSH_BUDGET) return; // budget exhausted this window

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
      if (this.fcmRoomPushCount >= FCM_ROOM_PUSH_BUDGET) break; // room budget hit
      const lastMs = this.lastFcmPushMs.get(peer) || 0;
      if (now - lastMs < FCM_PUSH_COOLDOWN_MS) continue; // cooled down
      this.lastFcmPushMs.set(peer, now);
      this.fcmRoomPushCount++;
      // We're waking an offline peer — it will reconnect (likely with ?catchup),
      // so open the store-and-forward buffering window to retain the utterance.
      this.bufferUntilMs = now + BUFFER_TTL_MS;

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
   * Replay buffered audio frames newer than `cutoff` (epoch ms) to a single
   * just-connected socket, wrapped as OP_REPLAY_FRAME (0x19). Frames are sent
   * in arrival order. Runs inside waitUntil() so it never blocks the WS
   * handshake; best-effort, so a closed socket mid-replay just stops.
   *
   * We snapshot the matching frames up front so concurrent appends to
   * `this.audioBuffer` (the live hot path keeps running) can't shift indices
   * out from under us. peer_id is emitted empty — see buildReplayFrame.
   */
  async replayBufferedAudio(ws, cutoff, clientId) {
    // Drop anything older than the TTL even if `since` reaches further back —
    // the buffer never retains beyond ~30 s anyway, but this keeps the cutoff
    // honest if a client sends an ancient `since`.
    const floor = Math.max(cutoff, Date.now() - BUFFER_TTL_MS);
    const pending = [];
    for (const entry of this.audioBuffer) {
      if (entry.ts > floor) pending.push(entry.frame);
    }
    if (pending.length === 0) return;

    // Pace the replay so a full ~1500-frame window doesn't land as one
    // synchronous burst. A small yield every CHUNK frames keeps the DO
    // responsive to live traffic and lets backpressure surface as a send
    // throw (which we treat as "peer gone" and stop).
    const CHUNK = 50; // ~1 s of audio per chunk at 50 fps
    let sent = 0;
    for (const frame of pending) {
      try {
        ws.send(buildReplayFrame(frame, ""));
      } catch {
        // Socket closed/failed mid-replay — stop; nothing else to do.
        return;
      }
      sent++;
      if (sent % CHUNK === 0) {
        // Yield to the event loop between chunks so live fan-out and the
        // sweeper aren't starved during a long catch-up. A microtask yield
        // (no timer, no `scheduler` global / compat-flag dependency) is enough
        // to let queued WS messages and the alarm run between chunks.
        await Promise.resolve();
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
