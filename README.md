<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-R3JUDXVQZIFD
-->
# Sassy Talk(s)

One source tree. Edit files here — there is no nested `sassy-talk-clean/` copy and no parallel stub apps at the repo root.

## Layout

| Path | What it is |
|------|------------|
| `android-app/` | Production Android app (Compose). Build + ship from here. |
| `android-native/` | Rust JNI / native audio+transport for Android |
| `ios-native/` | iOS native + Rust |
| `tauri-desktop/` | Desktop (Tauri) |
| `core/` | Shared Rust core (`sassytalkie-core`) |
| `cloudflare-worker/` | PTT relay worker |
| `scripts/` | Commit helpers + `ship.sh` release pipeline |
| `docs/` | Design / handoff / legal |

## Android build

```powershell
cd android-app
.\gradlew.bat assembleDirectRelease bundlePlayRelease
```

- **Direct APK** (website / license key): `android-app/app/build/outputs/apk/direct/release/app-direct-release.apk`
- **Play AAB** (Billing): `android-app/app/build/outputs/bundle/playRelease/app-play-release.aab`

Ship with `scripts/ship.sh <version>` after both artifacts exist.

## Do not resurrect dual trees

A previous layout kept incomplete `android-app/` / `tauri-desktop/` / `cloudflare-worker/` stubs beside a nested `sassy-talk-clean/` tree. Agents patched the stubs; the real app diverged; regressions followed. **Never reintroduce a second copy of these projects.**
