<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-P4Z5QTGY2BWU
-->
# SassyTalkie — Non-Terrestrial Network (Satellite) Support Design

**Status:** design (2027–2028 planning horizon)
**Scope:** how SassyTalkie behaves when the only path off-device is a
satellite / NTN link — Android 15+ carrier satellite messaging, the broader
direct-to-cell trend (Starlink Direct-to-Cell, AST SpaceMobile, carrier NB-NTN),
and emergency-satellite fallbacks.
**Grounding:** this design reuses, not reinvents, three things already in the
codebase — the relay's **store-and-forward ring buffer + FCM wake**
(`cloudflare-worker/src/ptt-relay.js`), the **`audio_cache.rs` Replay path**, and
the **Opus encoder** (`android-native/src/codec.rs`). NTN support is a *new
degradation tier on top of existing machinery*, not a new transport.

> **Core strategy in one line.** On a detected NTN/limited link, SassyTalkie
> **stops pretending it can stream live audio** and instead degrades to
> **asynchronous "voice-mail": record → compress hard → enqueue to the relay's
> store-and-forward buffer → FCM-wake the recipient → they catch up via the
> existing Replay path**, with a text/pre-canned fallback when even that is too
> expensive.

---

## 1. The reality of NTN links

NTN is **not a thin cellular link — it is a different regime**, and the app must
treat it as such:

- **Bandwidth:** carrier NB-NTN / direct-to-cell messaging is measured in
  **bytes-to-low-kbps**, often **text-only** at first rollout. Even
  data-capable direct-to-cell is a small fraction of LTE and is **shared and
  scheduled** across a satellite beam's whole footprint.
- **Latency:** LEO round-trips add tens-to-hundreds of ms *just for propagation*;
  GEO is ~500–600 ms one way. Add scheduling/queueing and you are well past the
  conversational-delay threshold.
- **Intermittency:** coverage comes and goes with satellite passes, look-angle,
  and obstructions. Links **drop and re-acquire** on a cadence of seconds to
  minutes. "Connected" is a transient state, not a steady one.
- **Cost/power:** transmit windows are precious; the radio is power-hungry.
  Chattiness (our normal ~50 fps heartbeat + audio) is actively harmful here.

**Consequence:** live Opus streaming is **off the table** on NTN. Our normal
path is ~50 frames/sec of 20 ms Opus at 32 kbps (`codec.rs` `VOICE_BITRATE =
32000`, `CODEC_FRAME_SIZE = 960`), plus heartbeats and the relay's 120 msg/s
allowance — none of which fits an NTN budget. The relay even documents normal
PTT as "~50 frames/sec ... 20 ms Opus" (`ptt-relay.js`). On NTN we must collapse
that to a **single, heavily-compressed, store-and-forward voice blob per
utterance** — or to text.

---

## 2. How SassyTalkie adapts on a detected NTN / limited link

The adaptation is a **layered fallback**, applied in order of link severity. Each
layer reuses existing code.

### 2.1 Layer 1 — Lean on the store-and-forward relay queue (async voice-mail)

The relay **already has a store-and-forward buffer** built for exactly the
"peer was offline / on a bad link, replay what they missed" case
(`ptt-relay.js`, "Store-and-forward (async voice / catch-up)"):

- A DO-local ring buffer of recent broadcast frames
  (`BUFFER_MAX_FRAMES = 1500` ≈ 30 s, `BUFFER_MAX_BYTES = 2 MB`,
  `BUFFER_TTL_MS = 30_000`).
- A reconnecting peer requests catch-up (`?catchup=1` / `?since=<ms>`) and is
  replayed missed audio wrapped as **`OP_REPLAY_FRAME` (0x19)**
  (`core/src/protocol.rs`, `buildReplayFrame`).

**NTN reuse:** the *sender* on NTN doesn't stream — it uploads **one complete
utterance** (a single compressed blob, see §2.3) into this buffer, then
disconnects to save the radio. The *receiver*, when their own NTN window opens,
pulls it via the existing `?catchup`/`OP_REPLAY_FRAME` mechanism. The 30 s /
2 MB / 30 s-TTL bounds are tuned for terrestrial catch-up and will need an
**NTN profile with longer TTL and a per-utterance (not per-frame) buffering
mode** — see §6 — but the *mechanism* is already there.

### 2.2 Layer 2 — FCM wake to trigger catch-up

On a terrestrial link a sender emits **`OP_WAKE` (0x17)** / **`OP_PTT_START_V2`
(0x15)**, and the relay fans out an **FCM push** to any registered-but-offline
peer (`ptt-relay.js` `firePushesForOfflinePeers`, throttled by
`FCM_PUSH_COOLDOWN_MS = 10_000`). FCM itself rides whatever tiny data path
exists — including, increasingly, carrier NTN messaging.

**NTN reuse:** the FCM wake is the **signaling primitive** that tells a
backgrounded peer "there is a voice-mail waiting for you; open a window and pull
it." On NTN we *decouple* the wake (cheap, tiny, tolerant of latency) from the
audio (expensive, deferred). The sender's blob upload + a single `OP_WAKE` is
the whole NTN send transaction; the receiver's FCM-triggered catch-up is the
whole NTN receive transaction. No live socket needs to persist across the
satellite pass.

### 2.3 Layer 3 — Aggressive codec settings

`codec.rs` currently runs Opus at **32 kbps, 20 ms frames, complexity 5, in-band
FEC, 10% expected loss**. That is right for cellular and wrong for NTN. Define an
**NTN codec profile**:

| Setting | Terrestrial (today) | NTN profile |
|---|---|---|
| Bitrate | 32 kbps (`VOICE_BITRATE`) | **lowest intelligible** — ~6 kbps (Opus floor for usable voice) |
| Frame size | 20 ms (`CODEC_FRAME_SIZE = 960`) | **longer frames** (60 ms) — fewer packets, less per-packet overhead |
| Application | `OPUS_APPLICATION_VOIP` | keep VOIP (speech-optimized) |
| FEC | in-band FEC on, 10% loss | **on, higher expected-loss %** — NTN drops more |
| Delivery | live, per-frame fan-out | **one concatenated blob per utterance**, store-and-forward |

The win is multiplicative: ~5× lower bitrate × longer frames (fewer
packets/overhead) × one-blob-instead-of-streaming turns a 5-second utterance
from hundreds of frames at 32 kbps into a **single small payload** that can fit
an NTN window. The encoder already exposes the relevant `opus_encoder_ctl`
knobs (bitrate, FEC, packet-loss-perc are all set in `VoiceEncoder::new`); the
NTN profile is a **parameter set + a "buffer whole utterance then upload" path**,
not new codec code.

### 2.4 Layer 4 — Text / pre-canned message fallback

When even a 6 kbps voice blob won't fit the window (first-gen carrier NTN is
frequently **text-only**), drop to **text**:

- **Pre-canned messages** ("ON SCENE", "EN ROUTE", "NEED BACKUP", "OK/COPY") —
  a handful of bytes each, the realistic floor for NTN messaging, and a natural
  fit for a tactical/PTT audience.
- **Free text** as a secondary option where the link allows.
- **Optional local transcription:** the codebase already has Whisper FFI
  (`android-native/src/whisper_ffi.rs`, `transcription.rs`). On NTN we can
  **transcribe the operator's utterance on-device and send the transcript** —
  text costs a fraction of even the most aggressive voice blob, and the
  recipient gets the content immediately while any voice blob trickles behind it.

### 2.5 Tie-back to `audio_cache.rs` Replay mode

The receive side is **already built**. `audio_cache.rs` has a `Replay`
`CacheMode` and the machinery around it:

- Caught-up `OP_REPLAY_FRAME` audio decodes and feeds the cache exactly like
  live audio; the cache's **`history` ring** (`max_history` entries) holds it.
- `replay_by_id` / `get_history_frames` let the UI play a received voice-mail
  on demand — `get_history_frames` explicitly "works whether or not a transport
  is currently active," which is precisely the NTN situation (link already
  closed by the time the user taps play).
- `clear_active` preserves `history` across disconnect/reconnect — so a voice-mail
  pulled during one satellite pass survives the inevitable drop before the user
  listens.

So an NTN-delivered utterance lands in the **same Replay/history surface** as a
terrestrial catch-up. The UI's existing "catch-up indicator" and timeline replay
button work unmodified; NTN just changes *how the bytes arrived*, not *how
they're presented*.

---

## 3. Android API surface — detecting NTN / limited connectivity

Detection drives the degradation state machine (§4). Sources, roughly in order
of directness:

- **Android 15 (API 35) carrier satellite signals.** Android 15 surfaces
  carrier-satellite connectivity. The relevant detectors:
  - `NetworkCapabilities` — inspect the active network for the
    **NOT-bandwidth-constrained / metered / restricted** capabilities and the
    **satellite-capable** signals exposed on 15+. A network reporting satellite
    backing or extreme constraint → enter NTN tier.
  - `TelephonyManager` / carrier APIs — carrier satellite messaging state.
  - `ConnectivityManager.NetworkCallback` — react to transitions (terrestrial →
    satellite → none) in real time rather than polling.
- **Bandwidth estimate as a proxy.** `NetworkCapabilities.getLinkDownstreamBandwidthKbps()`
  / upstream estimate: a link reporting single-digit-to-low-kbps upstream is, for
  our purposes, NTN-class regardless of *why* — treat low estimated bandwidth as
  a trigger even on pre-15 devices or non-satellite-but-terrible links.
- **`isActiveNetworkMetered()` + constrained flags** — NTN is invariably metered
  and constrained; combine with the bandwidth estimate to avoid false positives
  on a merely-metered-but-fast cellular link.
- **Empirical fallback (works everywhere, no new permissions).** Our transport
  already measures health: `cellular_transport.rs` tracks an outbound queue with
  `is_outbound_congested()` / `dropped()` counters. **Sustained outbound
  congestion + RTT blowout (ping/pong in `ptt-relay.js`) + repeated reconnects**
  is a transport-level signature of an NTN-class link even when the OS doesn't
  label it. This is the **portable detector** — it needs no Android 15 API and
  no new permission, and it's the safety net when OS signals are absent or wrong.

**Design choice:** consume OS signals **when present** (Android 15+), but make
the **empirical congestion/RTT/reconnect heuristic authoritative** so the
feature degrades gracefully on every device and OEM regardless of NTN-API
rollout.

---

## 4. Graceful-degradation state machine

A single explicit state machine drives the tiers. It is monotonic-with-hysteresis:
**degrade fast** (one bad signal is enough — don't waste an NTN window proving
the link is bad), **recover slow** (require sustained good signal before resuming
live, to avoid flapping mid-conversation).

```
        ┌──────────────┐  good link sustained (N s)   ┌──────────────┐
        │   TERRESTRIAL │ ◀─────────────────────────── │  DEGRADING   │
        │  (live Opus,  │                              │ (probation)  │
        │  50 fps, 32k) │ ───────────────────────────▶ │              │
        └──────┬───────┘   congestion / RTT / OS-NTN   └──────┬───────┘
               │                                              │ confirmed NTN-class
               │ catastrophic drop                            ▼
               │                                       ┌──────────────┐
               │                                       │  NTN_VOICE   │
               │                                       │ async voice- │
               │                                       │ mail: 6kbps  │
               │                                       │ blob + wake  │
               │                                       └──────┬───────┘
               │                                              │ blob won't fit /
               ▼                                              ▼ text-only link
        ┌──────────────┐                               ┌──────────────┐
        │   OFFLINE    │ ◀──────────────────────────── │  NTN_TEXT    │
        │ (queue local,│   no path at all              │ pre-canned / │
        │ wake on recon)│                              │ transcript   │
        └──────────────┘                               └──────────────┘
```

State behaviors:

- **TERRESTRIAL** — current behavior. Live fan-out, normal codec, heartbeats.
- **DEGRADING (probation)** — a trigger fired but isn't confirmed. Immediately
  **stop new live streaming**, raise the jitter buffer, and start buffering the
  current utterance whole (so if it confirms NTN, the utterance is already a
  blob ready to upload). Cheap, reversible.
- **NTN_VOICE** — confirmed NTN-class. Per utterance: record → encode with the
  **NTN codec profile** (§2.3) → upload one blob to the relay store-and-forward
  buffer → emit one `OP_WAKE` → close the socket. Receive: on FCM wake, open a
  window, `?catchup` pull, feed `audio_cache.rs` Replay/history.
- **NTN_TEXT** — voice blob won't fit. Pre-canned messages / free text /
  on-device transcript (§2.4). Same wake + store-and-forward envelope, tiny
  payload.
- **OFFLINE** — no path at all. Queue locally (the `cellular_transport.rs`
  outbound queue already buffers), and on reconnect replay the wake + blob.
  The relay's FCM-wake-on-reconnect path handles the inverse (peers who were
  offline get woken).

Transitions are driven by §3's detector. Hysteresis: a single congestion/RTT/OS
signal degrades; **N seconds of clean live traffic** is required to climb back
toward TERRESTRIAL, stepping down one tier at a time (NTN_TEXT → NTN_VOICE →
TERRESTRIAL) so we never yo-yo a user between "live" and "voice-mail" mid-talk.

---

## 5. Feasible now vs. dependent on carrier/OEM rollout

**Feasible now — no NTN hardware required:**

- The **empirical NTN-class detector** (congestion + RTT + reconnect heuristic)
  on top of `cellular_transport.rs` — works on any link, any device.
- The **degradation state machine** and the **NTN codec profile** (it's an Opus
  parameter set + a buffer-whole-utterance path; `codec.rs` already exposes the
  knobs).
- **Store-and-forward voice-mail** end-to-end — the relay buffer
  (`ptt-relay.js`), `OP_REPLAY_FRAME`, FCM wake, and `audio_cache.rs` Replay are
  all already shipping. We can build and test the *entire async-voice experience*
  on a deliberately throttled normal link, today, before any satellite exists in
  the path.
- **Text / pre-canned fallback** and **on-device transcript** (Whisper FFI is
  already in-tree).
- An **NTN store-and-forward relay profile** (longer TTL, per-utterance blob
  buffering) — a relay-side config change, fully under our control.

**Dependent on carrier/OEM NTN rollout (not in our control):**

- The **Android 15+ satellite `NetworkCapabilities`/carrier signals** — only
  meaningful on devices + carriers that expose them. We treat these as a
  *better* detector when available, never a *required* one.
- **Whether FCM wake and a tiny data/text payload actually traverse a given
  carrier's NTN** — first-gen carrier NTN is often text-only and allowlisted;
  our text/pre-canned tier exists precisely for that floor, but *which* bytes a
  carrier will carry is their policy, not ours.
- **Direct-to-cell data capacity** (Starlink D2C, AST SpaceMobile, etc.)
  improving enough to fit a 6 kbps voice blob in a usable window — trending the
  right way through 2027–2028 but device/region/carrier-gated.

**Design stance:** build the *whole adaptation* against our own throttle and the
empirical detector now, so when a carrier/OEM NTN path lights up the app already
knows how to behave — the satellite link becomes "just another constrained
transport" the state machine already handles, and the OS satellite signals
become a detection *upgrade* rather than a prerequisite.

---

## 6. Open items / required changes (summary)

- **Relay NTN profile:** longer `BUFFER_TTL_MS`, a per-utterance (blob) buffering
  mode distinct from the per-frame ring, and an upload endpoint shape for
  "one complete utterance" rather than streamed frames. (`ptt-relay.js`)
- **`codec.rs` NTN profile:** parameterize bitrate/frame-size/FEC and add a
  "buffer whole utterance → encode → emit one blob" path alongside the live
  encoder.
- **Detector:** implement the empirical congestion/RTT/reconnect classifier over
  `cellular_transport.rs`, plus an Android 15 `NetworkCapabilities`/carrier-signal
  adapter behind a capability check.
- **State machine:** the TERRESTRIAL ↔ DEGRADING ↔ NTN_VOICE ↔ NTN_TEXT ↔ OFFLINE
  controller with degrade-fast/recover-slow hysteresis.
- **UI:** surface the tier ("Satellite — voice-mail mode") and reuse the existing
  catch-up indicator + Replay timeline for delivery; expose pre-canned messages.
- **No change needed** to: `audio_cache.rs` Replay/history (already fits),
  `OP_REPLAY_FRAME`/`OP_WAKE` protocol opcodes (already defined in
  `core/src/protocol.rs`), or the E2E crypto path (NTN blobs are still
  AES-256-GCM frames — the relay stays a blind forwarder).
