<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-EFQTS2UQEPZ5
-->
# SassyTalk — Developer Manifesto

> Last updated: 2026-03-29. This document is the ground truth for onboarding any developer or AI assistant.
> **MANIFESTO.md lives in `sassy-talk-clean/`. The production deploy repo is `sassyconsultingllc-cloudflare/`.**

---

## What It Is

Encrypted push-to-talk walkie-talkie. AES-256-GCM end-to-end encryption, per-channel keys (8 channels, 3 subchannels each). Works over WiFi multicast (same network, zero infrastructure) AND Cloudflare Relay (across networks, cellular). On-device Whisper transcription. No server ever sees plaintext audio.

---

## Two-Worker Architecture

**This is critical. Two separate Cloudflare workers serve two separate domains.**

```
sassyconsultingllc.com         — main website (worker: sassyconsultingllc-cloudflare/)
relay.sassy-consults.com       — PTT relay only (worker: sassy-talk-clean/cloudflare-worker/)
```

- `sassyconsultingllc-cloudflare/src/worker.js` — handles website, R2 downloads, contact form,
  checkout. PTT routes are 301 → relay.sassy-consults.com. NEVER add relay logic here.
- `sassy-talk-clean/cloudflare-worker/` — ONLY the PTT relay. No website, no assets, no APIs.
  Deployed as `sassytalk-relay` worker. **This is the one the app connects to.**

**Deploy relay:** `cd sassy-talk-clean/cloudflare-worker && npx wrangler deploy`
**Deploy website:** `cd sassyconsultingllc-cloudflare && npx wrangler deploy`

**The old `sassy-talk-clean/cloudflare-worker/src/worker.js` is a stale stub — ignore it. The real
relay entry point is `ptt-relay-worker.js`.**

---

## System Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         Android App (Kotlin/Compose)                 │
│                                                                      │
│   MainActivity → WalkieService (foreground) → SassyTalkNative (JNI)  │
│                                                                      │
│  UI Screens:                                                         │
│    AppNavigation   — routing + singleton lifecycle (AutoConnectMgr)  │
│    MainScreen      — PTT button, channel/subchannel, QR dialog       │
│    QRAuthScreen    — generate/scan/paste QR, channel picker          │
│    UsersScreen     — peer list, mute/favorite, auto-refresh          │
│    TranscriptionFeedScreen — Whisper log, placeholder → real text    │
│    ProfileScreen   — name + emoji + color avatar                     │
│                                                                      │
│  Services/Bridges:                                                   │
│    AutoConnectManager  — dual WiFi+relay connect, failover           │
│    CellularWebSocketClient — OkHttp WS to relay.sassy-consults.com   │
│    TranscriptionBridge — VAD + PCM buffer + whisper JNI              │
│    WhisperModelManager — downloads ggml-tiny.en.bin from HuggingFace │
│    BleSignalingService — BLE GATT server, peer discovery             │
│    BluetoothTransport  — RFCOMM audio channel                        │
│    PttCoordinator      — BLE PTT signaling                           │
└─────────────────────────┬────────────────────────────────────────────┘
                           │ JNI (libsassytalkie.so)
┌─────────────────────────▼────────────────────────────────────────────┐
│                     Rust Native (android-native/)                    │
│                                                                      │
│  jni_bridge.rs     — ALL JNI exports (~2100 lines), one file         │
│  state.rs          — StateMachine, JniAppState, global state         │
│  audio_pipeline.rs — TX/RX threads, wire frame pack/unpack           │
│  transport.rs      — TransportManager: WiFi + Cellular + BT routing  │
│  crypto.rs         — AES-256-GCM encrypt/decrypt, X25519 ECDH        │
│  session.rs        — SessionRegistry (8 channel slots + group names) │
│  codec.rs          — Opus encoder/decoder (48kHz, 20ms frames)       │
│  audio.rs          — AudioRecord/AudioTrack via JNI (not AAudio)     │
│  audio_cache.rs    — Multi-speaker queue, replay system              │
│  users.rs          — UserRegistry (peer tracking, mute/favorite)     │
│  wifi_transport.rs — UDP multicast socket (239.255.42.42:5555)       │
│  cellular_transport.rs — WebSocket relay queue (Kotlin polls/JNI)    │
│  transcription.rs  — WhisperEngine, 48k→16k downsample               │
│  whisper_ffi.rs    — FFI to whisper_wrapper.c                        │
│  opus_ffi.rs       — FFI to libopus                                  │
│  build.rs          — Compiles libopus, links whisper static libs     │
└──────────┬──────────────────────────────┬────────────────────────────┘
           │ UDP multicast                │ WebSocket (via Kotlin)
           ▼                              ▼
   239.255.42.42:5555           relay.sassy-consults.com/ws
   (same-network peers)         (cross-network, Cloudflare DO)
                                          │
                              ┌───────────▼──────────────┐
                              │  PttRoom Durable Object   │
                              │  (ptt-relay.js)           │
                              │  16 peers/room, blind fwd │
                              │  Hibernation-aware        │
                              └──────────────────────────┘
```

---

## Transport Logic

Both transports run simultaneously when both are available. This is intentional.

```
WiFi available + relay connected → send on BOTH
WiFi drops → relay continues, no interruption
Relay only (cellular) → WiFi path skipped
BT only → Bluetooth RFCOMM path
```

**Room ID**: derived from session QR `session_id`. All channels for a given session share one relay
room. Channel isolation is enforced by encryption — packets for channel 2 are encrypted with the
channel-2 key. If a device doesn't have that key, decryption fails, packet silently dropped.

**The relay never decrypts anything.** It is a blind binary forwarder.

---

## Wire Frame Format

```
[channel:1][subchannel:1][sender_id_len:1][sender_id:N][name_len:1][name:M][timestamp:8][encrypted_payload]
```

- `channel` byte is **OUTSIDE** the AES envelope — receiver uses it to look up the right key
- `subchannel` (0=Main, 1=A, 2=B) also outside envelope — for same-key sub-group filtering
- Everything from `encrypted_payload` onward is AES-256-GCM ciphertext + tag
- Subchannel mismatch: device still registers the sender in UserRegistry, but does not play audio

---

## Per-Channel Encryption

8 channel slots, each independent. `SessionRegistry` in `session.rs`:

```rust
pub struct SessionRegistry {
    channels: [Option<ChannelSession>; 8],  // index 0 = channel 1
    device_name: String,
}
pub struct ChannelSession {
    pub key: SessionKey,
    pub group_name: String,
    pub session_id: String,
    pub expires_at: u64,
}
```

`TransportManager` in `transport.rs` mirrors this:

```rust
channel_crypto: [Option<CryptoSession>; 8],
```

**Legacy crypto field**: `transport.rs` also has a top-level `crypto: Option<CryptoSession>` for
backward compat with send/receive paths that haven't been fully migrated. When importing a QR,
**BOTH** `channel_crypto[ch]` AND `crypto` must be set or the session appears authenticated but
audio is unencrypted. See `nativeImportSessionFromQR` in `jni_bridge.rs`.

---

## Critical Code Paths

### QR → Session → Transport → Audio TX

```
1. QRAuthScreen: user picks channel (1-8), optional group name, taps "Generate"
2. → SassyTalkNative.generateChannelQR(channel, durationHours, groupName)
3. → jni_bridge.rs:nativeGenerateChannelQR
4. → session.rs:generate_session_qr(channel, duration, group_name)
5. → random 32-byte AES key → JSON:
   { "session_id": "...", "channel": 2, "group_name": "Alpha Team",
     "key": "<hex>", "expires_at": <unix_ts>, "version": 2 }
6. → QR displayed or copied to clipboard / shared as text
7. Other device scans/pastes → nativeImportSessionFromQR
8. → session.rs:import_session(json) → stores ChannelSession in channels[ch-1]
9. → transport.rs:set_channel_crypto(ch, crypto_session) — sets BOTH slots
10. → SessionRegistry key persisted to SharedPreferences "session_ch_N"
11. Audio TX: PTT press → audio_pipeline.rs TX thread → mic → Opus encode
12. → pack_wire_frame(channel, subchannel, sender_id, name, timestamp, encoded)
13. → transport.rs:send(frame) → encrypt_for_channel(ch, frame)
14. → WiFi multicast send (if connected) + cellular outbound queue (if connected)
```

### App Start → Session Restore

```
1. AppNavigation.kt: nativeInit() called once
2. → jni_bridge.rs:nativeInit → init_transcription_bridge_class() [CRITICAL]
3. AutoConnectManager.autoConnect() → WiFi multicast + relay connect (both simultaneously)
4. On connect callback → restoreSession() called AGAIN after transport init
5. restoreSession() reads SharedPreferences "session_ch_1" through "session_ch_8"
6. Each found JSON → nativeImportSessionFromQR → re-applies crypto
   (MUST happen AFTER transport init or crypto is lost when transport initializes)
```

### RX → Audio → Transcription

```
1. WiFi or Kotlin WebSocket → bytes arrive
2. jni_bridge.rs: nativePushCellularPacket (cellular) or internal WiFi thread
3. → audio_pipeline.rs RX thread → unpack_wire_frame
4. → extract channel byte (unencrypted) → look up channel_crypto[ch]
5. → AES-256-GCM decrypt → Opus decode → PCM
6. → AudioCache.push(sender_id, pcm_frame) → dequeue → AudioTrack play
7. → call_transcription_bridge(sender_id, name, pcm, is_favorite, is_muted)
8. → JNI call into Kotlin TranscriptionBridge.onAudioReceived()
9. → VAD: RMS energy check, accumulate PCM while speaking
10. → 400ms silence → finalizeSpeechSegment()
11. → nativeTranscribe48k(fullPcm) on background coroutine
12. → Rust: downsample 48k→16k → whisper_full() inference (1-3 seconds)
13. → text result bubbles back → TranscriptionEntry StateFlow → UI updates
    (placeholder "[name spoke for Xms]" shown immediately, replaced on completion)
```

---

## File Reference — Rust Native

| File | Purpose | Notes |
|------|---------|-------|
| `lib.rs` | Module declarations | Add new modules here |
| `jni_bridge.rs` | ALL JNI exports | ~2100 lines, one file by design |
| `state.rs` | Global app state | `StateMachine` (audio/transport/session), `JniAppState` (UI-facing) |
| `audio_pipeline.rs` | TX/RX audio threads | Wire frame pack/unpack, dual-transport send |
| `transport.rs` | TransportManager | Routes to WiFi/Cellular/BT, per-channel crypto dispatch |
| `crypto.rs` | AES-256-GCM | encrypt/decrypt, X25519 ECDH key exchange |
| `session.rs` | SessionRegistry | 8 channel slots, QR generate/import, group names |
| `codec.rs` | Opus | VoiceEncoder/VoiceDecoder, 48kHz 20ms frames |
| `audio.rs` | Android audio | AudioRecord + AudioTrack via JNI reflection |
| `audio_cache.rs` | Multi-speaker queue | Per-sender queuing, replay buffer |
| `users.rs` | UserRegistry | Peer tracking, mute state, favorite state, display name |
| `wifi_transport.rs` | UDP multicast | Socket management, 239.255.42.42:5555 |
| `cellular_transport.rs` | WS relay bridge | Thread-safe queue; Kotlin polls outbound, pushes inbound |
| `transcription.rs` | Whisper engine | WhisperEngine struct, 48k→16k downsample |
| `whisper_ffi.rs` | Whisper C FFI | Calls into whisper_wrapper.c |
| `opus_ffi.rs` | Opus C FFI | Calls into libopus |
| `build.rs` | Build script | Compiles libopus from audiopus_sys source, links whisper static libs |

---

## File Reference — Kotlin App

| File | Purpose | Notes |
|------|---------|-------|
| `SassyTalkNative.kt` | JNI bridge object | All `external fun` declarations + session persistence helpers |
| `MainActivity.kt` | Entry point | Permission handling, service binding |
| `WalkieService.kt` | Foreground service | Multicast/wake locks, persistent notification |
| `AutoConnectManager.kt` | Dual-transport connect | WiFi + relay simultaneously, NetworkCallback failover |
| `CellularWebSocketClient.kt` | OkHttp WebSocket | Connects to relay.sassy-consults.com, calls JNI push/callbacks |
| `TranscriptionBridge.kt` | VAD + Whisper bridge | PCM accumulation, silence detection, placeholder entries |
| `WhisperModelManager.kt` | Model download | Fetches ggml-tiny.en.bin from HuggingFace, progress UI |
| `BleSignalingService.kt` | BLE GATT server | Peer discovery via Bluetooth advertising |
| `BluetoothTransport.kt` | RFCOMM audio | Bluetooth data channel for audio |
| `PttCoordinator.kt` | BLE PTT | PTT signaling over BLE + RFCOMM TX pump |
| `ui/AppNavigation.kt` | Screen routing | Session restore, init sequencing, **AutoConnectManager singleton** |
| `ui/MainScreen.kt` | Main PTT screen | PTT button, channel/subchannel picker, QR dialog, reconnect |
| `ui/QRAuthScreen.kt` | QR flow | Generate/scan/paste, channel picker (1-8), group name field |
| `ui/UsersScreen.kt` | Peer list | Mute/favorite toggles, auto-refresh every 2s |
| `ui/TranscriptionFeedScreen.kt` | Whisper log | Placeholder → real text update, scroll |
| `ui/ProfileScreen.kt` | Avatar setup | Name + emoji + color |

---

## File Reference — Cloudflare

| File | Deployment | Purpose |
|------|-----------|---------|
| `sassyconsultingllc-cloudflare/src/worker.js` | sassyconsultingllc (sassyconsultingllc.com) | Website + R2 downloads + APIs. PTT routes → 301 to relay domain |
| `sassyconsultingllc-cloudflare/wrangler.jsonc` | — | D1 db, R2 bucket, DO bindings for main worker |
| `sassy-talk-clean/cloudflare-worker/src/ptt-relay-worker.js` | sassytalk-relay (relay.sassy-consults.com) | Pure relay entry point. Routes: /health, /ws, /api/ptt/ws |
| `sassy-talk-clean/cloudflare-worker/src/ptt-relay.js` | — | PttRoom Durable Object. 16 peers/room, hibernation-aware |
| `sassy-talk-clean/cloudflare-worker/wrangler.toml` | — | Relay worker config. PTT_RELAY binding, relay.sassy-consults.com route |
| `sassy-talk-clean/cloudflare-worker/src/worker.js` | DISABLED / stub | Old combined worker. DO NOT DEPLOY. |

---

## Known Issues & Watch List

### Active Landmines

| Issue | Symptom | Fix |
|-------|---------|-----|
| `init_transcription_bridge_class()` not called | Transcription silently fails on both devices | Must be called in `nativeInit` while on main thread (has app classloader) |
| Legacy `crypto` field not set on QR import | Audio "UNENCRYPTED" after scanning new QR | `nativeImportSessionFromQR` must call both `set_channel_crypto()` AND `set_crypto()` |
| `restoreSession()` before transport init | Crypto set then lost when transport initializes | Call `restoreSession()` AGAIN after `autoConnect()` callback fires |
| `AutoConnectManager` recreated per compose | Transport killed on every navigation | Must be `remember { AutoConnectManager(...) }` at AppNavigation level, not MainScreen |
| `DisposableEffect` calling `disconnect()` | Multicast dies when user navigates to UsersScreen | Remove any `onDispose { autoConnect.disconnect() }` from MainScreen |
| `getSessionId()` reading root JSON | Returns null for new QR format | Scan `channels[]` array for session_id, not root JSON field |
| Durable Object hibernation wake | 4000 Unknown client on reconnect | `ws.deserializeAttachment()` must be called before rejecting new clients |
| `c++_shared` not bundled on older devices | Crash on A03s (NDK ABI issue) | Use `c++_static` + `c++abi` in build.rs, never `c++_shared` |
| BLE advertising error 1 on some devices | Cosmetic — doesn't affect audio | Non-fatal, ignore unless BT is primary transport |
| Heartbeat beacons trigger AudioCache entries | Noisy logs | 1-frame 20ms heartbeats should be filtered in audio_cache before queuing |
| `nativeGetUsers()` reading wrong registry | Users never show in UI | Must read from `StateMachine.user_registry` not `JniAppState.user_registry` |

### Architecture Constraints

- **All JNI in one file** (`jni_bridge.rs`) — 2100+ lines is intentional. Do not split. Rust JNI with multiple files creates symbol export nightmares.
- **Audio via JNI reflection** (`audio.rs`) — not AAudio/Oboe. Intentional for max API level compat.
- **Whisper via CMake** (`build.rs`) — not cc crate. whisper.cpp requires CMake. Pre-built static libs in `whisper-libs/{arm64-v8a,x86_64}/`. Do not try to build whisper from source at compile time.
- **Subchannel enforced in Rust** — Kotlin never filters subchannels. RX thread in audio_pipeline.rs does the channel+subchannel check.

---

## Pending Features (not yet built)

| Feature | Description |
|---------|-------------|
| Push notification on transcription | Background notification: "Alex: 'Meet at the corner'" when Whisper finishes |
| About page | Privacy policy link, per-permission explanations, copyright, version |
| PTT hold + buffer | Buffer audio during hold, burst-send entire message on release (not live streaming) |
| Session crypto re-apply | Fully robust re-apply on every reconnect event, not just app start |
| Replay last 10 min | Audio cache replay from TranscriptionFeedScreen |
| Notification when Whisper loads | Loading indicator when ggml-tiny.en.bin downloads/initializes |
| macOS/iOS | Tauri 2.0 desktop builds exist in structure, not started |
| Main worker compartmentalization | Break `sassyconsultingllc-cloudflare/src/worker.js` into per-page workers |
| Remove mybestsites.online | Leftover code in main worker, strip it |

---

## Bugs Squashed (Hall of Shame)

| Bug | Root Cause | How It Manifested |
|-----|-----------|-------------------|
| Transcription never worked | `init_transcription_bridge_class()` defined but never called | Zero transcription entries, no error anywhere |
| Users never showed on remote devices | `nativeGetUsers` read from `JniAppState.user_registry` (always empty) instead of `StateMachine.user_registry` (where RX thread writes) | User list blank even with active peers |
| Session cleared on navigation | `DisposableEffect.onDispose` in MainScreen called `autoConnect.disconnect()` | Navigate to Users screen → back → no audio |
| UNENCRYPTED after app restart | `restoreSession()` called before transport init; transport init wiped the crypto state | App showed UNENCRYPTED badge, audio sent in plaintext |
| UNENCRYPTED after scanning QR | `nativeImportSessionFromQR` set `channel_crypto` but not legacy `crypto` field | Scan QR → no change in encryption status |
| Relay 500 error | Worker bound as `PTT_RELAY` but code called `env.PTT_ROOMS.idFromName()` | All relay connections failed with HTTP 500 |
| Relay 4000 Unknown client | DO hibernation wake didn't restore client attachment before rejecting | Clients got disconnected on DO cold start |
| APK crash on A03s | `libsassytalkie.so` linked `c++_shared` dynamically; .so not bundled in APK | Immediate crash on launch on Android 9/10 devices |
| APK downloaded as .zip | Worker returned `application/octet-stream` for .apk extension | Browser renamed .apk to .zip on download |
| `getSessionId()` null | New QR JSON moved `session_id` into `channels[]`; code still read root JSON | Relay room ID was null, all relay connects landed in wrong room |
| Channel picker didn't scroll | LazyRow missing modifier for scrollability | Couldn't select channels 5-8 |
| Three-person session silent | AutoConnectManager recreated on every MainScreen compose → transport reinit mid-session | Third peer heard nothing |
| Audio duplicated on speaker | AudioCache queued heartbeat beacons as utterances | Clicking/double-play on every PTT release |

---

## Build & Deploy

### Build APK

```bash
# From android-native/
cargo ndk -t arm64-v8a -t x86_64 build --release

# Copy .so files
cp target/aarch64-linux-android/release/libsassytalkie.so \
   ../android-app/app/src/main/jniLibs/arm64-v8a/
cp target/x86_64-linux-android/release/libsassytalkie.so \
   ../android-app/app/src/main/jniLibs/x86_64/

# From android-app/
./gradlew assembleRelease

# Install to all connected devices
adb install -r app/build/outputs/apk/release/app-release.apk
```

### Upload to R2

```bash
npx wrangler r2 object put sassy-downloads/sassytalk/android/SassyTalk-latest.apk \
  --file=app/build/outputs/apk/release/app-release.apk \
  --remote \
  --content-type="application/vnd.android.package-archive"
```

### Deploy Relay

```bash
# ONLY from sassy-talk-clean/cloudflare-worker/
cd sassy-talk-clean/cloudflare-worker
npx wrangler deploy
# → deploys to relay.sassy-consults.com
```

### Deploy Website Worker

```bash
# ONLY from sassyconsultingllc-cloudflare/
cd sassyconsultingllc-cloudflare
npx wrangler deploy
# → deploys to sassyconsultingllc.com
```

**Never cross-deploy.** The relay worker config is `wrangler.toml`. The main worker config is
`wrangler.jsonc`. They are in different directories. Deploying the wrong one overwrites the wrong
domain.

### Build Dependencies

| Dependency | Version | Notes |
|-----------|---------|-------|
| Rust | 1.93+ | stable toolchain |
| cargo-ndk | latest | `cargo install cargo-ndk` |
| Android NDK | 26.1 | Linkers configured in `.cargo/config.toml` |
| Node.js | 18+ | For wrangler |
| Wrangler | 3.x | `npm i -g wrangler` |
| Keystore | — | `app/keystore/release.keystore`, alias: sassytalkie, pass: sassytalk2025 |
| Whisper libs | pre-built | `whisper-libs/{arm64-v8a,x86_64}/` — do not rebuild |

---

## Testing Verification Checklist

After any significant change:

- [ ] S24 generates QR for channel 2, A03s scans → both show channel 2 with group name
- [ ] Both devices on channel 2: PTT works bidirectionally, audio plays, users appear in list
- [ ] S24 switches to channel 1 (no key on A03s): A03s hears nothing
- [ ] Kill and reopen app on both: sessions restore, no re-auth needed
- [ ] Navigate to Users screen and back: audio still works (AutoConnectManager not destroyed)
- [ ] S24 drops WiFi: A03s (relay only) continues to receive via relay
- [ ] Three-person session (S24 + A03s + A17): all hear each other simultaneously
- [ ] Transcription: speech creates placeholder immediately, updates with text in 1-3s
- [ ] APK downloaded from sassyconsultingllc.com saves as `.apk`, not `.zip`
- [ ] Relay health: `curl https://relay.sassy-consults.com/health` returns `{"status":"ok",...}`
