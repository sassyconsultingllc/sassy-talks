/**
 * SassyTalk PTT Relay — Dedicated Worker
 *
 * Pure WebSocket relay for encrypted audio. No website, no APIs, no assets.
 * Lives at relay.sassyconsultingllc.com
 */

export { PttRoom } from "./ptt-relay.js";
import { handleShareRoute } from "./share.js";
import { handlePresenceRoute } from "./presence.js";
// Single source of truth for token verification + key rotation. The /ws path
// uses the exact same verifier as /presence and /share so they can never drift
// apart on what tokens they accept (the inline copy this replaced had already
// diverged — it was missing a type guard in timingSafeEqualHex).
import {
  verifyCapabilityToken,
  secretsFor,
  hmacSha256Hex,
  isValidRoomId,
} from "./relay-auth.js";

// Token lifetime in seconds. Short enough to limit replay risk, long enough
// that flaky cellular reconnects within the same session don't need a refresh.
const TOKEN_TTL_SEC = 300;

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
      const tokenError = await verifyCapabilityToken(token, roomId, secretsFor(env));
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
