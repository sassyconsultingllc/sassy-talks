# Self-Hosting the SassyTalk Relay

The SassyTalk relay is a single Cloudflare Worker + Durable Object. You can run
your **own** relay so that none of your traffic ever touches Sassy Consulting's
infrastructure — full data sovereignty for CJIS / FedRAMP / NDAA / air-gapped-
adjacent deployments. The relay only ever sees **end-to-end-encrypted ciphertext
plus routing metadata**; it cannot decrypt audio. Self-hosting moves even that
metadata into your own Cloudflare account.

> The relay is a blind forwarder. Self-hosting does not change the security
> model — it changes **who operates the box** and **whose logs** the connection
> metadata lands in.

---

## What you get

- A WebSocket relay at your own domain (e.g. `relay.example.org`).
- Encrypted session-share blobs (`/share`) and optional FCM wake-push
  (`/presence`) under your own KV namespace and your own Firebase project.
- Capability-token auth (`/auth`) signed with your own secret — no SassyTalk
  account, no shared trust with us.

## Prerequisites

- A Cloudflare account with a zone (domain) you control.
- `node` ≥ 18 and `npx wrangler` (v4). `npm install` in this directory pulls it.
- (Optional, for wake-push) a Firebase project with Cloud Messaging enabled and
  a **service-account JSON** (Project Settings → Service accounts → Generate key).

## 1. Point the worker at your domain

Edit `wrangler.toml`:

```toml
name = "sassytalk-relay"            # rename if you like
main = "src/ptt-relay-worker.js"
compatibility_date = "2024-12-01"

routes = [
  { pattern = "relay.example.org/*", zone_name = "example.org" }   # ← your zone
]
```

`account_id` is intentionally **not** committed — supply it via the
`CLOUDFLARE_ACCOUNT_ID` env var or your `wrangler login` profile.

## 2. Create your KV namespace

The relay stores encrypted share blobs and FCM presence rows in one KV
namespace bound as `SHARES`:

```bash
npx wrangler kv namespace create SHARES
npx wrangler kv namespace create SHARES --preview
```

Put the returned `id` / `preview_id` into the `[[kv_namespaces]]` block in
`wrangler.toml` (replace the values shipped in this repo — they point at our
namespace and you do not have access to them).

## 3. Set your secrets

```bash
# REQUIRED — HMAC key for room capability tokens. Fail-closed: without it the
# relay refuses every /ws upgrade.
openssl rand -hex 32 | npx wrangler secret put AUTH_SECRET

# OPTIONAL — only if you want offline-peer wake notifications. Paste the full
# Firebase service-account JSON when prompted. Omit entirely to disable push;
# the relay still forwards audio, it just won't wake backgrounded peers.
npx wrangler secret put FCM_SERVICE_ACCOUNT_JSON
```

## 4. Deploy

```bash
npm install
npx wrangler deploy
```

Verify:

```bash
curl https://relay.example.org/health
# → {"service":"sassytalk-relay","status":"ok","max_peers_per_room":16}
```

## 5. Point the app at your relay

The clients (Android `cellular_transport.rs`, desktop `transport/cellular.rs`)
target a relay base URL. Override it to your domain:

- **Build-time:** set the relay base host to `relay.example.org` where the
  client constructs the `/auth`, `/ws`, `/presence`, and `/share` URLs. Search
  the client for the relay host constant and replace it (a single base-URL
  constant; the path suffixes stay identical).
- **Runtime (recommended for fleets):** expose the relay base URL as a setting
  so an MDM/policy profile can point a managed fleet at your relay without a
  rebuild. (Wiring this as a user/admin setting is the one client change needed;
  the protocol and endpoints are unchanged.)

Both endpoints use the same token flow: client `GET /auth?room=<id>` → opens
`GET /ws?room=<id>&token=<t>`. As long as your `AUTH_SECRET` is set, tokens
minted by your relay are accepted only by your relay.

---

## Federation (multi-relay)

A room is keyed by `room id` and lives in exactly one Durable Object on one
relay. To bridge two independently-operated relays (e.g. two agencies that want
to interoperate without sharing infrastructure), run a **bridge peer**: a
headless client that joins the same logical room on both relays and forwards
ciphertext frames between them. Because frames are end-to-end encrypted, the
bridge — like the relays themselves — never sees plaintext; it only needs the
room id and a capability token on each relay. This keeps each operator's
metadata in their own account while letting encrypted audio cross the boundary.

> Sealed-sender note: when the metadata-resistance layer (`core::sealed`) is
> enabled, the room id on the wire is a per-epoch blinded handle derived from
> the shared session key. Bridges and relays handle it identically — it is just
> an opaque 8–64-char string to them — so federation composes cleanly with
> metadata resistance.

## Operational notes

- **Logs:** `[observability.logs]` is enabled in `wrangler.toml`. Logs capture
  connection events (room open, peer join/leave, rate-limit trips) — never audio
  content. Disable or tune retention to your policy.
- **Capacity:** `MAX_PEERS_PER_ROOM = 16` and a per-socket rate limit of 120
  msg/s are set in `src/ptt-relay.js`. Adjust for your deployment.
- **Cost:** idle rooms use WebSocket Hibernation, so an empty room consumes no
  compute. DO + KV usage bills to your account.
