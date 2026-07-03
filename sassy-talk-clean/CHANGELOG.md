# Changelog

All notable changes to SassyTalkie. Format loosely follows
[Keep a Changelog](https://keepachangelog.com); versions map to Android
`versionName` (versionCode in parentheses).

## [2.9.0] (47) — 2026-07-03

### Added
- **Distribution flavors.** The app now builds as two variants with identical
  features and the same `applicationId`:
  - **Play** — free install with a one-time in-app unlock
    (`sassytalkie_unlock`) through Google Play Billing. Purchases restore
    automatically on reinstall or a second device; refunds revoke on the next
    online launch.
  - **Direct** (website APK) — unlocks with a license key or promo code.
    One license covers up to 3 devices; activation issues a 30-day offline
    receipt that silently renews whenever the app is online, so off-grid use
    keeps working.
- **License service** on the relay worker (`/license/*`): activation,
  revalidation, device deactivation, plus operator endpoints for issuing and
  revoking keys. Keys and device ids are stored only as salted HMACs — the
  server never persists a raw key. Keys carry 100 bits of CSPRNG entropy.
- **Promo codes**: shareable codes with redemption caps and optional expiry,
  entered in the same activation field. Device-bound and idempotent — a
  re-entry refreshes the unlock instead of consuming another redemption.
- **Verified App Links**: `assetlinks.json` now serves the release signing
  cert, so `https://relay.sassyconsultingllc.com/v/…` invite links open the
  sideloaded app directly instead of the browser landing page.

### Changed
- Invite deep-links imported while the app is locked now hold at the unlock
  screen (the session still imports and is live immediately after unlock).
- **Edge-to-edge display** using the modern API: content draws full-bleed
  behind transparent system bars with proper inset handling (fixes the Play
  Console "deprecated edge-to-edge APIs" and "may not display for all users"
  advisories on Android 15+).
- **Rotation and large screens**: the portrait lock is gone — the app now
  rotates and resizes on tablets, foldables, and split-screen. A live session
  (audio engine, transport, PTT) survives rotation without interruption.
- Release artifacts are flavor-qualified: website APK =
  `app-direct-release.apk`, Play AAB = `app-play-release.aab`; CI and the
  ship pipeline updated to match.

### Security
- Entitlement state lives in Android-Keystore-encrypted storage, same as
  session keys.
- License endpoints fail closed when server secrets are unconfigured.

## [2.8.1] (46) — 2026-06-25

### Added
- **Cross-platform invite links.** `https://relay…/v/<id>#<key>` now imports
  on Android, iOS, and desktop. The decryption key rides in the URL fragment
  and never reaches the server; the shared Rust core performs one audited
  AES-256-GCM open on every platform (byte-parity proven against Android's
  Java encrypter with an independent known-answer vector).
- Browser landing page for recipients without the app (`/v/<id>`): open-app /
  install chooser that resolves the key entirely client-side.
- `/.well-known/assetlinks.json` and `apple-app-site-association` served from
  the relay so tapped invites can open the app directly (Universal/App Links).
- Desktop: paste an invite link straight into the join field — auto-detected
  vs raw QR JSON. iOS: `onOpenURL` handler + associated-domains entitlement.

### Fixed
- Android share-link generation aligned to the worker `/share` contract —
  fixes the "no token" copy-link failure and dead invite links.

## [2.7.6] (45) — 2026-06-23

Baseline for this changelog: 16 KB page-size support, hybrid post-quantum
key exchange (X25519 + ML-KEM) with auto-upgrade on 2-party sessions,
WiFi → relay → Bluetooth transport fallback with auto-recovery, sealed-sender
blinding, group mix-mode, live transcription feed. Earlier history lives in
the git log.
