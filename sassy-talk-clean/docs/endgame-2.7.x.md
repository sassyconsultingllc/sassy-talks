<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-PKYHYTE35FRB
-->
# SassyTalkie v2.7.x — Endgame Roadmap

> Three releases. Finite, named features. After v2.7.2 the app is "done" — further bugs go in the v2.8 backlog instead of fix-of-the-day cycles.

---

## v2.7.0 — Pairing Polish

Closes the pairing-flow rough edges that surfaced through the v2.6.x cycle.

1. **2 s force-reconnect timeout** (already on disk) — drops dead 15 s wait when `pttCoordinator` is null on Auth-screen-first generate. Cosmetic / log-cleanliness; behavior already correct because Rust set_cellular_room covers the case.
2. **Pairing-success toast with context** — instead of "Joined session", show "Joined {host_device_name} on channel {N}". Confirms which room you landed in without guessing.
3. **"Test relay" button on MainScreen** — taps `/auth` against relay.sassyconsultingllc.com, reports HTTP status + round-trip ms in a snackbar. Lets you isolate "is the relay up?" from "are my peers there?".

## v2.7.1 — Group Awareness

Surfaces existing PttCoordinator state that the UI never showed.

4. **Peer roster chip** in MainScreen header — "3 peers · Alice, Bob, Carol". Updates from existing `LivenessTracker.peerIds()` + UserRegistry name lookup.
5. **Peer join/leave toasts** — when `LivenessTracker` reports a state change, fire a snackbar. Hookable from existing `_anyPeerStale` flow.
6. **Cache mini-status on MainScreen** — bottom strip mirroring TranscriptionFeedScreen's cache bar, but compact: just mode pip + "X queued" when non-idle.

## v2.7.2 — Diagnostics & Resilience

Production-ready surfaces — the "this is shipping" tier.

7. **Diagnostic info sheet** — new Settings entry; one-screen dump: relay URL + ws state, current room, peer count, last error, app version + commit SHA, devices serial, copy-all button.
8. **Persisted timeline** — TranscriptionBridge writes entries to a JSON file at every commit (debounced 250 ms), restores on launch. Survives app death without losing the "who spoke when" log.
9. **Network-type indicator** — WiFi vs cellular badge in MainScreen header. Single ConnectivityManager.NetworkCallback registered in WalkieService, reflected as a StateFlow.

---

## Out of scope (deferred to v2.8 or later)

- Nonce-prefix widening (wire-protocol change, needs interop test)
- Lock-order audit across Rust modules (holistic refactor)
- Bitrate-guard implementation (design doc only, see `bitrate-guard-design.md`)
- Client-side mixing tuning (Mix mode shipped in v2.6.3; tuning per real-world feedback)
- Server-side ring-buffer-replay client integration (server has it, client doesn't consume yet)
- Foodie-finder / cohort import from other apps
- Crashlytics / telemetry pipeline beyond the local diagnostic sheet

## Ship gate

After v2.7.2 lands, you install on TC21 + Brick 2.0 and run a 10-minute mixed-use session: pair, talk for 5 minutes both directions, change channels, end + rejoin a session, force a network type change (WiFi off → cellular). If nothing crashes and audio flows both ways, **the app ships to Play Store production track**. Further audits stop and go into a v2.8 backlog file. I am not allowed to start any new "scan for bugs" cycle without explicit ask.
