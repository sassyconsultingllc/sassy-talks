// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-NQVZ3ZHQVPBR
/**
 * presence.js — FCM presence registry for the SassyTalk relay.
 *
 * Maps (room, peer) → FCM token in the SHARES KV namespace so the Durable
 * Object can wake offline peers with a push when a transmission starts.
 *
 * Consumed by:
 *   ptt-relay-worker.js → handlePresenceRoute (HTTP register/unregister)
 *   ptt-relay.js (DO)   → listPresence / dropPresence / getPresenceVersion
 *
 * Privacy: a presence row holds only a random per-install peer ID and an FCM
 * token. No audio, no content, no real identity. The token lets Google wake the
 * device; the push payload (see fcm.js) carries only a room id. Rows self-expire
 * (FCM tokens rotate), so an abandoned install leaves nothing behind for long.
 *
 * KV layout (all under the SHARES binding):
 *   presence:<roomId>:<peerId>   value = fcmToken   metadata = { peer, token }
 *   presence-ver:<roomId>        value = monotonically-increasing version int
 */

import { verifyCapabilityToken, extractToken, isValidRoomId, secretsFor } from "./relay-auth.js";

// FCM tokens rotate; a stale row should disappear on its own even if the app
// never gets the chance to DELETE it. 30 days comfortably outlives a token.
const PRESENCE_TTL_SEC = 60 * 60 * 24 * 30;
const MAX_PEER_ID_LEN = 64;
// FCM registration tokens are ~150–300 chars today; cap well above that but
// far below KV's 25 MiB value limit so a malformed client can't bloat storage.
const MAX_TOKEN_LEN = 4096;

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, DELETE, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type, Authorization",
};

const presenceKey = (roomId, peerId) => `presence:${roomId}:${peerId}`;
const presencePrefix = (roomId) => `presence:${roomId}:`;
const versionKey = (roomId) => `presence-ver:${roomId}`;

const isValidPeer = (p) => typeof p === "string" && p.length > 0 && p.length <= MAX_PEER_ID_LEN;
const isValidToken = (t) => typeof t === "string" && t.length > 0 && t.length <= MAX_TOKEN_LEN;

/**
 * HTTP entry point. Returns a Response for /presence, or null so the worker
 * can keep routing other paths.
 *
 * POST   /presence  { room, peer, fcm_token }  (Authorization: Bearer <capToken>)
 * DELETE /presence  { room, peer }             (Authorization: Bearer <capToken>)
 */
export async function handlePresenceRoute(request, env, url) {
  if (url.pathname !== "/presence") return null;

  if (request.method === "OPTIONS") {
    return new Response(null, { headers: CORS_HEADERS });
  }
  if (!env.SHARES) return json({ error: "presence store unavailable" }, 503);

  if (request.method === "POST") return registerPresence(request, env);
  if (request.method === "DELETE") return unregisterPresence(request, env);
  return json({ error: "Method not allowed" }, 405);
}

async function registerPresence(request, env) {
  const body = await safeJson(request);
  if (!body) return json({ error: "Invalid JSON body" }, 400);

  const room = body.room;
  const peer = body.peer;
  const token = body.fcm_token ?? body.token;
  if (!isValidRoomId(room)) return json({ error: "Missing or invalid room" }, 400);
  if (!isValidPeer(peer)) return json({ error: "Missing or invalid peer" }, 400);
  if (!isValidToken(token)) return json({ error: "Missing or invalid fcm_token" }, 400);

  const capErr = await verifyCapabilityToken(extractToken(request, new URL(request.url)), room, secretsFor(env));
  if (capErr) return json({ error: capErr }, 401);

  await env.SHARES.put(presenceKey(room, peer), token, {
    expirationTtl: PRESENCE_TTL_SEC,
    metadata: { peer, token },
  });
  await bumpVersion(env, room);
  return json({ ok: true });
}

async function unregisterPresence(request, env) {
  const body = await safeJson(request);
  if (!body) return json({ error: "Invalid JSON body" }, 400);

  const room = body.room;
  const peer = body.peer;
  if (!isValidRoomId(room)) return json({ error: "Missing or invalid room" }, 400);
  if (!isValidPeer(peer)) return json({ error: "Missing or invalid peer" }, 400);

  const capErr = await verifyCapabilityToken(extractToken(request, new URL(request.url)), room, secretsFor(env));
  if (capErr) return json({ error: capErr }, 401);

  await dropPresence(env, room, peer);
  return json({ ok: true });
}

/**
 * List all registered (peer, token) pairs for a room. Used by the DO's FCM
 * fan-out. Reads tokens straight from KV list metadata so a 16-peer room costs
 * a single list() call rather than 16 gets.
 */
export async function listPresence(env, roomId) {
  if (!env || !env.SHARES || !roomId) return [];
  const out = [];
  let cursor;
  do {
    const res = await env.SHARES.list({ prefix: presencePrefix(roomId), cursor, limit: 1000 });
    for (const k of res.keys) {
      const md = k.metadata;
      if (md && md.peer && md.token) {
        out.push({ peer: md.peer, token: md.token });
      } else {
        // Older row written without metadata — fall back to a value read.
        const peer = k.name.slice(presencePrefix(roomId).length);
        const token = await env.SHARES.get(k.name);
        if (peer && token) out.push({ peer, token });
      }
    }
    cursor = res.list_complete ? undefined : res.cursor;
  } while (cursor);
  return out;
}

/** Remove a single presence row and bump the room version. */
export async function dropPresence(env, roomId, peer) {
  if (!env || !env.SHARES || !roomId || !peer) return;
  await env.SHARES.delete(presenceKey(roomId, peer));
  await bumpVersion(env, roomId);
}

/**
 * Current presence version for a room. The DO caches its presence list and
 * compares this stamp to decide whether to refetch — lets a DELETE invalidate
 * the cache without an inter-DO RPC. Returns 0 when unset.
 */
export async function getPresenceVersion(env, roomId) {
  if (!env || !env.SHARES || !roomId) return 0;
  const v = await env.SHARES.get(versionKey(roomId));
  const n = v ? Number.parseInt(v, 10) : 0;
  return Number.isFinite(n) ? n : 0;
}

async function bumpVersion(env, roomId) {
  // Read-modify-write. KV is eventually consistent and we don't need a strict
  // counter — any change in value is enough to invalidate the DO's cache. A
  // lost increment under a race just means one extra presence refetch.
  const cur = await getPresenceVersion(env, roomId);
  // Roll over well before precision loss; the absolute value is irrelevant,
  // only that consecutive writes differ.
  const next = (cur + 1) % 1_000_000_000;
  await env.SHARES.put(versionKey(roomId), String(next), { expirationTtl: PRESENCE_TTL_SEC });
}

async function safeJson(request, maxBytes = 8192) {
  const len = parseInt(request.headers.get("Content-Length") || "0", 10);
  if (len > maxBytes) return null;
  try {
    const text = await request.text();
    if (text.length > maxBytes) return null;
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...CORS_HEADERS },
  });
}
