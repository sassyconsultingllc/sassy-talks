# SassyTalkie Android Native Build Results

**Date:** 2026-02-15
**Rust Toolchain:** 1.93.1 (stable)
**Android NDK:** r27c
**Targets:** aarch64-linux-android (arm64-v8a), x86_64-linux-android

## Built Artifacts

| File | Arch | Size |
|------|------|------|
| `jniLibs/arm64-v8a/libsassytalkie.so` | ARM64 | 3.0 MB |
| `jniLibs/x86_64/libsassytalkie.so` | x86_64 | 3.3 MB |

## JNI Exports (20 functions)

All exported via `com.sassyconsulting.sassytalkie.RustBridge`:

- `initialize()` → `jboolean`
- `getState()` → `jstring`
- `getPairedDevices()` → `jstring`
- `connectDevice(address)` → `jboolean`
- `startListening()` → `jboolean`
- `disconnect()` → `jboolean`
- `pttPress()` → `jboolean`
- `pttRelease()` → `jboolean`
- `setChannel(ch)` → void
- `getChannel()` → `jint`
- `generateSessionQr(hours)` → `jstring`
- `importSession(qrData)` → `jboolean`
- `isAuthenticated()` → `jboolean`
- `getSessionStatus()` → `jstring`
- `clearSession()` → void
- `getCacheStatus()` → `jstring`
- `getUsersJson()` → `jstring`
- `setUserMuted(id, muted)` → void
- `setUserFavorite(id, fav)` → void
- `getVersion()` → `jstring`

Plus `android_main` entry point for NativeActivity/GameActivity.

## Source Modules (13 files)

| Module | Size | Purpose |
|--------|------|---------|
| `lib.rs` | 27.5 KB | App entry, egui UI (3 screens) |
| `jni_bridge.rs` | 25.2 KB | JNI wrappers + 20 exports |
| `state.rs` | 10.6 KB | StateMachine, TX/RX threads |
| `audio_cache.rs` | 7.7 KB | Multi-speaker queue/replay |
| `bluetooth.rs` | 6.6 KB | RFCOMM connection management |
| `wifi_transport.rs` | 5.8 KB | UDP multicast discovery + audio |
| `transport.rs` | 5.1 KB | Unified BT/WiFi with encryption |
| `permissions.rs` | 5.3 KB | Android runtime permissions |
| `audio.rs` | 5.1 KB | AudioRecord/AudioTrack JNI |
| `session.rs` | 4.5 KB | QR-based session key exchange |
| `codec.rs` | 3.9 KB | IMA ADPCM voice compression |
| `crypto.rs` | 3.1 KB | AES-256-GCM + X25519 ECDH |
| `users.rs` | 1.9 KB | User registry (mute/favorite) |

## Integration

Copy `jniLibs/` into Android app's `app/src/main/` directory:
```
app/src/main/jniLibs/
├── arm64-v8a/
│   └── libsassytalkie.so
└── x86_64/
    └── libsassytalkie.so
```

## Build Warnings

50 warnings (0 errors). Mostly unused variables and static_mut_refs deprecation notices.
