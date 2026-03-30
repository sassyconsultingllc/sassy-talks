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

export class PttRoom extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    // Map<WebSocket, { id: string, device: string, joinedAt: number }>
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

    const clientId = url.searchParams.get("client_id") || crypto.randomUUID();
    const rawDevice = url.searchParams.get("device") || "Unknown";
    const device = decodeURIComponent(rawDevice).substring(0, MAX_DEVICE_NAME_LEN);

    const session = {
      id: clientId,
      device,
      joinedAt: Date.now(),
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

    if (typeof message === "string") {
      // Text control message
      try {
        const parsed = JSON.parse(message);
        if (parsed.type === "ping") {
          ws.send(JSON.stringify({ type: "pong", ts: Date.now() }));
        }
      } catch {
        // Ignore malformed JSON
      }
      return;
    }

    // Binary message = encrypted audio frame → broadcast to all OTHER peers
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
