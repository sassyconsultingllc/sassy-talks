# Efficiency Audit — 2026-07 (v3.1.8)

Scope: `android-app` (Kotlin/Compose), `android-native` (Rust/JNI), `core` (crypto/audio_cache).

**Disposition:** 3.1.8 was audit-only for efficiency. **3.1.9** ships sticky-session UX plus selected P0/P1 efficiency fixes (below).

## Shipped in 3.1.9 (beyond original plan)

- AutoConnect WiFi-loss grace (1.8s) before reconnect UI / failover
- Quiet peer snackbars (no first-sighting toast); soft status copy
- Cellular idle poll 2ms → 20ms
- RX poll scratch buffers (thread-local) in `transport.rs`
- WakeLock decoupled from MulticastLock; 3m activity renew on PTT/RX
- `ByteString.of(buf, 0, len)` (no spread-copy) on cellular TX
- Sticky roster / FCM prefers WalkieService FGS

## Shipped in 3.1.8 (transport — not efficiency)

- Rust: re-promote `active=Bluetooth` when IP dies while RFCOMM is up; keep audio pipeline via `has_live_audio_path()`.
- Kotlin: local-first AutoConnect (WiFi/BT preferred; relay as long-distance backup).
- Relay WS loss callback → stay on WiFi or fall back to BT (not WiFi-`onLost`-only).
- Advisory: relay counted active only when the WebSocket is connected.
- Settings copy updated for local-first policy.

## P0 backlog (fix in a later upversion)

| ID | Finding | Evidence | Suggested fix |
|----|---------|----------|---------------|
| E1 | Idle cellular outbound pump ≈500 JNI polls/sec | ~~`POLL_INTERVAL_MS = 2`~~ → **20 ms in 3.1.9**; still not blocking pop | Optional: `PacketQueue::pop_wait` |
| E2 | Per-poll heap alloc on WiFi + cellular RX | **Thread-local scratch in 3.1.9** | Further: non-blocking WiFi + sleep when empty |
| E3 | `PARTIAL_WAKE_LOCK` (4h) tied to MulticastLock | **Decoupled in 3.1.9** (3m activity renew) | — |
| E4 | JNI `AudioRecord.read` allocates `short[]` every read | `jni_bridge.rs` audio read path | Reuse GlobalRef `short[]` per thread, or Oboe/AAudio later |

## P1 backlog

| ID | Finding | Evidence | Suggested fix |
|----|---------|----------|---------------|
| E5 | Extra copies on cellular TX | **`ByteString.of(buf,0,len)` in 3.1.9** | Longer-term: encrypt into pre-sized buffer |
| E6 | RX String/PCM clones at speak rate | `users.rs` name rewrite; `audio_pipeline` clones | Update name only when changed; move ownership into cache |
| E7 | MainScreen infinite pulse always recomposes | `MainScreen.kt` `rememberInfiniteTransition` | Animate only while transmitting |
| E8 | Always-on cache status poll (JNI + JSON) | `TranscriptionBridge` 500 ms | Poll only while UI subscribed |
| E9 | Duplicate liveness traffic | HB 2s + keepalive 4s + OkHttp ping 15s + TX silence beacon | Document/suppress redundant paths when all up |
| E10 | Telemetry bridge 1 Hz JNI | `WalkieService` | Gate to debug / overlay visible |

## P2 backlog

| ID | Finding | Disposition |
|----|---------|-------------|
| E11 | Whisper leftovers on disk (not linked) | Delete tree to shrink repo |
| E12 | ML Kit / icons size | Feature-critical; size audit only |
| E13 | Replay-window HashSet on decrypt | Correctness > micro-opt |
| E14 | Log volume on connect/PTT | Gate hot-path logs to DEBUG |

## Already healthy

- TranscriptionBridge per-frame JNI gated off by default.
- Shared OkHttp client; reconnect single-flight + `closed` zombie guard.
- Bounded packet queues / playout / history.
- Whisper inference removed from production binary.

## Next efficiency-focused release

Prioritize **E1–E3** (battery/CPU vitals), then **E5/E7/E8**.
