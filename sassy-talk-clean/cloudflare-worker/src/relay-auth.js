/**
 * relay-auth.js — Shared capability-token helpers for the SassyTalk relay.
 *
 * Token format mirrors ptt-relay-worker.js exactly so a token minted by
 * GET /auth?room=... is accepted by every endpoint:
 *
 *   token = "<expSec>.<hexSig>"  where  hexSig = HMAC-SHA256(`${roomId}.${expSec}`, AUTH_SECRET)
 *
 * presence.js and share.js import verifyCapabilityToken from here so that
 * registering an FCM token / minting a share blob requires the same room
 * capability as opening the WebSocket. The worker keeps its own inline copy on
 * the hot /ws path; this module is the single source of truth for the rest.
 */

// Allowed clock skew between worker and client when verifying token.exp.
const CLOCK_SKEW_SEC = 30;

/**
 * Verify a capability token against a room id.
 * Returns null on success, or a human-readable error string on failure.
 * Fails CLOSED: with no active key configured this rejects every token.
 *
 * `secret` accepts a single key or a list of active keys. During an AUTH_SECRET
 * rotation, pass secretsFor(env) = [AUTH_SECRET, AUTH_SECRET_PREV] so tokens
 * minted under the outgoing key keep verifying until they expire. Tokens are
 * always *signed* with the current AUTH_SECRET only.
 */
export async function verifyCapabilityToken(token, roomId, secret) {
  const secrets = (Array.isArray(secret) ? secret : [secret]).filter(Boolean);
  if (secrets.length === 0) return "AUTH_SECRET not configured";
  if (!token || typeof token !== "string") return "Missing token";
  const dot = token.indexOf(".");
  if (dot <= 0) return "Malformed token";
  const expSec = Number.parseInt(token.slice(0, dot), 10);
  const sig = token.slice(dot + 1);
  if (!Number.isFinite(expSec) || !sig) return "Malformed token";

  const nowSec = Math.floor(Date.now() / 1000);
  if (expSec + CLOCK_SKEW_SEC < nowSec) return "Token expired";

  // Accept a signature from any active key. Loop over all candidates without
  // short-circuiting so verification time doesn't reveal which key matched.
  let ok = false;
  for (const s of secrets) {
    const expected = await hmacSha256Hex(`${roomId}.${expSec}`, s);
    if (timingSafeEqualHex(sig, expected)) ok = true;
  }
  return ok ? null : "Invalid token signature";
}

/**
 * Active verification keys, current first: [AUTH_SECRET] normally, or
 * [AUTH_SECRET, AUTH_SECRET_PREV] during a rotation grace window.
 *
 * Zero-downtime rotation:
 *   1. wrangler secret put AUTH_SECRET_PREV   (= the current AUTH_SECRET value)
 *   2. wrangler secret put AUTH_SECRET        (= a fresh `openssl rand -hex 32`)
 *   3. wait one token lifetime (TOKEN_TTL_SEC) for old tokens to expire
 *   4. wrangler secret delete AUTH_SECRET_PREV
 */
export function secretsFor(env) {
  return [env.AUTH_SECRET, env.AUTH_SECRET_PREV].filter(Boolean);
}

/**
 * Pull the capability token from an incoming request: prefer the
 * `Authorization: Bearer <token>` header, fall back to a `?token=` query param
 * (handy for browser-opened share links that can't set headers).
 */
export function extractToken(request, url) {
  const auth = request.headers.get("Authorization") || "";
  if (auth.startsWith("Bearer ")) return auth.slice(7).trim();
  return url.searchParams.get("token");
}

export async function hmacSha256Hex(data, secret) {
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

export function timingSafeEqualHex(a, b) {
  if (typeof a !== "string" || typeof b !== "string") return false;
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

export function isValidRoomId(id) {
  return typeof id === "string" && id.length >= 8 && id.length <= 64;
}
