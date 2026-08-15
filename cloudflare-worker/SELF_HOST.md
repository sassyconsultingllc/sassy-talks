<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-QD4QRHEVGZH5
-->
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

## 3. Set your secrets and auth policy

```bash
# REQUIRED — HMAC key for room capability tokens. Fail-closed: without it the
# relay refuses every /ws upgrade.
openssl rand -hex 32 | npx wrangler secret put AUTH_SECRET

# OPTIONAL — only if you want offline-peer wake notifications. Paste the full
# Firebase service-account JSON when prompted. Omit entirely to disable push;
# the relay still forwards audio, it just won't wake backgrounded peers.
npx wrangler secret put FCM_SERVICE_ACCOUNT_JSON
```

`AUTH_SECRET` is the only relay-auth secret. The proof flow deliberately reuses
it; do not create or commit a second key. `SHARES` is also used for short-lived
proof-nonce replay records.

The default auth policy is compatibility mode:

- Existing clients calling `GET /auth?room=<id>` receive the legacy v1
  room-bound token and continue to work.
- Updated tooling can add `peer=<stable-id>` and receives a v2 token bound to
  room, peer, expiry, and auth class. The `/ws` and `/presence` routes reject a
  peer claim that differs from a v2 token.
- `auth_class=terminal` always requires the versioned proof below. Reserved
  privileged text controls also fail closed: ordinary clients are rejected and
  terminal-class clients receive `privileged_control_unsupported` until a
  reviewed server-side operation exists.

For a managed deployment, after **every client has been upgraded to send proof
headers**, enable proof-required issuance:

```toml
[vars]
REQUIRE_AUTH_PROOF = "true"
```

Do not enable this against the currently shipped Android/iOS/desktop clients:
they use legacy `GET /auth?room=...` and will receive HTTP 400 until their auth
request includes a peer identity and proof. This switch is intentionally
fail-closed.

### Versioned proof format

Trusted terminal/operator tooling that already holds `AUTH_SECRET` sends:

```text
GET /auth?room=<room>&peer=<peer>&auth_class=terminal
X-Sassy-Auth-Version: 1
X-Sassy-Auth-Timestamp: <unix seconds>
X-Sassy-Auth-Nonce: <16-128 URL-safe random characters>
X-Sassy-Auth-Proof: <lowercase hex HMAC-SHA256>
```

The signed bytes are exactly:

```text
v1\n<auth_class>\n<room>\n<peer>\n<timestamp>\n<nonce>
```

The timestamp must be within 60 seconds of the Worker clock. A nonce is accepted
once for 120 seconds (subject to Cloudflare KV's eventual-consistency model).
Never put `AUTH_SECRET` itself in a URL, log, mobile build, or browser bundle.

Important boundary: this proves possession of the **relay operator secret** and
binds the issued token to a claimed room and peer. It is not proof of the
end-to-end room encryption key. The blind relay intentionally never receives
that key, so it cannot validate a room-key HMAC without weakening the E2E
design. In normal client compatibility mode, the unguessable room identifier
(or sealed-sender key-derived room handle) remains the membership capability.

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

Legacy clients use `GET /auth?room=<id>` and then
`GET /ws?room=<id>&token=<t>`. Identity-aware clients request
`GET /auth?room=<id>&peer=<peer>` and must use the same peer on `/ws` and
`/presence`. Tokens minted by one relay are accepted only by a relay with the
same active `AUTH_SECRET`.

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
- **Key rotation:** `AUTH_SECRET_PREV` is accepted only while
  `AUTH_SECRET_PREV_UNTIL` (Unix seconds) is in the future. Set both before
  replacing `AUTH_SECRET`; remove them after the bounded grace period.
- **Required config:** `PTT_RELAY` Durable Object, `SHARES` KV, and
  `AUTH_SECRET`. Proof-required/terminal issuance fails if `SHARES` is absent.
  `FCM_SERVICE_ACCOUNT_JSON` is optional and only enables wake pushes.
