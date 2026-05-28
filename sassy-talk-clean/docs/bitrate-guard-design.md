# Bitrate-guard Design Notes

> Status: **draft, not wired into the wire protocol.** Receiver-side guard fn is implementable today against estimated bitrate; sender-side advertisement requires a protocol version bump that affects every client.

---

## Problem

The Android client encodes Opus at a hardcoded 24 kbps VBR (see `OpusEncoder.applyPttDefaults`). Every SassyTalkie install in the wild encodes identically. But there's nothing on the receive path that *enforces* this — a tampered client, a forked build, or a future iOS port could send 8 kbps (unintelligible mush) or 64 kbps (bandwidth burn) and the receiver would happily decode it.

For groups of 4-6 in Mix mode, mismatched bitrates also cause the AGC to chase its tail — one quiet stream and one loud stream produce a perpetually pumping mix.

## Two options

### Option A: per-frame in-band header byte (in plaintext, inside encryption)

Plaintext payload becomes `[u8 bitrate_bucket][opus_frame_bytes]`. The relay still sees only nonce + ciphertext — no protocol change there. Receiver peels off byte 0 on decrypt, compares to its allowed set, drops the frame if outside.

```
PLAINTEXT (was): [opus_bytes]
PLAINTEXT (v2):  [0xB1][bucket:u8][opus_bytes]
```

Magic byte 0xB1 (sentinel = "bitrate one") disambiguates v1 vs v2 plaintexts. Opus frames never start with a TOC byte of 0xB1 (TOC byte has structured bit layout — `[config:5][s:1][c:2]` — config field max is 31), so a legacy receiver that doesn't know about v2 will fail decode on a v2 frame instead of playing garbage. v2 receivers do the inverse check.

**Bucket values (u8):**

| Bucket | bps     | Use case                           |
|--------|---------|------------------------------------|
| 0x00   | 8000    | Lo-fi / extreme bandwidth squeeze  |
| 0x01   | 12000   | Cellular fallback                  |
| 0x02   | 16000   | Marginal coverage                  |
| 0x03   | 24000   | **Current default**                |
| 0x04   | 32000   | High quality                       |
| 0x05   | 48000   | Stereo / music (not used)          |
| 0x06   | 64000   | Max — only if explicitly configured|

Receiver allow-set defaults to `{0x02, 0x03, 0x04}` (16-32 kbps). Frames outside are dropped + a `BITRATE_REJECT` counter increments per peer. A peer over the reject threshold within a window gets booted from the room.

**Pros:** per-frame, no handshake needed, defends against mid-session tampering.
**Cons:** every frame pays 1 byte overhead (+0.4% bandwidth at 24 kbps / 20 ms). Needs migration plan — v1 and v2 clients can't talk for the duration of the roll.

### Option B: session-level advertise at handshake

Use the existing X25519 key-exchange + session-handshake pathway (see `crypto.rs::SessionKey` and the QR-pair flow in `session.rs`) to negotiate codec params up front:

```
session_handshake_v2 {
  ...existing fields...,
  codec: { name: "opus", bitrate_bps: 24000, frame_ms: 20 }
}
```

Both peers commit to the negotiated bitrate at session start. Receiver still computes estimated bitrate from frame size (bytes × 8 / frame_ms) and rejects anything more than ±50% from the agreed value.

**Pros:** zero per-frame overhead. Cleaner semantically. Reusable for other codec params (frame size, application mode, complexity).
**Cons:** session-scoped — can't change mid-call. A tampered client can lie at handshake then send something else; the guard catches that via the frame-size estimator, but it's a softer enforcement.

## Recommendation

**Ship Option B first.** Add `codec` to the session handshake, wire the size-estimator guard on the receiver. Zero wire-format change, defends 95% of the threat. If you ever need stronger per-frame attestation, Option A can layer on later.

## Receiver guard skeleton (Option B)

Drop this into `audio_cache.rs` or a sibling module. Called from the RX path on every decoded frame before it reaches `ingest_frame`:

```rust
/// Estimated bitrate (bps) for an Opus frame of `frame_bytes` bytes
/// covering `frame_ms` ms of audio.
#[inline]
fn estimate_bitrate_bps(frame_bytes: usize, frame_ms: u32) -> u32 {
    ((frame_bytes as u64 * 8 * 1000) / frame_ms as u64) as u32
}

/// Returns true if the frame's measured bitrate is within ±tolerance_pct
/// of the negotiated bitrate for this session. False → drop frame, bump
/// per-peer reject counter, evict if counter crosses threshold.
pub fn bitrate_within_bounds(
    frame_bytes: usize,
    negotiated_bps: u32,
    tolerance_pct: u32,
) -> bool {
    let measured = estimate_bitrate_bps(frame_bytes, 20);  // 20 ms default
    let delta = if measured > negotiated_bps {
        measured - negotiated_bps
    } else {
        negotiated_bps - measured
    };
    let allowed_delta = negotiated_bps * tolerance_pct / 100;
    delta <= allowed_delta
}
```

Caveats:
- Opus VBR means single-frame measurement is noisy. Use a sliding window of 10 frames before deciding to drop.
- Silence frames (DTX / CN) are tiny — 2-3 bytes regardless of bitrate. Treat any frame under 10 bytes as "exempt" (silence beacon).
- 20 ms frame size is assumed. If you later add 40 ms / 60 ms frames, pass actual `frame_ms` from the session handshake.

## Rollout sequence

1. Add `codec` field to session-handshake JSON. Optional for backward compat (peers without it default to `{opus, 24000, 20}`).
2. Add `estimate_bitrate_bps` + `bitrate_within_bounds` to receiver. Default tolerance 50%.
3. Wire reject counter + auto-evict on 30+ rejects/min per peer.
4. Telemetry: log measured vs negotiated for a week, tune tolerance based on real VBR spread.
5. (Optional) tighten tolerance to 20-30% once you have data.

No client gets dropped during step 1-2 since the guard is observe-only until step 3.

## Why not enforce server-side?

The relay never sees plaintext (E2E AES-GCM). It can't measure bitrate of the encrypted payload meaningfully — ciphertext size is always (nonce + plaintext + tag), so it's just plaintext size + 28 bytes. It COULD enforce a frame-size cap (e.g. reject frames > 200 bytes as suspect) but that's a blunter instrument and applies at the wrong layer (audio QoS should be enforced where audio is decoded).

Server-side ALSO has no way to know what bitrate two peers agreed to at handshake — that information is in the encrypted session blob, by design.
