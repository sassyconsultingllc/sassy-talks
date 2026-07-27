<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-TRE5664GFP4D
-->
# Changelog

All notable changes to SassyTalkie. Format loosely follows
[Keep a Changelog](https://keepachangelog.com); versions map to Android
`versionName` (versionCode in parentheses).

## [3.1.13] (65) — 2026-07-27

### Fixed
- **Bluetooth silent TX.** Failover/connected/PTT now require an RFCOMM data
  link (BLE alone is control-plane). RFCOMM retry + “Linking Bluetooth…” UI;
  PTT refuses with a clear reason when no audio path is ready.
- **Minimized loudspeaker jitter.** FGS holds AudioFocus, renews WakeLock on
  inbound RX, and bumps jitter prebuffer while backgrounded.
- **Share “Open in SassyTalk”.** Viewer uses App Link → custom scheme →
  intent:// with paste fallback (worker deploy required for live page).

### Changed
- iOS `MARKETING_VERSION` aligned to **3.1.13** / build **65**.
- GitHub Actions **Build iOS** now runs on published releases so macOS
  runners can produce the matching iOS build automatically.

## [3.1.6] (58) — 2026-07-15

### Fixed
- **Permission-screen crash on notification deny.** POST_NOTIFICATIONS (and
  Bluetooth) are now optional: startup gates only on mic + camera, the results
  callback no longer auto-re-requests (a permanently-denied permission answered
  instantly, looping the request until ANR), and a permanently-denied core
  permission routes to the app's Settings page instead of a dead button.
- **Dead PTT button while roster shows peers.** The on-screen PTT now falls
  back to the native IP pipeline when `PttCoordinator` is absent (Bluetooth
  off/unpermitted or entitlement not yet cached at init) — the same fallback
  the notification-shade toggle and hardware PTT already had. BT transport
  init also retries on entitlement unlock and Main-screen mount instead of
  being skipped once per session, and a rejected press now explains itself on
  the hint line instead of eating the touch.
- **Keyboard resize.** IME insets were applied twice (root `safeDrawing` +
  per-screen `imePadding`), shifting content by 2x the keyboard height; the
  root now excludes IME and `windowSoftInputMode="adjustResize"` pins the
  behavior across OEM defaults.
- **QR session screen.** Continue renders directly above the generated QR
  (was: below the share buttons inside a scrolling column — off-screen on
  phones), the inner scroll is gone, and the duplicate Active Session card is
  suppressed while the generated QR is on screen.
- **Paywall promo/license entry hidden by keyboard.** Both gate screens now
  scroll and imePadding so the code field rides above the IME (typed text was
  invisible under the keyboard); typed text is white on a filled container
  with a teal cursor for contrast.
- **Session crypto wiped by back-navigation.** The radio screen's back arrow
  called native `disconnect()`, which nulls the transport's AEAD session —
  re-entering via Continue then rejected every PTT press ("Authenticate via
  QR first") while presence still showed peers. Back is now pure navigation
  (End Session remains the hard teardown), and Continue re-arms crypto from
  the persisted channel session if a wipe already happened.
- **Debug builds re-locked onto the paywall.** `refresh()` lacked the debug
  entitlement bypass `isUnlockedCached` has, so the silent post-startup
  reconciliation flipped dev installs onto the gate a beat after launch.

### Added
- **Invite links inside the session.** The radio screen's Session QR dialog
  now has the same Copy Link / Share Link actions as the Auth screen — no
  more backing out of the session to mint an invite.

## [3.1.5] (57) — 2026-07-14

### Added
- **In-app diagnostics panel.** Settings toggle shows live relay room, cellular
  stats, peer counts, and copyable debug dump for remote join troubleshooting.

### Fixed
- **Remote invite links.** `sassytalk://` deep links, https invite URL import,
  and Enter Code tab paste fallback; screenshots allowed on auth/QR screens.
- **Relay join stability.** Disabled auto PQC on relay peers; zombie socket and
  transport-steal fixes; same-name self-echo no longer drops peer audio.

### Changed
- Fresh native rebuild (`libsassytalkie.so`) from `main` merge.

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
