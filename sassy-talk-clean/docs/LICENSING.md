# SassyTalkie Monetization — Play Paywall + Direct License Keys

Two distribution flavors of the Android app (same `applicationId`, so a user
can move website → Play without losing data):

| Flavor  | Gate                              | Distribution                     | Build task |
|---------|-----------------------------------|----------------------------------|------------|
| `play`  | Google Play Billing, one-time IAP `sassytalkie_unlock` | Play Store (AAB) | `bundlePlayRelease` |
| `direct`| License key → relay worker `/license/*` | Website APK download       | `assembleDirectRelease` |

The gate lives in `Screen.Gate` (AppNavigation.kt) and is provided per-flavor
by `license/Entitlements.kt` in `src/play/` and `src/direct/`. Entitlement
state caches in EncryptedSharedPreferences (`license/LicenseStore.kt`).

## Play flavor

- Product: one-time in-app product `sassytalkie_unlock` — create it in Play
  Console → Monetize → In-app products, price $1.99/$2.49 tier, and set the
  app's Play listing itself to FREE (the paywall replaces the up-front price).
- Restore is automatic (queryPurchasesAsync on gate entry and on every launch
  via the silent refresh); refunds revoke on the next online launch.

## Direct flavor

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
