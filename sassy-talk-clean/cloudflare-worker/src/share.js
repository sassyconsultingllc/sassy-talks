/**
 * share.js — Encrypted session-share blobs for the SassyTalk relay.
 *
 * Lets one device hand a session invite to another over the internet without
 * the relay ever learning the session key. The CLIENT encrypts the invite
 * locally; only the ciphertext is uploaded. The decryption key travels in the
 * share link's URL #fragment, which browsers and apps never transmit to the
 * server — so the worker stores an opaque blob it cannot read.
 *
 * Consumed by ptt-relay-worker.js → handleShareRoute.
 *
 *   POST   /share          body = ciphertext bytes   → { id, expires_at }
 *   GET    /share/<id>      → ciphertext bytes (application/octet-stream)
 *   DELETE /share/<id>      → revoke early
 *
 * Optional one-time semantics: POST /share?burn=1 deletes the blob on first
 * successful GET (a dead-drop). KV layout: share:<id> in the SHARES namespace.
 */

// Invites are small (a key id, room id, codec params, signature). 64 KiB is
// generous headroom while bounding storage abuse from an unauthenticated POST.
const MAX_BLOB_BYTES = 64 * 1024;
const DEFAULT_TTL_SEC = 60 * 60 * 24 * 7;   // 7 days
const MAX_TTL_SEC = 60 * 60 * 24 * 30;      // 30 days ceiling
const MIN_TTL_SEC = 60;                      // 1 minute floor

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, DELETE, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type, Authorization",
};

const shareKey = (id) => `share:${id}`;
const ID_RE = /^[A-Za-z0-9_-]{16,64}$/;

export async function handleShareRoute(request, env, url) {
  const path = url.pathname;
  if (path !== "/share" && !path.startsWith("/share/")) return null;

  if (request.method === "OPTIONS") {
    return new Response(null, { headers: CORS_HEADERS });
  }
  if (!env.SHARES) return json({ error: "share store unavailable" }, 503);

  if (path === "/share" && request.method === "POST") {
    return createShare(request, env, url);
  }
  if (path.startsWith("/share/")) {
    const id = path.slice("/share/".length);
    if (!ID_RE.test(id)) return json({ error: "Invalid share id" }, 400);
    if (request.method === "GET") return readShare(env, id);
    if (request.method === "DELETE") return deleteShare(env, id);
  }
  return json({ error: "Method not allowed" }, 405);
}

async function createShare(request, env, url) {
  const buf = await request.arrayBuffer();
  if (!buf || buf.byteLength === 0) return json({ error: "Empty body" }, 400);
  if (buf.byteLength > MAX_BLOB_BYTES) return json({ error: "Blob too large" }, 413);

  let ttl = Number.parseInt(url.searchParams.get("ttl") || "", 10);
  if (!Number.isFinite(ttl)) ttl = DEFAULT_TTL_SEC;
  ttl = Math.max(MIN_TTL_SEC, Math.min(MAX_TTL_SEC, ttl));

  const burn = url.searchParams.get("burn") === "1";
  const id = generateId();
  const expiresAt = Math.floor(Date.now() / 1000) + ttl;

  await env.SHARES.put(shareKey(id), buf, {
    expirationTtl: ttl,
    metadata: { burn, createdAt: Date.now() },
  });

  return json({ id, expires_at: expiresAt, burn });
}

async function readShare(env, id) {
  const { value, metadata } = await env.SHARES.getWithMetadata(shareKey(id), { type: "arrayBuffer" });
  if (!value) return json({ error: "Not found or expired" }, 404);

  // One-time dead-drop: remove on first read. Best-effort — a racing double
  // GET could both succeed before the delete lands, which is acceptable for an
  // invite blob that's useless without the #fragment key anyway.
  if (metadata && metadata.burn) {
    try { await env.SHARES.delete(shareKey(id)); } catch { /* ignore */ }
  }

  return new Response(value, {
    status: 200,
    headers: {
      "Content-Type": "application/octet-stream",
      "Cache-Control": "no-store",
      ...CORS_HEADERS,
    },
  });
}

async function deleteShare(env, id) {
  await env.SHARES.delete(shareKey(id));
  return json({ ok: true });
}

/** URL-safe 16-byte random id (base64url, 22 chars). */
function generateId() {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...CORS_HEADERS },
  });
}
