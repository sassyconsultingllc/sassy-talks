// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-APNSWAKEPUSH1
/**
 * apns.js — Apple Push Notification service wake-push sender for the relay.
 *
 * Consumed by wake-push.js → routeWakePush when presence.platform === "apns".
 *
 * Sends a background (content-available) push with the same wake custom keys
 * the iOS AppDelegate already handles (kind/type=wake, room, ts/sentAt). Never
 * puts audio in the push.
 *
 * Auth: JWT (ES256) minted from env.APNS_PRIVATE_KEY (.p8) with
 * APNS_KEY_ID / APNS_TEAM_ID. Host is api.push.apple.com, or
 * api.sandbox.push.apple.com when APNS_USE_SANDBOX is truthy.
 *
 * If any required secret is missing, skips and returns a non-secret reason.
 *
 * Returns { ok, status, error, stale } — same shape as fcm.js.
 */

const JWT_TTL_SEC = 50 * 60; // Apple allows up to 60m; refresh early
const TOKEN_SKEW_MS = 60_000;

let _jwtCache = null; // { key, jwt, expMs }

const REQUIRED = ["APNS_KEY_ID", "APNS_TEAM_ID", "APNS_PRIVATE_KEY", "APNS_BUNDLE_ID"];

function missingApnsSecrets(env) {
  if (!env) return REQUIRED.slice();
  return REQUIRED.filter((k) => {
    const v = env[k];
    return typeof v !== "string" || !v.trim();
  });
}

function useSandbox(env) {
  const v = env?.APNS_USE_SANDBOX;
  if (typeof v !== "string") return false;
  const s = v.trim().toLowerCase();
  return s === "1" || s === "true" || s === "yes";
}

export async function sendApnsWakePush(env, token, roomId) {
  const missing = missingApnsSecrets(env);
  if (missing.length) {
    const reason = `APNs not configured: missing ${missing.join(", ")}`;
    console.warn(reason);
    return { ok: false, status: 0, error: reason, stale: false };
  }
  if (!token) {
    return { ok: false, status: 0, error: "Empty token", stale: true };
  }

  let jwt;
  try {
    jwt = await getApnsJwt(env);
  } catch (e) {
    return { ok: false, status: 0, error: `APNs JWT mint failed: ${e.message}`, stale: false };
  }

  const host = useSandbox(env) ? "api.sandbox.push.apple.com" : "api.push.apple.com";
  const url = `https://${host}/3/device/${token}`;
  const wakeTs = String(Date.now());
  const payload = {
    aps: { "content-available": 1 },
    kind: "wake",
    type: "wake",
    room: String(roomId),
    ts: wakeTs,
    sentAt: wakeTs,
  };

  let res;
  try {
    res = await fetch(url, {
      method: "POST",
      headers: {
        authorization: `bearer ${jwt}`,
        "apns-topic": env.APNS_BUNDLE_ID.trim(),
        "apns-push-type": "background",
        "apns-priority": "5",
        "content-type": "application/json",
      },
      body: JSON.stringify(payload),
    });
  } catch (e) {
    return { ok: false, status: 0, error: `fetch failed: ${e.message}`, stale: false };
  }

  if (res.ok) return { ok: true, status: res.status, error: null, stale: false };

  let reason = "";
  try {
    const body = await res.json();
    reason = body?.reason || "";
  } catch { /* non-JSON */ }

  // 410 Gone / BadDeviceToken / Unregistered → drop the presence row
  const stale = res.status === 410
    || reason === "BadDeviceToken"
    || reason === "Unregistered"
    || reason === "DeviceTokenNotForTopic";
  return {
    ok: false,
    status: res.status,
    error: reason || `HTTP ${res.status}`,
    stale,
  };
}

async function getApnsJwt(env) {
  const keyId = env.APNS_KEY_ID.trim();
  const teamId = env.APNS_TEAM_ID.trim();
  const cacheKey = `${keyId}:${teamId}`;
  const now = Date.now();
  if (_jwtCache && _jwtCache.key === cacheKey && _jwtCache.expMs - TOKEN_SKEW_MS > now) {
    return _jwtCache.jwt;
  }

  const iat = Math.floor(now / 1000);
  const header = { alg: "ES256", kid: keyId };
  const claims = { iss: teamId, iat };
  const signingInput = `${b64url(JSON.stringify(header))}.${b64url(JSON.stringify(claims))}`;
  const key = await importEcPkcs8(env.APNS_PRIVATE_KEY);
  const sig = await crypto.subtle.sign(
    { name: "ECDSA", hash: "SHA-256" },
    key,
    new TextEncoder().encode(signingInput),
  );
  const jwt = `${signingInput}.${b64urlBytes(new Uint8Array(sig))}`;
  _jwtCache = { key: cacheKey, jwt, expMs: now + JWT_TTL_SEC * 1000 };
  return jwt;
}

async function importEcPkcs8(pem) {
  const der = pemToDer(pem);
  return crypto.subtle.importKey(
    "pkcs8",
    der,
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign"],
  );
}

function pemToDer(pem) {
  // Accept PEM with real newlines or literal \n from secret stores.
  const normalized = pem.includes("\\n") ? pem.replace(/\\n/g, "\n") : pem;
  const b64 = normalized
    .replace(/-----BEGIN [^-]+-----/, "")
    .replace(/-----END [^-]+-----/, "")
    .replace(/\s+/g, "");
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

function b64url(str) {
  return b64urlBytes(new TextEncoder().encode(str));
}

function b64urlBytes(bytes) {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Test helper: which required env names are absent (never returns values). */
export function apnsConfigGaps(env) {
  return missingApnsSecrets(env);
}
