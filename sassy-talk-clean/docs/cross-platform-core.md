# sassytalkie-core — cross-platform core crate

Created in the v2.7.x cycle to end the Android-↔-Windows feature drift. Lives at `sassy-talk-clean/core/` and is depended on by both `android-native/` and `tauri-desktop/src-tauri/` via local `path = "../core"` references.

## What's in core

| Module               | Lines | Status   | Consumers                  |
|----------------------|-------|----------|----------------------------|
| `protocol`           | ~130  | new      | android, tauri (transport) |
| `audio_cache`        | 1497  | migrated | android                    |
| `cohort_history`     | 354   | migrated | android                    |
| `crypto`             | 212   | migrated | android                    |

Total shared logic: **~2200 lines** of pure-Rust, platform-agnostic code (no JNI, no AudioRecord/AudioTrack, no cpal, no ContentProvider). Builds standalone on Windows host; `cargo test --lib` in `core/` produces 32 passing tests (the 3 failures are the same pre-existing Live-jitter ones in `audio_cache` we've tracked all session — not caused by extraction).

## How consumers pull it in

### android-native/Cargo.toml
```toml
sassytalkie-core = { path = "../core" }
```
`src/lib.rs` keeps `pub mod crypto; pub mod audio_cache; pub mod cohort_history;` — these now resolve to thin re-exports inside the existing files (or were deleted and re-pointed). Every existing `use crate::crypto::SessionKey` in `jni_bridge.rs`, `session.rs`, `cellular_transport.rs`, etc., still works without source changes.

### tauri-desktop/src-tauri/Cargo.toml
```toml
sassytalkie-core = { path = "../../core" }
```
`src/transport/control.rs` now imports protocol opcodes:
```rust
pub use sassytalkie_core::protocol::{
    OP_PTT_START, OP_PTT_STOP,
    OP_HEARTBEAT, OP_PARTNER_OFFLINE,
    OP_PTT_START_V2, OP_PTT_STOP_V2,
    OP_WAKE, OP_REPLAY_FRAME,
};
```
Already-existing desktop-only opcodes (`OP_READY_ACK`, `OP_PING`, etc.) stay in `control.rs` until they're needed on Android too.

## What's NOT in core (and why)

| Module / file              | Why platform-specific                                  |
|----------------------------|--------------------------------------------------------|
| `audio.rs` (android-native) | AudioRecord / AudioTrack via JNI                       |
| `audio.rs` (tauri)          | cpal — host-OS audio                                   |
| `jni_bridge.rs`             | JVM `JNIEnv`, `JString`, `JObject` types               |
| `audio_effects.rs`          | Android AcousticEchoCanceler / NoiseSuppressor         |
| `audio_routing.rs`          | Android AudioManager                                   |
| `device_quirks.rs`          | Android `Build.MANUFACTURER` quirks                    |
| `wifi_*.rs`                 | Android WifiManager / multicast lock                   |
| `tauri-desktop/security/*`  | OS-keychain-backed identity (not yet on Android)       |

## Progress

| Item | Status | Notes |
|---|---|---|
| `protocol` shared module | ✓ v2.7.4 | Both crates pull opcodes from core |
| `audio_cache` shared module | ✓ v2.7.4 | Android in v2.7.4, tauri in v2.8 |
| `cohort_history` shared module | ✓ v2.7.4 | Android only so far; tauri pending UI |
| `crypto` shared module | ✓ v2.7.4 | Android using core; tauri keeps its own (API mismatch) |
| `session` shared module | ✓ v2.8 | Pure-logic SessionManager, both crates can consume |
| Tauri audio_cache consumer | ✓ v2.8 | Wired in `lib.rs` RX loop, single-peer for now |
| Tauri cohort_history consumer | ☐ | Needs desktop UI for rejoin list |
| Tauri crypto API unification | ☐ | Tauri's `encrypt(pt, nonce) → (ct, tag)` ≠ Android's `encrypt(pt) → nonce‖ct‖tag` — mechanical refactor |
| Tauri session consumer | ✓ v2.8 | `join_cellular_session` via QR import |
| **Tauri cellular relay transport** | **✓ v2.8** | `transport/cellular.rs` — WebSocket relay client with core `CryptoSession`. Basic internet PTT works; control-plane parity (PTT v2, liveness, catch-up) still pending — see `docs/audit-2026-07-03.md` P1 items. |
| Codec unification | ☐ | Android: libopus via cc. Tauri: audiopus via audiopus_sys. Either works. |
| iOS-native | ☐ | Out of scope until iOS revives |

## Windows-Android interop status (updated 2026-07-03)

Cellular relay transport shipped in v2.8 (`transport/cellular.rs`). Desktop can join Android sessions over the internet via QR/share-link import and encrypted WebSocket relay.

**Remaining gaps** (see `docs/audit-2026-07-03.md` §7 P1):
- Desktop skips control frames over cellular (no liveness tracker, no OP_PTT_START_V2 emit)
- OP_REPLAY catch-up broken on all clients (non-TLV framing mismatch)
- Android `/presence` and `/share` missing auth tokens
- UDP desktop path still uses divergent local crypto (LAN-only; not Android-interoperable)

## Workflow going forward

- Wire-protocol changes (new opcodes, payload format) — touch ONLY `core/src/protocol.rs`. Both clients pick up the new constant automatically.
- Audio cache logic changes (queue ordering, mix tuning) — touch ONLY `core/src/audio_cache.rs`. Both clients inherit the change.
- Platform-specific changes (AudioTrack tuning, cpal device picking, JNI bridge) — touch only the consumer crate. Core stays untouched.
- A change that spans both — split into a core PR + a consumer PR; CI on the consumer side picks up the new core revision via `path = "../core"`.

## Build verification (this session)

```
$ cd core && cargo build           # OK
$ cd core && cargo test --lib      # 32 pass / 3 pre-existing fail
$ cd android-native && cargo ndk -t aarch64-linux-android -t x86_64-linux-android \
    -o ./jniLibs build --release   # OK — release profile clean
$ cd tauri-desktop/src-tauri && cargo check  # OK — 54s compile, 1 unused-import warning unrelated
```
