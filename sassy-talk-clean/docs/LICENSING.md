# SassyTalkie Monetization — Play Paywall + Direct License Keys

Two distribution flavors of the Android app (same `applicationId`, so a user
can move website → Play without losing data):

| Flavor  | Gate                              | Distribution                     | Build task |
|---------|-----------------------------------|----------------------------------|------------|
| `play`  | Google Play Billing, one-time IAP `sassytalkie_unlock` | Play Store (AAB) | `bundlePlayRelease` |
| `direct`| License key or promo code → relay worker `/license/*` | Website APK download       | `assembleDirectRelease` |

The gate lives in `Screen.Gate` (AppNavigation.kt) and is provided per-flavor
by `license/Entitlements.kt` in `src/play/` and `src/direct/`. Entitlement
state caches in EncryptedSharedPreferences (`license/LicenseStore.kt`).

## Play flavor

- Product: one-time in-app product `sassytalkie_unlock` — create it in Play
  Console → Monetize → In-app products, price **$3.99**, and set the
  app's Play listing itself to FREE (the paywall replaces the up-front price).
- Restore is automatic (queryPurchasesAsync on gate entry and on every launch
  via the silent refresh); refunds revoke on the next online launch.

## Direct flavor

Website checkout is **$3.99** via Lemon Squeezy (`/api/checkout` → `sassy-talk`
product). Paid orders issue a relay-side license key that unlocks the direct
APK. Friends & family can skip checkout with a **promo code** (same field on
the activation screen).

Server side is `cloudflare-worker/src/license.js` backed by D1
(`sassytalkie-licenses`, binding `LICENSES`) + two secrets:

```
LICENSE_SALT          # HMAC key: key/device hashing + receipt signing
LICENSE_ADMIN_TOKEN   # Bearer token for issue/revoke/info
```

D1 stores only HMAC-SHA256(LICENSE_SALT, key) — raw keys are shown once at
issue time and never persisted. Receipts are 30-day HMAC tokens; the app
revalidates opportunistically past half-life and rides the receipt through
offline stretches. Revoked keys die within 30 days everywhere.

### Operator workflows (curl)

```bash
RELAY=https://relay.sassyconsultingllc.com
ADMIN="Authorization: Bearer $LICENSE_ADMIN_TOKEN"

# Sell a license (website order) → send the key to the customer
curl -s -X POST $RELAY/license/issue -H "$ADMIN" \
  -d '{"email":"buyer@example.com","note":"stripe #1234"}'
# → {"ok":true,"keys":["SASSY-XXXXX-XXXXX-XXXXX-XXXXX"],"max_devices":3}

# Batch of 10 for a promo
curl -s -X POST $RELAY/license/issue -H "$ADMIN" -d '{"count":10,"note":"promo"}'

# Refund → kill the key (device receipts lapse within 30 days)
curl -s -X POST $RELAY/license/revoke -H "$ADMIN" -d '{"key":"SASSY-..."}'

# Support: what devices are on this key?
curl -s "$RELAY/license/info?key=SASSY-..." -H "$ADMIN"
```

### Promo codes

One shared code, capped redemptions, device-bound, same 30-day receipts.
The gate screen accepts either credential in the same field (license format
routes to /license/activate, anything else to /license/promo). Stored as
salted hashes like license keys; low entropy is inherent to shareable codes,
so caps + expiry are the abuse controls. Redemption is idempotent per device —
re-entry/revalidation never burns a slot.

```bash
# Create a promo (custom text, 6-40 chars A-Z 0-9 -), cap + optional expiry
curl -s -X POST $RELAY/license/promo-create -H "$ADMIN" \
  -d '{"code":"LAUNCH-2026","max_redemptions":250,"expires_days":90,"note":"launch week"}'

# Or let the server generate one (SASSYTALK-XXXXXX)
curl -s -X POST $RELAY/license/promo-create -H "$ADMIN" -d '{"max_redemptions":50}'

# Kill a leaked promo (existing devices lapse within 30 days)
curl -s -X POST $RELAY/license/promo-revoke -H "$ADMIN" -d '{"code":"LAUNCH-2026"}'

# How many redemptions so far?
curl -s "$RELAY/license/promo-info?code=LAUNCH-2026" -H "$ADMIN"
```

Play-flavor note: promo unlock is deliberately direct-only. For Play installs
use Play Console's native promo codes on the `sassytalkie_unlock` product —
bypassing Play Billing with an in-app code risks a policy strike.

## App Links (production)

`ANDROID_CERT_SHA256` in wrangler.toml `[vars]` currently carries the
release/upload keystore cert (verifies the direct/sideloaded APK):
`3C:AA:...:3C:56`. **Play installs are re-signed by Google** — append the Play
App Signing SHA-256 (Play Console → Test and release → App integrity) as a
comma-separated second fingerprint and redeploy, or App Links won't verify on
Play-installed builds. Check with:
`curl -sI https://relay.sassyconsultingllc.com/.well-known/assetlinks.json`
(header `X-Wellknown-Configured: true`).

### Release pipeline

`scripts/ship.sh <version>` now expects:

```
./gradlew assembleDirectRelease bundlePlayRelease
# website APK: app/build/outputs/apk/direct/release/app-direct-release.apk
# Play AAB:    app/build/outputs/bundle/playRelease/app-play-release.aab
```

CI (`.github/workflows/build-aab.yml`) builds/publishes the play-flavor AAB.

### Threat model notes

- Keys carry 100 bits of CSPRNG entropy — online guessing is a non-issue.
- A rooted device can bypass any client-side gate; accepted non-goal at $2.
- D1 leak yields only salted hashes (keys + device ids unrecoverable).
- Endpoints fail closed when LICENSE_SALT / LICENSE_ADMIN_TOKEN are unset.
