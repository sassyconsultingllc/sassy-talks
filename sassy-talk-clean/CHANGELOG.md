# Changelog

All notable changes to SassyTalkie. Format loosely follows
[Keep a Changelog](https://keepachangelog.com); versions map to Android
`versionName` (versionCode in parentheses).

## [3.1.1] (52) — 2026-07-07

### Added
- **Play paywall promo redemption.** Friends & family can enter a promo code
  on the Play build paywall (relay `/license/promo`) without a Google Play
  purchase. Receipt refresh mirrors the direct-license offline model.

## [3.1.0] (51) — 2026-07-07

### Added
- **Picture-in-picture.** Leaving the main radio screen while connected
  auto-enters PiP with channel, transport, and TX/RX status.
- **Efficient QR rendering.** Shared `QrBitmap` utility generates at display
  size with bulk pixel writes instead of oversized `setPixel` loops.

### Changed
- **Edge-to-edge (Android 15).** API 35 theme disables contrast scrims,
  cutout mode is `always`, navigation bar uses dark icons, and insets are
  applied once at the activity root (no double-padding on child screens).
- **Predictive back** enabled for Android 13+.
- Removed dead legacy XML layout, unused drawables, and WalkieService
  network-type polling (badge was removed earlier).
- Profile and auth screens use `imePadding()` for keyboard overlap.
- PiP overlay live-updates channel/transport; auto-enter enabled on Android 12+.
- End-session and entitlement refresh use coordinator-safe threading.
- **Android 12+ splash screen** with branded slate background on cold start.
- Removed unused AppCompat/Material/ConstraintLayout dependencies (~legacy XML).

## [3.0.2] (50) — 2026-07-07

### Fixed
- **Paywall infinite loading.** Play billing catalog load now times out after
  15s, surfaces errors when the unlock product is missing or Play is
  unavailable, and offers Retry / Restore instead of a stuck "Loading…" button.

## [3.0.1] (49) — 2026-07-07

### Fixed
- **PTT orchestration.** All transmit paths (on-screen, notification shade,
  hardware key, Bluetooth media button) now route through `PttCoordinator` so
  BLE wake, RFCOMM pump, and reach watchdog run consistently.
- **"Not reaching peer" false alarm.** Main screen now uses the watchdog's
  `peerReachFailed` signal instead of an inverted reach indicator.
- **Users tab offline labels.** Registry peers without heartbeats yet show
  "On channel"; "Out of contact" only when liveness tracking confirms STALE.
- **Relay WebSocket leak.** Reconnecting cellular relay tears down the
  previous client before opening a new one.
- **Peer registry eviction.** Peers are removed from the user registry when
  they leave the channel.
- **UI polish.** Centralized cache-status polling, QR generation off the main
  thread, fixed snackbar overlay layout, hold-vs-tap PTT hint text, and
  deduplicated incoming-audio vs cache strip display.

## [3.0.0] (48) — 2026-07-07

### Added
- **Transport Advisor.** The app now scores available encrypted audio planes
  (WiFi multicast, Cloudflare relay, Bluetooth) and tells you which path is
  active and whether a better one is reachable. When cellular is down, it
  confirms Bluetooth fallback; when WiFi returns, it advises switching for
  lower latency. Advisories appear inline on the main screen and as toasts on
  meaningful path changes.
- **Active audio-plane badge.** Connection status now shows the encrypted PTT
  transport plane (not just the OS network type), color-coded: green = WiFi,
  orange = relay, cyan = Bluetooth.

### Changed
- Consolidates the v2.9.0 paywall (Play IAP + direct license key) with the
  full multi-transport encrypted PTT stack into the shipping 3.0 release.

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
