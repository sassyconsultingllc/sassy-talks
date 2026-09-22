/**
 * rate-limit.js — Coarse per-IP edge rate limiting for the SassyTalk relay.
 *
 * Cloudflare Workers have no shared in-process memory across isolates, so this
 * uses the SHARES KV namespace as a fixed-window counter keyed by
 * (bucket, client-ip, window-start). It is intentionally *coarse and
 * best-effort*:
 *   - KV is eventually consistent, so concurrent requests can undercount within
 *     a window. That is acceptable — the goal is to bound sustained abuse of the
 *     unauthenticated `/auth` mint and the KV-write `/presence` and `/share`
 *     paths, not to enforce an exact quota.
 *   - If KV is unavailable or the IP is unknown, requests are ALLOWED (fail
 *     open) so a KV hiccup can never take the relay down.
 *
 * KV's minimum expirationTtl is 60s, so windows must be >= 60s.
 */

// KV minimum TTL is 60s; add margin so a counter outlives its window.
const TTL_MARGIN_SEC = 10;

const rlKey = (bucket, id, windowStart) => `rl:${bucket}:${id}:${windowStart}`;

/**
 * Returns true if the caller has exceeded `limit` requests in the current
 * `windowSec` window for `bucket`, and should be rejected with 429.
 */
export async function rateLimited(env, bucket, id, limit, windowSec) {
  if (!env || !env.SHARES || !id || id === "unknown") return false; // fail open
  const win = Math.max(60, windowSec);
  const now = Math.floor(Date.now() / 1000);
  const windowStart = now - (now % win);
  const key = rlKey(bucket, id, windowStart);

  let count = 0;
  try {
    const cur = await env.SHARES.get(key);
    count = cur ? Number.parseInt(cur, 10) : 0;
    if (!Number.isFinite(count)) count = 0;
  } catch {
    return false; // read failure → don't block
  }

  if (count >= limit) return true;

  try {
    await env.SHARES.put(key, String(count + 1), { expirationTtl: win + TTL_MARGIN_SEC });
  } catch {
    // Write failure (e.g. KV per-key write rate) → best-effort, don't block.
  }
  return false;
}

/** Best-effort client IP from Cloudflare edge headers. */
export function clientIp(request) {
  return (
    request.headers.get("CF-Connecting-IP") ||
    request.headers.get("X-Forwarded-For") ||
    "unknown"
  );
}
