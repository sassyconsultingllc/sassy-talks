# Sassy Talk(s)

Short overview

This repository contains multiple app artifacts and platform-specific builds for the Sassy Talk(s) project. Key folders:

- `sassy-talk-clean/` — main app sources (web/tauri/desktop, Android, iOS, native components).
- `v1.1.0-lobby/` — standalone lobby GUI (web/React/TypeScript) kept as a separate top-level snapshot.

Status (2026-01-18):

- `v1.1.0-lobby` is not merged into the main codebase; it contains a ready-to-review GUI implementation.
- A full workspace inventory was saved to `v:\Projects\sassytalkie\file-inventory.txt`.
- Next steps: create branch `merge/lobby-into-main`, generate diffs for GUI files, apply selected changes, run builds/tests, and open a PR.

Quick pointers

- GUI sources to inspect: `v1.1.0-lobby/src` (components, hooks, services, styles, assets).
- Primary desktop/web entrypoints: `sassy-talk-clean/tauri-desktop/src` and `sassy-talk-clean/tauri-desktop/src-tauri`.
- Android/iOS native code is in `sassy-talk-clean/android-app` and `sassy-talk-clean/ios-native`.

If you want, I can (a) create the `merge/lobby-into-main` branch, (b) generate diffs limited to GUI files, or (c) apply selected files into a branch for testing.
