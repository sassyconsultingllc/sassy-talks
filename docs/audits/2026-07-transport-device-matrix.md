# Transport verification matrix — v3.1.12

Physical devices required for BT RFCOMM and reliable WiFi multicast. Emulators are relay-only and must not be used to sign off local planes.

Log tags: `AutoConnect`, `PTT.Coord`, `BluetoothTransport`, `CellularWS`, Rust `TransportManager` / `StateMachine`.

**v3.1.14 code fixes (2026-07-29):**
- Relay soft rate-limit + soft send backpressure (no flap on first overshoot).
- OkHttp WS send retries when buffer full (was silent TX drop ≈ high “loss”).
- Diagnostics: RX rates, RTT/HB, queue/ws drops; jitter default 5 + adaptive.
- Worker deploy required for relay soft-backpressure to take effect live.

**v3.1.12 code fixes (2026-07-27):**
- BT "connected" / failover / PTT now require **RFCOMM** (BLE-only no longer silent-TX).
- RFCOMM link retry + "Linking Bluetooth…" UI while dialing.
- FGS AudioFocus + tighter RX WakeLock renew + background jitter bump (3→8 frames).
- Share viewer multi-strategy Open button (needs worker deploy — wrangler token missing in this session).

| # | Case | Expect | Pass? |
|---|------|--------|-------|
| 1 | Same LAN, WiFi on, two phones | Status: WiFi (relay backup OK); PTT works without needing relay | |
| 2 | Same LAN, kill relay / airplane data while WiFi stays | Local multicast PTT continues; no FAILED flap | |
| 3 | Disable WiFi, relay still up | Failover to Cloudflare Relay; PTT works | |
| 4 | No internet, RFCOMM paired | BT primary only after RFCOMM up; log `RFCOMM peer(s)`; peers hear TX | |
| 5 | Cellular-only + RFCOMM, then drop mobile data | BT re-promotion — **no silent TX** | |
| 6 | WiFi returns after BT degraded | Upgrade to WiFi; MulticastLock re-acquired | |
| 7 | Relay WS dies while WiFi up | Stay on WiFi (local-first); advisory updates | |
| 8 | Minimize app during peer TX | Loudspeaker stays prioritized; less jitter than pre-3.1.12 | |
| 9 | Open invite via viewer "Open in SassyTalk" | App opens with session (or clear paste fallback) | pending worker deploy |
| 10 | Cellular-only PTT, watch diagnostics | local drop % low; RX pkt/s > 0 while peer talks; ws_tx_drop near 0 | |
| 11 | Induce backpressure (slow peer / airplane blip) | No 1008 rate-limit flap; soft-drop at most | needs worker deploy |

## Unit coverage

- `transport::tests::*` Bluetooth promotion — **PASS**
- `BtAudioPathTest` — **PASS**
- `TransportAdvisorTest` (incl. zero-RFCOMM) — **PASS**

## Physical devices

No handsets attached at build time. Run rows 1–8 and **10–11** on two phones before calling relay loss done.

## Worker deploy

`ptt-relay.js` soft-backpressure + `viewer.js` deep-link strategies are in-tree; deploy with `wrangler deploy` when `CLOUDFLARE_API_TOKEN` is available.
