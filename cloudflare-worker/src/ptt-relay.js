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

const MAX_PEERS_PER_ROOM = 16;
const MAX_DEVICE_NAME_LEN = 100;
const HEARTBEAT_STALE_MS = 8_000;
const SWEEP_INTERVAL_MS  = 2_000;

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
    // Map<WebSocket, { id: string, device: string, joinedAt: number, lastSeenMs: number }>
    // Restored from serialized attachments on wake-up
    this.sessions = new Map();
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

    const session = {
      id: clientId,
      device,
      joinedAt: Date.now(),
      lastSeenMs: Date.now(),
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

    // Start sweeper alarm if not already running
    if (!(await this.ctx.storage.getAlarm())) {
      await this.ctx.storage.setAlarm(Date.now() + SWEEP_INTERVAL_MS);
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
        this.sessions.set(ws, attachment);
      }
    }

    if (typeof message !== "string") {
      // Binary message
      const bytes = message instanceof ArrayBuffer
        ? new Uint8Array(message)
        : new Uint8Array(message.buffer, message.byteOffset, message.byteLength);

      // Track heartbeat for liveness
      // Verify this is a valid heartbeat TLV (opcode 0x10, payload exactly 23 bytes)
      // to avoid collision with audio frames whose first length byte happens to be 0x10.
      if (bytes.length >= 3 && bytes[0] === 0x10) {
        const payloadLen = bytes[1] | (bytes[2] << 8);
        if (payloadLen === 23 && bytes.length >= 26) {
          const session = this.sessions.get(ws);
          if (session) {
            session.lastSeenMs = Date.now();
            ws.serializeAttachment(session);
          }
        }
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
    // Re-arm based on count of non-closed sockets observed during the sweep,
    // because ws.close() may not remove the socket from getWebSockets() in the same tick.
    if (liveCount > 0) {
      await this.ctx.storage.setAlarm(Date.now() + SWEEP_INTERVAL_MS);
    }
  }

  /**
   * Called when a WebSocket connection closes.
   */
  async webSocketClose(ws, code, reason, wasClean) {
    const session = this.sessions.get(ws) || ws.deserializeAttachment() || {};
    this.sessions.delete(ws);
    try { ws.close(code, reason); } catch { /* already closed */ }

    // Notify remaining peers
    const leaveMsg = JSON.stringify({
      type: "peer_left",
      client_id: session.id || "unknown",
      device: session.device || "Unknown",
      peers: this.ctx.getWebSockets().length,
    });

    const sockets = this.ctx.getWebSockets();
    for (const peer of sockets) {
      try {
        peer.send(leaveMsg);
      } catch {
        // Dead socket — will be cleaned up on next message or close
        this.sessions.delete(peer);
      }
    }

    // Also push binary PARTNER_OFFLINE TLV for clients that parse binary control
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
