/**
 * SassyTalk PTT Relay — Dedicated Worker
 *
 * Pure WebSocket relay for encrypted audio. No website, no APIs, no assets.
 * Lives at relay.sassy-consults.com
 */

export { PttRoom } from "./ptt-relay.js";
import { handleShareRoute } from "./share.js";
import { handlePresenceRoute } from "./presence.js";

// Token lifetime in seconds. Short enough to limit replay risk, long enough
// that flaky cellular reconnects within the same session don't need a refresh.
const TOKEN_TTL_SEC = 300;

// Allowed clock skew between worker and client when verifying token.exp.
const CLOCK_SKEW_SEC = 30;

// Short-link targets. Keep the right-hand-side updated whenever the underlying
// R2 path or marketing-site download route changes — the public short URL
// stays stable (it is what gets baked into QR codes, posters, etc.).
const SHORT_LINKS = {
  "/dl/apk": "https://sassyconsultingllc.com/download/sassy-talk/android/sassytalkie.apk",
};

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname;

    if (request.method === "OPTIONS") {
      return new Response(null, { headers: CORS_HEADERS });
    }

    // Encrypted session-share endpoints. End-to-end: the worker never sees
    // plaintext or the key (key lives in the recipient URL's #fragment).
    const shareResp = await handleShareRoute(request, env, url);
    if (shareResp) return shareResp;

    // Presence: map (room, peer) → FCM token so audio for an offline peer
    // can trigger a wake push.
    const presenceResp = await handlePresenceRoute(request, env, url);
    if (presenceResp) return presenceResp;

    // Health check
    if (path === "/" || path === "/health") {
      return jsonResponse({
        service: "sassytalk-relay",
        status: "ok",
        max_peers_per_room: 16,
      });
    }

    // Issue an HMAC-signed token bound to (roomId, expiry). Client hits this
    // before opening the WebSocket. No PII required — the token is purely a
    // capability grant, and the audio payload itself is end-to-end encrypted.
    if (path === "/auth") {
      const roomId = url.searchParams.get("room");
      if (!isValidRoomId(roomId)) {
        return jsonResponse({ error: "Missing or invalid room ID" }, 400);
      }
      if (!env.AUTH_SECRET) {
        // Surface a clear error rather than silently issuing tokens with a
        // weak/empty key. Operator must `wrangler secret put AUTH_SECRET`.
        return jsonResponse({ error: "AUTH_SECRET not configured" }, 500);
      }
      const expSec = Math.floor(Date.now() / 1000) + TOKEN_TTL_SEC;
      const token = await signToken(roomId, expSec, env.AUTH_SECRET);
      // No CORS on the token response: native clients (OkHttp/reqwest) don't
      // enforce CORS, and withholding it stops a malicious web origin from
      // reading a freshly-minted room capability token.
      return jsonResponse({ token, expires_at: expSec, ttl: TOKEN_TTL_SEC }, 200, false);
    }

    // Short-link redirects. 302 (not 301) so we can re-point later without
    // browsers and link unfurlers permanently caching the old destination.
    if (SHORT_LINKS[path]) {
      return new Response(null, {
        status: 302,
        headers: {
          Location: SHORT_LINKS[path],
          // 5-minute redirect cache — small enough to roll a fix quickly,
          // large enough that QR-scan stampedes don't hammer the worker.
          "Cache-Control": "public, max-age=300",
          ...CORS_HEADERS,
        },
      });
    }

    // WebSocket relay
    if (path === "/ws" || path === "/api/ptt/ws") {
      const roomId = url.searchParams.get("room");
      if (!isValidRoomId(roomId)) {
        return new Response("Missing or invalid room ID", { status: 400 });
      }

      // Token verification. We do this in the worker so an attacker can't
      // even cause a DO to be instantiated (which costs $) without a valid
      // capability for that specific room. Fail CLOSED: a missing AUTH_SECRET
      // is a server misconfiguration, not a reason to drop room-scoping and
      // let anyone open any room.
      if (!env.AUTH_SECRET) {
        return new Response("Server misconfigured: AUTH_SECRET unset", { status: 503 });
      }
      const token = url.searchParams.get("token");
      const tokenError = await verifyToken(token, roomId, env.AUTH_SECRET);
      if (tokenError) {
        return new Response(tokenError, { status: 401 });
      }

      const doId = env.PTT_RELAY.idFromName(roomId);
      const room = env.PTT_RELAY.get(doId);
      return room.fetch(request);
    }

    return new Response("Not found", { status: 404 });
  },
};

function isValidRoomId(id) {
  return typeof id === "string" && id.length >= 8 && id.length <= 64;
}

function jsonResponse(body, status = 200, cors = true) {
  const headers = { "Content-Type": "application/json" };
  if (cors) Object.assign(headers, CORS_HEADERS);
  return new Response(JSON.stringify(body), { status, headers });
}

/**
 * Token format: "<expSec>.<hexSig>" where hexSig = HMAC-SHA256(roomId + "." + expSec, secret).
 * Compact, query-param-safe, no JSON parsing needed on either end.
 */
async function signToken(roomId, expSec, secret) {
  const sig = await hmacSha256Hex(`${roomId}.${expSec}`, secret);
  return `${expSec}.${sig}`;
}

async function verifyToken(token, roomId, secret) {
  if (!token || typeof token !== "string") return "Missing token";
  const dot = token.indexOf(".");
  if (dot <= 0) return "Malformed token";
  const expSec = Number.parseInt(token.slice(0, dot), 10);
  const sig = token.slice(dot + 1);
  if (!Number.isFinite(expSec) || !sig) return "Malformed token";

  const nowSec = Math.floor(Date.now() / 1000);
  if (expSec + CLOCK_SKEW_SEC < nowSec) return "Token expired";

  const expected = await hmacSha256Hex(`${roomId}.${expSec}`, secret);
  // Constant-time compare to avoid leaking byte-level timing.
  if (!timingSafeEqualHex(sig, expected)) return "Invalid token signature";
  return null;
}

async function hmacSha256Hex(data, secret) {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    enc.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, enc.encode(data));
  const bytes = new Uint8Array(sig);
  let hex = "";
  for (const b of bytes) hex += b.toString(16).padStart(2, "0");
  return hex;
}

function timingSafeEqualHex(a, b) {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}
