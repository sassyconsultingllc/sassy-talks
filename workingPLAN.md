# SassyTalky — Working Plan

Living task checklist, updated as work progresses. Shuffled for logical dependency order.

**Legend:** ✅ done+verified · 🟡 done/unverified (needs device build) · 🔵 in progress · ⬜ not started

---

## 1. Audio Reliability — "drunk/mosquito" garbling (cell/WiFi) — ✅ DONE (Rust-verified)

Root causes found & fixed in `android-native/src/audio_pipeline.rs`:
- **TX timestamp jitter**: wall-clock `now_ms()` + `rotate_left(17)` gave non-monotonic epochs → cross-transport dedup dropped real frames. Replaced with per-PTT-press stable `tx_epoch` + monotonic `tx_frame_index`.
- **TX partial-frame drops**: read loop capped at 25 attempts/50ms → dropped partial frames → gaps → speed/pitch wobble. Raised to 80 attempts with exponential backoff (2→32ms), ~200ms budget.
- **PlayoutPacer premature snap-forward**: 200ms threshold was too aggressive on lossy paths → sudden speed changes. Widened to 500ms.
- **Opus FEC**: confirmed in-band FEC + `decode_fec_from_next` is the correct repair path; kept.
- **New diagnostics**: `TX_PARTIAL_FRAME_DROPS` + `RX_DECODE_FAILURES` in `diag::snapshot_json()` — the "real-time compression check" is now visible on the panel.

**Tests:** 189 core + 57 native pass. ✅

## 2. Encryption Verifier — ✅ DONE (Rust-verified)

- Added `test_tampered_ciphertext_is_rejected` in `core/src/crypto.rs`: flips every AEAD byte, asserts GCM rejects each, verifies no replay-window poisoning.
- AEAD + replay protection (nonce window) already existed in `CryptoSession`; now proven.

## 3. Cohort Ping/Pong Protocol — 🟡 DONE (Rust-verified, Kotlin unverified)

New authenticated short-message protocol:
- New opcodes `OP_COHORT_PING` (0x21) / `OP_COHORT_PONG` (0x22).
- Rust codecs in `core/src/ptt_frames.rs` + round-trip/truncation tests. ✅
- Kotlin: `ControlFrame` codecs, `PttCoordinator` dispatch (AEAD-sealed via `AuthenticatedControlPlane`), periodic ping every 5th heartbeat, relay routing window widened `0x10..0x22`. 🟡 unverified (no SDK).

## 4. Bluetooth (BLE) Transport — 🔵 IN PROGRESS

**Decision:** Peer-to-peer GATT (one peripheral / one central, shared audio+control characteristic), no-internet walkie-talkie.

- **Android** (`BleSignalingService.kt`): already has GATT server+client, PTT char (WRITE+NOTIFY), MTU 517, AEAD sealing. Needs: write pacing (20ms/frame), reconnect handling, and to be promoted from "signaling" to full audio bearer.
- **iOS** (`BluetoothManager.swift`): currently discovery-only. Needs full GATT client+server carrying the same TLV frames — reuse the Rust core codec.

## 5. Deep Linking — 🟡 DIAGNOSED + partial fix, needs deploy/verification

**Root cause (verification gate):** The share sends the **https** link
`https://relay.sassyconsultingllc.com/v/<id>#<key>`. For a tap to open the app
*directly*, Android App Links verification must pass, which requires the
`ANDROID_CERT_SHA256` in `cloudflare-worker/wrangler.toml` to exactly match the
**signing cert of the installed build**. If you sideload a **debug** build (or
a build signed with a different keystore than the Play re-signing cert listed),
`android:autoVerify` fails and the link always bounces to the browser landing
page (`viewer.js`) — i.e. "does nothing, have to paste into the app."

**What's already in place:**
- Manifest: `android:autoVerify="true"` on https `relay.sassyconsultingllc.com/v/` (✅)
- Worker serves `/.well-known/assetlinks.json` with `ANDROID_CERT_SHA256` (✅)
- `MainActivity.captureShareIntent` handles cold-start + `onNewIntent` warm re-entry (✅)
- Custom schemes `sassy-talks://` + legacy `sassytalk://` registered as fallback (✅)

**Fixes applied in this pass:**
- Added `WalkieService.notifyPeerJoined(...)` — a system "Joined radio room"
  notification on successful invite import (was toast-only, invisible if backgrounded).
- Wired it into the `AppNavigation` re-success path.

**Still needed (cannot verify here):**
- Ensure `ANDROID_CERT_SHA256` matches the actual deployed signing cert (Play
  App Signing for release; the debug keystore for dev builds). Sideloading a
  debug build against the release fingerprint = verification will fail.
- Confirm the worker `[vars]` are deployed (X-Wellknown-Configured header).
  `curl -sI https://relay.sassyconsultingllc.com/.well-known/assetlinks.json`

**User decision (2026-10-20):** iOS will rely on the `sassy-talks://`
custom scheme (not https Universal Links) for device testing — so the AASA
/ Apple app-ID config (**task B previously listed**) is **dropped**. Android
side still uses the assetlinks fingerprints (task A). The connected iPhone
(Sean Mattews, `A3DE57C5…`) + valid Apple Development identities are present,
so the path to a real device build is open.

## 6. "Phone thinks it's in a call" + battery — 🟡 DIAGNOSED, needs device logs

The code already scopes the phone-call behaviors to RX bursts:
- Kotlin: `requestRadioAudioFocus` uses `USAGE_VOICE_COMMUNICATION` (O+) /
  `STREAM_VOICE_CALL` (<O), engaged only on inbound frames, **abandoned after
  5s** of silence (`noteInboundRx` → `rxWakeHoldJob`).
- Native: `audio_routing::engage_comm_mode` sets `MODE_IN_COMMUNICATION`, and
  `audio_routing::release()` restores it in `AudioEngine::stop_playing`
  (called by the RX thread after 5s idle).
- Wake lock: 3-min per inbound frame, released on silence.

**Suspects for the persistent symptom (need device confirmation):**
- If audio/heartbeats keep flowing, the stays-hot window re-arms forever and
  the phone never leaves "call" mode. Check `diag` counters for continuous RX.
- Volume-rocker snapping to `STREAM_VOICE_CALL` is inherent to comm-mode; if the
  user wants the rocker to control media during idle, comm-mode must be released
  even more aggressively when not actively in a burst.
- Battery: confirm the multicast lock is released when the radio is idle (it is
  held for service lifetime by design; verify it's not being renewed on a tight
  loop via `lastWakeRenewMs`).
- **Requires `adb shell dumpsys audio` / battery stats on the device** to see
  which hold actually lingers — I can't run that here.

## 7. Themed UI/UX (ux-persuasion-engineer) + per-cohort color — 🔵 IN PROGRESS

- Split "themed UI/UX" into: (a) a persuasion-first pass on the main screens
  (onboarding/permission prompts, PTT affordance, invite flow, empty/peer
  states), and (b) a per-cohort background color token stored per room ID.
- The `.agent/skills/ux-persuasion-engineer/SKILL.md` skill exists — I'll apply
  it to the flow copy/prompts. Per-cohort color is a real feature; I'll add a
  `cohortColor` token in `ManagedConfig` keyed by session ID and wire it into
  the theme surfaces.
- **Code achievable here; rendering/screenshots requires Android Studio/Xcode.**

## 8. Screenshots (iOS + Android) — 🟡 PARTIAL: iOS simulator done, device/Android pending

**Progress:** Built, installed, and launched the iOS app on an iPhone 17 Pro simulator
(iOS 26.5) and captured `docs/screenshots/ios-simulator-home.png`.
**Blocked for real-device iOS:** Apple ID login for the free Personal Team
(`No Account for Team "49WZYYVNFS"`) — you must add your free Apple ID in
Xcode → Settings → Accounts and approve it.
**Blocked for Android:** still no Android SDK on this machine.

---

## Option B setup (free Apple ID device testing) — 🟡 DONE in code + simulator-verified, device blocked on Apple ID

Changes made in `ios-native/`:
- Added `SassyTalkie-Debug.entitlements` — reduced copy that omits the paid
  capabilities (`multicast`, `associated-domains`).
- `project.pbxproj`: Debug config now uses the reduced entitlements + bundle ID
  `com.sassyconsulting.sassytalkie.dev`; Release unchanged.
- Fixed `INFOPLIST_FILE` in both target configs (was pointing at a stale
  `build/...xcarchive/.../Info.plist`) → now the real source `Info.plist`.
- Removed stale, gitignored `ios-native/build/` archive.

Verified by a full simulator build (`BUILD SUCCEEDED`), install, launch, and
screenshot. Device install is blocked only on the Apple ID login (human step).

---

## Suggested improvement queue (optional, flag if unwanted)
- [ ] BLE: write pacing + reconnect + promote to real audio path
- [ ] Deep link: host `assetlinks.json` on worker
- [ ] Add a "user joined" notification + roster toast on invite import
- [ ] Per-cohort color: store color token per room ID in `ManagedConfig`, apply to background/surface
- [ ] iOS: GATT client+server (needs Xcode)
