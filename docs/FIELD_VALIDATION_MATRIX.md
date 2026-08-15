<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-->
# Field validation matrix

Coverage mapping for SassyTalkie field claims. **Physical-only rows are not
green unless a human fills Pass/Fail on a real device.** Do not treat this
table as a certification.

How to run (host, no device):

```
cargo test -p sassytalkie-core
cargo test -p sassy-talk --lib
cd android-app && ./gradlew :app:testDirectDebugUnitTest
```

iOS `cargo test -p sassytalkie-ios` needs an Apple/host toolchain (fails on Windows).

Emulator (DEBUG APK only — **do not assembleRelease**): AVD `SassyTalkie_34` (`emulator-5554`).
Evidence this pass: `docs/evidence/emulator-2026-08-14/`.

Host suites this pass (2026-08-14): core **167/167**; desktop `--lib` **60 passed, 1 failed** (`test_random_port` bind 10013, pre-existing host restriction), **1 ignored** (live relay); android-native **57 passed, 0 failed, 1 ignored**; Android JVM `:app:testDirectDebugUnitTest` **145 tests, 0 failures**. P1–P4 **blocked** (no physical device; only `emulator-5554`). Evidence: `docs/evidence/emulator-2026-08-14/p-rows-blocked-emulator.png`.

| ID | Claim | Kind | Automated coverage | Executed this pass | Pass/Fail |
|----|--------|------|--------------------|--------------------|-----------|
| U1 | Authenticated control 0x18 round-trip, replay, fail-closed raw 0x10..=0x20 | Unit | `core/src/control_auth.rs`; Android `AuthenticatedControlPlaneTest`; desktop `control_plane.rs`; iOS `control.rs` | Yes — core 163; Android JVM 136; desktop 58 | Pass |
| U2 | Hybrid 4-way: INIT/RESP/CONFIRM 0x1F / CONFIRM_ACK 0x20; lost CONFIRM or lost ACK cannot split TX keys; auto-PQC off | Unit | `core/src/hybrid_rekey.rs` (lost confirm, lost ACK, late ACK, forged ACK, timeout rollback, no key split); `HybridRekeyPolicyTest`; desktop `lost_ack_policy_does_not_split` | Yes — this pass | Pass |
| U3 | Room ID is not authorization; enrollment token reject | Unit | `core/src/enrollment.rs`; `EnrollmentProofTest` | Yes | Pass |
| U4 | SOS / emergency opcodes unique vs hybrid | Unit | `EmergencyOpcodeTest`; core `protocol` / `emergency` | Yes | Pass |
| U5 | Live-bearer gating: BLE-only cannot TX; IP or RFCOMM required | Unit | `BtAudioPathTest` | Yes | Pass |
| U6 | Lock-screen PTT actions require notifications + pref | Unit | `ManagedConfigKeysTest` | Yes | Pass |
| U7 | HUD off unless radio UI + (debug or diag); MDM can force off; unmanaged default allows debug HUD | Unit | `DiagnosticsPrefsTest`; `ManagedConfig.DEFAULT_DIAGNOSTICS_ALLOWED` | Yes | Pass |
| U8 | Translation error 12 / missing pack wins over in-flight model download | Unit | `LiveTranslationLifecycleTest` (`needsOfflineSpeechPack`, `radio overlay prefers speech pack`) | Yes | Pass |
| U9 | Audio route idle: loudspeaker default, no flap when already on speaker | Unit | `RxOutputPolicyTest` | Yes | Pass |
| U10 | TX max duration / transport-loss stop | Unit | `TxSafetyPolicyTest` | Yes | Pass |
| U11 | Technical audit redact + disclaimer | Unit | `EmergencyAuditStoreTest`; `core/src/audit.rs` | Yes | Pass |
| U12 | FIPS restriction fail-closed when no provider (**not a FIPS/CJIS certification**) | Unit | `FipsProviderTest` (About text is not a Certified badge) | Yes | Pass |
| U13 | Desktop OS vault (Windows Credential Manager, macOS Keychain, Linux libsecret) with AES file fallback | Unit | `os_vault.rs`; `secret_store.rs`; `MemoryVault` trait | Yes | Pass |
| U14 | Relay-only: require_relay forces relay, disables Wi-Fi/BT | Unit | `ManagedConfig` keys + AutoConnect uses `wifiEnabled`/`relayEnabled` | Yes (key presence); full AutoConnect needs device | Pass (keys) |
| U15 | Radio chrome vs overlay: socket-up never shown as waiting-for-relay | Unit | `RelayConnectionState.radioStatusLine` | Yes | Pass |
| U16 | TLS intermediate pin-set (backups); mismatch fail-closed; production default on | Unit | `core/src/tls_pins.rs`; `RelayTlsPinsTest`; desktop `tls_pinning.rs` | Yes | Pass |
| U17 | FCM cold wake: visible high-priority notification, no mic FGS on API 34+, no crash policy | Unit | `FcmWakePolicyTest`; `OemBatteryGuidance`; `docs/FCM_OEM_LIMITS.md` | Yes | Pass |
| E1 | HUD off on Authenticate/Profile; HUD only on radio screens in debug | Emulator | `DiagnosticsPrefs.shouldShowOverlay` + visual | DirectDebug APK on `SassyTalkie_34`. Auth: `e1-auth-hud-off.png`. Profile: `e1-profile-hud-off.png`. Radio Channel 1 + People: `e1-e2-radio-hud.png`, `e2-hud-expanded-ws.png`, `e1-users-hud.png` (`TX 0/s RX 0/s`). | **Pass** |
| E2 | Overlay SESSION `ws` matches live Cloudflare relay (no `ws=down` while relay connected) | Emulator | `RelayConnectionState.isLive` / `overlayWsLabel` | Expanded HUD: `NET path=Cloudflare ws=connected`; `SESSION cell=connected ws=up ch=1` (`e2-hud-expanded-ws.png`, `uidump-users.xml`). Native `SassyDiag` `relay.state=connected`. | **Pass** |
| E3 | Notification has no PTT action unless `lock_screen_ptt` is on (default off); private/redacted | Emulator | `WalkieService` `showPttAction`; dumpsys | Shade expanded: `e3-notification-shade.png` — "Radio active — Cloudflare Relay", **no PTT actions**. dumpsys: `vis=PRIVATE`, publicVersion text "Radio active", `numWithActions=0`. | **Pass** |
| E4 | Translation error 12 / missing speech pack → UNAVAILABLE + install pack, not infinite download | Emulator | `LiveTranslationText.radioOverlayPrimary`; 45s model-download timeout | After DirectDebug reinstall: red banner **Need offline speech pack — open Settings** (`e1-e2-radio-hud.png`). Pre-fix APK showed infinite "Downloading English → Spanish model". | **Pass** |
| E-idle | Idle not stuck in `MODE_IN_COMMUNICATION`; loudspeaker default | Emulator | `dumpsys audio` | Requested/Actual `MODE_NORMAL`; communication device `speaker`. Brief `MODE_IN_COMMUNICATION` at session start then released. | **Pass** |
| E-ptt | PTT live-bearer (emulator mic silence OK) | Emulator | `SassyDiag` capture/codec | Hold produced `capture.frames=82`, `frames_encoded=82`, `tx_live=true` (`ptt-hold.png` after release). | **Pass** |
| E-sos | SOS opens and can cancel | Emulator | Main overflow → confirm dialog | `sos-confirm-dialog.png` Broadcast emergency? Cancel / BROADCAST SOS; `sos-cancelled.png` after Cancel. | **Pass** |
| I1 | Instrumented SOS encode/beacon | Instrumented | `EmergencyInstrumentedTest` | Not run this pass (JVM + visual prioritized) | |
| P1 | Bluetooth accessory hearing (headset/SCO) | **Physical only** | Checklist below | **Blocked** 2026-08-14: adb shows only `emulator-5554`. Samsung `R5CX820XFQN` not attached. No BT headset on emulator. | Blocked |
| P2 | Two-device mixed protocol hearing (new 0x18 vs old peer) | **Physical only** | Checklist below | **Blocked**: one emulator only; mixed-protocol needs two devices (or phone+emulator). | Blocked |
| P3 | Lock-screen PTT on a real phone | **Physical only** | Checklist below | **Blocked**: no physical phone. Emulator lock-screen PTT is not a P3 pass. | Blocked |
| P4 | SOS over radio (RF / BT mesh, not emulator) | **Physical only** | Checklist below | **Blocked**: no physical radio/BT mesh. Emulator SOS cancel path remains E-sos (do not dispatch a live emergency). | Blocked |

## Physical-only checklist (print / fill)

Device A: _none (emulator-5554 only)_  Device B: _none_  Build: debug (not release)  Date: 2026-08-14

| Row | Procedure | Pass | Fail | Notes |
|-----|-----------|------|------|-------|
| P1 | Pair a BT headset/SCO accessory; TX and RX heard on accessory; no earpiece-only surprise | ☐ | ☐ | Blocked — no physical device / no SCO accessory |
| P2 | One current-build peer + one older build without 0x18: audio still heard; privileged controls degrade (no crash, no unauthenticated emergency/rekey) | ☐ | ☐ | Blocked — second peer not present |
| P3 | Enable lock-screen PTT; lock the phone; start/stop TX from notification; MDM `enable_notifications=false` hides extra action | ☐ | ☐ | Blocked — emulator is not a real phone lock screen |
| P4 | SOS/man-down over radio path (not Wi-Fi-only emulator); peer receives authenticated emergency; local audit records `self_raised` | ☐ | ☐ | Blocked — no RF/BT mesh. Do not page production. Emulator confirm+cancel is E-sos only. |

Rows P1–P4 stay blank until a human marks them. Do not mark them passed from unit tests.
