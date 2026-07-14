/**
 * license.js — License-key issuance/activation for the direct-distribution
 * (website APK) build of SassyTalkie.
 *
 * The Play build never touches these endpoints (it uses Play Billing). The
 * direct build activates once, then revalidates opportunistically with a
 * signed receipt cached on-device.
 *
 * Privacy/hardening posture:
 *   - D1 stores only HMAC-SHA256(LICENSE_SALT, key) — a database leak does
 *     not yield activatable license keys. Raw keys exist exactly twice: in
 *     the /license/issue response (shown to the operator once) and on the
 *     customer's device.
 *   - Device ids are likewise stored as salted HMACs, never raw ANDROID_IDs.
 *   - Keys are 100 bits of CSPRNG entropy (crypto.getRandomValues) — online
 *     guessing is not a realistic threat, so there is no per-IP throttle here;
 *     Cloudflare WAF rate rules are the backstop if an abuser shows up.
 *   - Fail-closed: LICENSE_SALT unset → every /license/* request is rejected;
 *     LICENSE_ADMIN_TOKEN unset → admin routes are rejected.
 *
 * Endpoints (all JSON, no CORS — native clients only):
 *   POST /license/activate    {key, device_id, app_version?, device_name?}
 *                             → {ok, token, expires_at, devices_used, max_devices}
 *                             Binds a device slot (up to max_devices).
 *   POST /license/validate    {key, device_id}
 *                             → same shape; refreshes the receipt for an
 *                             already-activated device, never consumes a slot.
 *   POST /license/deactivate  {key, device_id} → {ok} — frees the slot.
 *
 * Promo codes — one shared code, capped redemptions, device-bound:
 *   POST /license/promo        {code, device_id, app_version?, device_name?}
 *                              → same receipt shape as activate. Idempotent per
 *                              device: re-redeeming refreshes the receipt
 *                              without consuming another redemption.
 *
 * Admin endpoints (Authorization: Bearer LICENSE_ADMIN_TOKEN):
 *   POST /license/issue        {count?, email?, note?, max_devices?} → {ok, keys:[...]}
 *   POST /license/revoke       {key} → {ok}
 *   GET  /license/info?key=SASSY-... → license row + active devices
 *   POST /license/promo-create {code?, max_redemptions?, note?, expires_days?}
 *                              → {ok, code, ...} (code generated when omitted)
 *   POST /license/promo-revoke {code} → {ok}
 *   GET  /license/promo-info?code=... → promo row + redemption count
 *
 * Receipt token format (mirrors relay-auth capability tokens):
 *   "<expSec>.<hexSig>"  where hexSig = HMAC-SHA256(`receipt.${keyHash}.${deviceHash}.${expSec}`, LICENSE_SALT)
 * The app treats expSec as its offline-entitlement horizon and revalidates
 * whenever it has network; the server re-checks revocation on every refresh.
 */

import { hmacSha256Hex, timingSafeEqualHex } from "./relay-auth.js";

// 30-day receipt: a paid user stays entitled through a month fully offline;
// a refunded/revoked key stops working within the same window.
const RECEIPT_TTL_SEC = 30 * 24 * 60 * 60;

const DEFAULT_MAX_DEVICES = 3;

// Per-IP throttle for the unauthenticated, low-entropy promo endpoint. Only
// failed guesses count (see promoGuessCount), so legit redeems/refreshes never
// trip it. Fails open when the KV binding is absent — Cloudflare WAF is the
// outer backstop.
const PROMO_GUESS_LIMIT = 15;
const PROMO_GUESS_WINDOW_SEC = 60 * 60;

// Crockford-ish alphabet: no 0/O/1/I lookalikes. 32 symbols = 5 bits each;
// 20 symbols = 100 bits of entropy per key.
const KEY_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const KEY_RE = /^SASSY(?:-[A-HJ-NP-Z2-9]{5}){4}$/;

/**
 * Route dispatcher. Returns a Response for /license/* paths, null otherwise
 * (mirrors handleShareRoute / handlePresenceRoute so the worker's fetch()
 * chain stays uniform).
 */
export async function handleLicenseRoute(request, env, url) {
  const path = url.pathname;
  if (!path.startsWith("/license/")) return null;

  if (!env.LICENSE_SALT) {
    return json({ ok: false, error: "License service not configured" }, 503);
  }
  if (!env.LICENSES) {
    return json({ ok: false, error: "License database not configured" }, 503);
  }

  try {
    if (request.method === "POST" && path === "/license/activate") {
      return await activate(request, env, /* consumeSlot */ true);
    }
    if (request.method === "POST" && path === "/license/validate") {
      return await activate(request, env, /* consumeSlot */ false);
    }
    if (request.method === "POST" && path === "/license/deactivate") {
      return await deactivate(request, env);
    }
    if (request.method === "POST" && path === "/license/promo") {
      return await redeemPromo(request, env);
    }

    // ── Admin surface ──
    const ADMIN_PATHS = [
      "/license/issue", "/license/revoke", "/license/info",
      "/license/promo-create", "/license/promo-revoke", "/license/promo-info",
    ];
    if (ADMIN_PATHS.includes(path)) {
      const denied = await requireAdmin(request, env);
      if (denied) return denied;
      if (request.method === "POST" && path === "/license/issue") return await issue(request, env);
      if (request.method === "POST" && path === "/license/revoke") return await revoke(request, env);
      if (request.method === "GET" && path === "/license/info") return await info(env, url);
      if (request.method === "POST" && path === "/license/promo-create") return await promoCreate(request, env);
      if (request.method === "POST" && path === "/license/promo-revoke") return await promoRevoke(request, env);
      if (request.method === "GET" && path === "/license/promo-info") return await promoInfo(env, url);
    }

    return json({ ok: false, error: "Not found" }, 404);
  } catch (err) {
    // D1 errors, malformed JSON bodies, etc. Never leak internals to clients.
    console.error("license route error:", err);
    return json({ ok: false, error: "Internal error" }, 500);
  }
}

// ── Client endpoints ──────────────────────────────────────────────────────

async function activate(request, env, consumeSlot) {
  const body = await readJson(request);
  if (!body) return json({ ok: false, error: "Malformed JSON body" }, 400);

  const key = normalizeKey(body.key);
  const deviceId = typeof body.device_id === "string" ? body.device_id.trim() : "";
  if (!key) return json({ ok: false, error: "Invalid license key format" }, 400);
  if (!deviceId || deviceId.length > 128) {
    return json({ ok: false, error: "Missing device_id" }, 400);
  }

  const keyHash = await hashKey(key, env);
  const deviceHash = await hashDevice(deviceId, env);

  const lic = await env.LICENSES.prepare(
    "SELECT key_hash, status, max_devices FROM licenses WHERE key_hash = ?",
  ).bind(keyHash).first();

  if (!lic) return json({ ok: false, error: "Unknown license key" }, 404);
  if (lic.status !== "active") {
    return json({ ok: false, error: `License ${lic.status}` }, 403);
  }

  const existing = await env.LICENSES.prepare(
    "SELECT device_hash FROM activations WHERE key_hash = ? AND device_hash = ?",
  ).bind(keyHash, deviceHash).first();

  if (existing) {
    await env.LICENSES.prepare(
      "UPDATE activations SET last_seen = datetime('now'), app_version = ? WHERE key_hash = ? AND device_hash = ?",
    ).bind(str(body.app_version), keyHash, deviceHash).run();
  } else {
    if (!consumeSlot) {
      // /validate never creates slots: a wiped device must go through
      // /activate so slot accounting stays honest.
      return json({ ok: false, error: "Device not activated" }, 403);
    }
    // Atomic slot claim: the count is evaluated INSIDE the insert, so two
    // concurrent /activate calls for different devices can't both clear a
    // separate COUNT check and overrun max_devices (TOCTOU). changes === 0
    // means the cap was already full.
    const claimed = await env.LICENSES.prepare(
      `INSERT INTO activations (key_hash, device_hash, device_name, app_version)
       SELECT ?, ?, ?, ?
       WHERE (SELECT COUNT(*) FROM activations WHERE key_hash = ?) < ?`,
    ).bind(keyHash, deviceHash, str(body.device_name), str(body.app_version), keyHash, lic.max_devices).run();
    if (!claimed.meta.changes) {
      return json(
        { ok: false, error: "Maximum devices reached", max_devices: lic.max_devices },
        403,
      );
    }
  }

  const usedNow = await env.LICENSES.prepare(
    "SELECT COUNT(*) AS n FROM activations WHERE key_hash = ?",
  ).bind(keyHash).first();

  const expSec = Math.floor(Date.now() / 1000) + RECEIPT_TTL_SEC;
  const sig = await hmacSha256Hex(`receipt.${keyHash}.${deviceHash}.${expSec}`, env.LICENSE_SALT);

  return json({
    ok: true,
    token: `${expSec}.${sig}`,
    expires_at: expSec,
    devices_used: usedNow?.n ?? 1,
    max_devices: lic.max_devices,
  });
}

async function deactivate(request, env) {
  const body = await readJson(request);
  if (!body) return json({ ok: false, error: "Malformed JSON body" }, 400);
  const key = normalizeKey(body.key);
  const deviceId = typeof body.device_id === "string" ? body.device_id.trim() : "";
  if (!key || !deviceId) return json({ ok: false, error: "Missing key or device_id" }, 400);

  const keyHash = await hashKey(key, env);
  const deviceHash = await hashDevice(deviceId, env);
  const res = await env.LICENSES.prepare(
    "DELETE FROM activations WHERE key_hash = ? AND device_hash = ?",
  ).bind(keyHash, deviceHash).run();

  if (!res.meta.changes) return json({ ok: false, error: "Activation not found" }, 404);
  return json({ ok: true });
}

// ── Promo codes ───────────────────────────────────────────────────────────

async function redeemPromo(request, env) {
  const body = await readJson(request);
  if (!body) return json({ ok: false, error: "Malformed JSON body" }, 400);

  const code = normalizePromo(body.code);
  const deviceId = typeof body.device_id === "string" ? body.device_id.trim() : "";
  if (!code) return json({ ok: false, error: "Invalid promo code" }, 400);
  if (!deviceId || deviceId.length > 128) {
    return json({ ok: false, error: "Missing device_id" }, 400);
  }

  // Promo codes are low-entropy and this endpoint is unauthenticated, so cap
  // guessing per source IP. Only failed lookups (below) count toward the limit,
  // so a legit device's redeem/refresh of a VALID code is never throttled.
  if ((await promoGuessCount(request, env, 0)) >= PROMO_GUESS_LIMIT) {
    return json({ ok: false, error: "Too many attempts; try again later" }, 429);
  }

  const codeHash = await hashPromo(code, env);
  const deviceHash = await hashDevice(deviceId, env);

  const promo = await env.LICENSES.prepare(
    "SELECT code_hash, status, max_redemptions, expires_at FROM promo_codes WHERE code_hash = ?",
  ).bind(codeHash).first();

  if (!promo) {
    await promoGuessCount(request, env, 1);
    return json({ ok: false, error: "Unknown promo code" }, 404);
  }
  if (promo.status !== "active") {
    return json({ ok: false, error: `Promo code ${promo.status}` }, 403);
  }
  if (promo.expires_at && new Date(promo.expires_at).getTime() < Date.now()) {
    return json({ ok: false, error: "Promo code expired" }, 403);
  }

  const existing = await env.LICENSES.prepare(
    "SELECT device_hash FROM promo_redemptions WHERE code_hash = ? AND device_hash = ?",
  ).bind(codeHash, deviceHash).first();

  if (existing) {
    // Idempotent refresh — this is also the direct client's revalidation
    // path, so a revoked/expired promo stops renewing receipts above.
    await env.LICENSES.prepare(
      "UPDATE promo_redemptions SET last_seen = datetime('now'), app_version = ? WHERE code_hash = ? AND device_hash = ?",
    ).bind(str(body.app_version), codeHash, deviceHash).run();
  } else {
    // Atomic redemption claim (see activate): the count is evaluated inside the
    // insert so concurrent redeems can't overrun max_redemptions (TOCTOU).
    const claimed = await env.LICENSES.prepare(
      `INSERT INTO promo_redemptions (code_hash, device_hash, device_name, app_version)
       SELECT ?, ?, ?, ?
       WHERE (SELECT COUNT(*) FROM promo_redemptions WHERE code_hash = ?) < ?`,
    ).bind(codeHash, deviceHash, str(body.device_name), str(body.app_version), codeHash, promo.max_redemptions).run();
    if (!claimed.meta.changes) {
      return json({ ok: false, error: "Promo code fully redeemed" }, 403);
    }
  }

  const expSec = Math.floor(Date.now() / 1000) + RECEIPT_TTL_SEC;
  const sig = await hmacSha256Hex(`receipt.promo.${codeHash}.${deviceHash}.${expSec}`, env.LICENSE_SALT);
  return json({ ok: true, token: `${expSec}.${sig}`, expires_at: expSec });
}

async function promoCreate(request, env) {
  const body = (await readJson(request)) ?? {};
  // Operator-chosen code (e.g. "SASSYVIP2026") or a generated one.
  const code = body.code != null ? normalizePromo(body.code) : generatePromoCode();
  if (!code) return json({ ok: false, error: "Invalid promo code format (6-40 chars, A-Z 0-9 -)" }, 400);

  const maxRedemptions = clampInt(body.max_redemptions, 1, 100000, 100);
  const expiresDays = clampInt(body.expires_days, 1, 3650, 0);
  const expiresAt = expiresDays
    ? new Date(Date.now() + expiresDays * 86400_000).toISOString()
    : null;

  const codeHash = await hashPromo(code, env);
  const dup = await env.LICENSES.prepare(
    "SELECT code_hash FROM promo_codes WHERE code_hash = ?",
  ).bind(codeHash).first();
  if (dup) return json({ ok: false, error: "Promo code already exists" }, 409);

  await env.LICENSES.prepare(
    "INSERT INTO promo_codes (code_hash, note, max_redemptions, expires_at) VALUES (?, ?, ?, ?)",
  ).bind(codeHash, str(body.note), maxRedemptions, expiresAt).run();

  // Like license keys: the raw code appears only in this response.
  return json({ ok: true, code, max_redemptions: maxRedemptions, expires_at: expiresAt });
}

async function promoRevoke(request, env) {
  const body = await readJson(request);
  const code = normalizePromo(body?.code);
  if (!code) return json({ ok: false, error: "Invalid promo code" }, 400);
  const codeHash = await hashPromo(code, env);
  const res = await env.LICENSES.prepare(
    "UPDATE promo_codes SET status = 'revoked' WHERE code_hash = ?",
  ).bind(codeHash).run();
  if (!res.meta.changes) return json({ ok: false, error: "Unknown promo code" }, 404);
  return json({ ok: true });
}

async function promoInfo(env, url) {
  const code = normalizePromo(url.searchParams.get("code"));
  if (!code) return json({ ok: false, error: "Invalid promo code" }, 400);
  const codeHash = await hashPromo(code, env);
  const promo = await env.LICENSES.prepare(
    "SELECT code_hash, note, status, max_redemptions, expires_at, created_at FROM promo_codes WHERE code_hash = ?",
  ).bind(codeHash).first();
  if (!promo) return json({ ok: false, error: "Unknown promo code" }, 404);
  const used = await env.LICENSES.prepare(
    "SELECT COUNT(*) AS n FROM promo_redemptions WHERE code_hash = ?",
  ).bind(codeHash).first();
  return json({ ok: true, promo, redemptions: used?.n ?? 0 });
}

// ── Admin endpoints ───────────────────────────────────────────────────────

async function requireAdmin(request, env) {
  if (!env.LICENSE_ADMIN_TOKEN) {
    return json({ ok: false, error: "Admin surface not configured" }, 503);
  }
  const auth = request.headers.get("Authorization") || "";
  const presented = auth.startsWith("Bearer ") ? auth.slice(7).trim() : "";
  // Hash both sides before comparing so timingSafeEqualHex's length check
  // doesn't leak the admin token's length.
  const a = await hmacSha256Hex(`admin.${presented}`, env.LICENSE_SALT);
  const b = await hmacSha256Hex(`admin.${env.LICENSE_ADMIN_TOKEN}`, env.LICENSE_SALT);
  return timingSafeEqualHex(a, b) ? null : json({ ok: false, error: "Unauthorized" }, 401);
}

async function issue(request, env) {
  const body = (await readJson(request)) ?? {};
  const count = clampInt(body.count, 1, 100, 1);
  const maxDevices = clampInt(body.max_devices, 1, 10, DEFAULT_MAX_DEVICES);

  const keys = [];
  for (let i = 0; i < count; i++) {
    const key = generateKey();
    const keyHash = await hashKey(key, env);
    await env.LICENSES.prepare(
      "INSERT INTO licenses (key_hash, email, note, max_devices) VALUES (?, ?, ?, ?)",
    ).bind(keyHash, str(body.email), str(body.note), maxDevices).run();
    keys.push(key);
  }
  // The only moment raw keys ever leave the worker. Deliver to the customer,
  // then this response is gone — there is no "look the key up later".
  return json({ ok: true, keys, max_devices: maxDevices });
}

async function revoke(request, env) {
  const body = await readJson(request);
  const key = normalizeKey(body?.key);
  if (!key) return json({ ok: false, error: "Invalid license key format" }, 400);
  const keyHash = await hashKey(key, env);
  const res = await env.LICENSES.prepare(
    "UPDATE licenses SET status = 'revoked' WHERE key_hash = ?",
  ).bind(keyHash).run();
  if (!res.meta.changes) return json({ ok: false, error: "Unknown license key" }, 404);
  return json({ ok: true });
}

async function info(env, url) {
  const key = normalizeKey(url.searchParams.get("key"));
  if (!key) return json({ ok: false, error: "Invalid license key format" }, 400);
  const keyHash = await hashKey(key, env);
  const lic = await env.LICENSES.prepare(
    "SELECT key_hash, email, note, status, max_devices, created_at FROM licenses WHERE key_hash = ?",
  ).bind(keyHash).first();
  if (!lic) return json({ ok: false, error: "Unknown license key" }, 404);
  const devices = await env.LICENSES.prepare(
    "SELECT device_name, app_version, first_seen, last_seen FROM activations WHERE key_hash = ?",
  ).bind(keyHash).all();
  return json({ ok: true, license: lic, devices: devices.results });
}

// ── Helpers ───────────────────────────────────────────────────────────────

function generateKey() {
  const bytes = new Uint8Array(20);
  crypto.getRandomValues(bytes);
  const groups = [];
  for (let g = 0; g < 4; g++) {
    let s = "";
    for (let i = 0; i < 5; i++) s += KEY_ALPHABET[bytes[g * 5 + i] % 32];
    groups.push(s);
  }
  return `SASSY-${groups.join("-")}`;
}

/** Uppercase, collapse whitespace, then enforce the canonical shape. */
function normalizeKey(raw) {
  if (typeof raw !== "string") return null;
  const key = raw.trim().toUpperCase().replace(/\s+/g, "");
  return KEY_RE.test(key) ? key : null;
}

// Promo codes are operator-chosen marketing strings, so the shape is loose:
// 6-40 chars of A-Z 0-9 and dashes after uppercasing. Low entropy is inherent
// to shareable codes — redemption caps and expiry are the abuse controls.
const PROMO_RE = /^[A-Z0-9-]{6,40}$/;
function normalizePromo(raw) {
  if (typeof raw !== "string") return null;
  const code = raw.trim().toUpperCase().replace(/\s+/g, "");
  // A license key pasted into the promo path is a user mistake, not a promo.
  if (KEY_RE.test(code)) return null;
  return PROMO_RE.test(code) ? code : null;
}

function generatePromoCode() {
  const bytes = new Uint8Array(6);
  crypto.getRandomValues(bytes);
  let tail = "";
  for (const b of bytes) tail += KEY_ALPHABET[b % 32];
  return `SASSYTALK-${tail}`;
}

/**
 * Best-effort guess counter for the promo endpoint, keyed by source IP in the
 * SHARES KV. `delta` 0 reads the current count; 1 records a failed guess.
 * Never throws and returns 0 when KV is unavailable, so it can only ADD
 * protection, never block a legitimate redemption.
 */
async function promoGuessCount(request, env, delta) {
  const kv = env.SHARES;
  if (!kv) return 0;
  const ip = request.headers.get("CF-Connecting-IP") || "unknown";
  const bucket = `promo-rl.${ip}`;
  try {
    const n = Number.parseInt((await kv.get(bucket)) || "0", 10) || 0;
    if (delta > 0) {
      await kv.put(bucket, String(n + delta), { expirationTtl: PROMO_GUESS_WINDOW_SEC });
    }
    return n;
  } catch {
    return 0;
  }
}

function hashKey(key, env) {
  return hmacSha256Hex(`lic.${key}`, env.LICENSE_SALT);
}

function hashPromo(code, env) {
  return hmacSha256Hex(`promo.${code}`, env.LICENSE_SALT);
}

function hashDevice(deviceId, env) {
  return hmacSha256Hex(`dev.${deviceId}`, env.LICENSE_SALT);
}

async function readJson(request) {
  try {
    return await request.json();
  } catch {
    return null;
  }
}

function str(v) {
  return typeof v === "string" ? v.slice(0, 200) : null;
}

function clampInt(v, min, max, dflt) {
  const n = Number.parseInt(v, 10);
  if (!Number.isFinite(n)) return dflt;
  return Math.min(max, Math.max(min, n));
}

function json(body, status = 200) {
  // Deliberately no CORS headers: these endpoints serve native apps and the
  // operator's curl, never a browser origin.
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}
