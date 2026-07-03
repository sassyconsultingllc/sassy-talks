import { describe, it, expect, beforeEach } from "vitest";
import { handleLicenseRoute } from "../src/license.js";
import { hmacSha256Hex } from "../src/relay-auth.js";

/**
 * Minimal in-memory D1 stand-in that understands exactly the statements
 * license.js issues. Statement matching is by SQL prefix — if license.js
 * grows new queries, extend the switch here.
 */
function mockD1() {
  const licenses = new Map(); // key_hash → row
  const activations = new Map(); // `${key_hash}|${device_hash}` → row

  return {
    licenses,
    activations,
    prepare(sql) {
      return {
        bind(...args) {
          return {
            async first() {
              if (sql.startsWith("SELECT key_hash, status, max_devices FROM licenses")) {
                return licenses.get(args[0]) ?? null;
              }
              if (sql.startsWith("SELECT device_hash FROM activations")) {
                return activations.get(`${args[0]}|${args[1]}`) ?? null;
              }
              if (sql.startsWith("SELECT COUNT(*) AS n FROM activations")) {
                let n = 0;
                for (const k of activations.keys()) if (k.startsWith(`${args[0]}|`)) n++;
                return { n };
              }
              if (sql.startsWith("SELECT key_hash, email, note, status")) {
                return licenses.get(args[0]) ?? null;
              }
              throw new Error(`mockD1.first: unhandled SQL: ${sql}`);
            },
            async all() {
              if (sql.startsWith("SELECT device_name, app_version")) {
                const results = [];
                for (const [k, v] of activations) if (k.startsWith(`${args[0]}|`)) results.push(v);
                return { results };
              }
              throw new Error(`mockD1.all: unhandled SQL: ${sql}`);
            },
            async run() {
              if (sql.startsWith("INSERT INTO licenses")) {
                licenses.set(args[0], {
                  key_hash: args[0], email: args[1], note: args[2],
                  status: "active", max_devices: args[3], created_at: "now",
                });
                return { meta: { changes: 1 } };
              }
              if (sql.startsWith("INSERT INTO activations")) {
                activations.set(`${args[0]}|${args[1]}`, {
                  device_name: args[2], app_version: args[3],
                  first_seen: "now", last_seen: "now",
                });
                return { meta: { changes: 1 } };
              }
              if (sql.startsWith("UPDATE activations SET last_seen")) {
                return { meta: { changes: 1 } };
              }
              if (sql.startsWith("DELETE FROM activations")) {
                const had = activations.delete(`${args[0]}|${args[1]}`);
                return { meta: { changes: had ? 1 : 0 } };
              }
              if (sql.startsWith("UPDATE licenses SET status = 'revoked'")) {
                const row = licenses.get(args[0]);
                if (row) row.status = "revoked";
                return { meta: { changes: row ? 1 : 0 } };
              }
              throw new Error(`mockD1.run: unhandled SQL: ${sql}`);
            },
          };
        },
      };
    },
  };
}

const SALT = "test-salt";
const ADMIN = "test-admin-token";

function makeEnv() {
  return { LICENSE_SALT: SALT, LICENSE_ADMIN_TOKEN: ADMIN, LICENSES: mockD1() };
}

function post(path, body, bearer) {
  const headers = { "Content-Type": "application/json" };
  if (bearer) headers.Authorization = `Bearer ${bearer}`;
  const url = new URL(`https://relay.example${path}`);
  return [new Request(url, { method: "POST", headers, body: JSON.stringify(body) }), url];
}

async function issueOne(env) {
  const [req, url] = post("/license/issue", { count: 1, max_devices: 2 }, ADMIN);
  const resp = await handleLicenseRoute(req, env, url);
  const json = await resp.json();
  return json.keys[0];
}

describe("license routes", () => {
  let env;
  beforeEach(() => { env = makeEnv(); });

  it("ignores non-license paths", async () => {
    const url = new URL("https://relay.example/auth");
    const resp = await handleLicenseRoute(new Request(url), env, url);
    expect(resp).toBeNull();
  });

  it("fails closed without LICENSE_SALT", async () => {
    env.LICENSE_SALT = undefined;
    const [req, url] = post("/license/activate", { key: "x", device_id: "d" });
    const resp = await handleLicenseRoute(req, env, url);
    expect(resp.status).toBe(503);
  });

  it("issues keys in canonical format and never stores them raw", async () => {
    const key = await issueOne(env);
    expect(key).toMatch(/^SASSY(-[A-HJ-NP-Z2-9]{5}){4}$/);
    for (const hash of env.LICENSES.licenses.keys()) {
      expect(hash).not.toContain(key);
      expect(hash).toMatch(/^[0-9a-f]{64}$/); // HMAC-SHA256 hex
    }
  });

  it("rejects admin routes with a bad bearer", async () => {
    const [req, url] = post("/license/issue", { count: 1 }, "wrong-token");
    const resp = await handleLicenseRoute(req, env, url);
    expect(resp.status).toBe(401);
  });

  it("activates an issued key and returns a verifiable receipt", async () => {
    const key = await issueOne(env);
    const [req, url] = post("/license/activate", { key, device_id: "device-A" });
    const resp = await handleLicenseRoute(req, env, url);
    const json = await resp.json();
    expect(resp.status).toBe(200);
    expect(json.ok).toBe(true);
    expect(json.devices_used).toBe(1);
    expect(json.max_devices).toBe(2);

    // Receipt = "<exp>.<hmac>" — recompute the signature server-side style.
    const [expSec, sig] = json.token.split(".");
    expect(Number(expSec)).toBeGreaterThan(Date.now() / 1000 + 29 * 24 * 3600);
    const keyHash = await hmacSha256Hex(`lic.${key}`, SALT);
    const devHash = await hmacSha256Hex("dev.device-A", SALT);
    const expected = await hmacSha256Hex(`receipt.${keyHash}.${devHash}.${expSec}`, SALT);
    expect(sig).toBe(expected);
  });

  it("re-activation of the same device does not consume a slot", async () => {
    const key = await issueOne(env);
    for (let i = 0; i < 3; i++) {
      const [req, url] = post("/license/activate", { key, device_id: "device-A" });
      const json = await (await handleLicenseRoute(req, env, url)).json();
      expect(json.devices_used).toBe(1);
    }
  });

  it("enforces max_devices", async () => {
    const key = await issueOne(env); // max_devices: 2
    for (const d of ["d1", "d2"]) {
      const [req, url] = post("/license/activate", { key, device_id: d });
      expect((await handleLicenseRoute(req, env, url)).status).toBe(200);
    }
    const [req, url] = post("/license/activate", { key, device_id: "d3" });
    const resp = await handleLicenseRoute(req, env, url);
    expect(resp.status).toBe(403);
    expect((await resp.json()).error).toMatch(/Maximum devices/);
  });

  it("deactivate frees a slot", async () => {
    const key = await issueOne(env);
    for (const d of ["d1", "d2"]) {
      const [req, url] = post("/license/activate", { key, device_id: d });
      await handleLicenseRoute(req, env, url);
    }
    let [req, url] = post("/license/deactivate", { key, device_id: "d1" });
    expect((await handleLicenseRoute(req, env, url)).status).toBe(200);
    [req, url] = post("/license/activate", { key, device_id: "d3" });
    expect((await handleLicenseRoute(req, env, url)).status).toBe(200);
  });

  it("validate refreshes an activated device but never creates a slot", async () => {
    const key = await issueOne(env);
    let [req, url] = post("/license/validate", { key, device_id: "device-A" });
    expect((await handleLicenseRoute(req, env, url)).status).toBe(403);

    [req, url] = post("/license/activate", { key, device_id: "device-A" });
    await handleLicenseRoute(req, env, url);
    [req, url] = post("/license/validate", { key, device_id: "device-A" });
    const resp = await handleLicenseRoute(req, env, url);
    expect(resp.status).toBe(200);
    expect((await resp.json()).ok).toBe(true);
  });

  it("revoked keys stop activating and validating", async () => {
    const key = await issueOne(env);
    let [req, url] = post("/license/activate", { key, device_id: "device-A" });
    await handleLicenseRoute(req, env, url);

    [req, url] = post("/license/revoke", { key }, ADMIN);
    expect((await handleLicenseRoute(req, env, url)).status).toBe(200);

    [req, url] = post("/license/validate", { key, device_id: "device-A" });
    const resp = await handleLicenseRoute(req, env, url);
    expect(resp.status).toBe(403);
    expect((await resp.json()).error).toMatch(/revoked/);
  });

  it("rejects unknown and malformed keys", async () => {
    let [req, url] = post("/license/activate", { key: "SASSY-AAAAA-BBBBB-CCCCC-DDDDD", device_id: "d" });
    expect((await handleLicenseRoute(req, env, url)).status).toBe(404);
    [req, url] = post("/license/activate", { key: "not-a-key", device_id: "d" });
    expect((await handleLicenseRoute(req, env, url)).status).toBe(400);
  });

  it("normalizes key case and whitespace", async () => {
    const key = await issueOne(env);
    const sloppy = ` ${key.toLowerCase()} `;
    const [req, url] = post("/license/activate", { key: sloppy, device_id: "d" });
    expect((await handleLicenseRoute(req, env, url)).status).toBe(200);
  });

  it("admin info reports devices", async () => {
    const key = await issueOne(env);
    const [aReq, aUrl] = post("/license/activate", { key, device_id: "d1", device_name: "Pixel" });
    await handleLicenseRoute(aReq, env, aUrl);

    const url = new URL(`https://relay.example/license/info?key=${key}`);
    const req = new Request(url, { headers: { Authorization: `Bearer ${ADMIN}` } });
    const resp = await handleLicenseRoute(req, env, url);
    const json = await resp.json();
    expect(json.license.status).toBe("active");
    expect(json.devices).toHaveLength(1);
    expect(json.devices[0].device_name).toBe("Pixel");
  });
});
