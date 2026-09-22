<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-LRGRYMMVA2XI
-->
# Google Play Data Safety Declaration

## Sassy-Talk by Sassy Consulting LLC

> **Key point for reviewers:** Sassy-Talk has two classes of transport. On **local
> transports** (Bluetooth LE, Wi-Fi Direct / same-network) **no data leaves the
> device to any server**. On the **relay transport** (internet fallback we operate
> on Cloudflare) traffic is **end-to-end encrypted** — the relay is a blind
> forwarder that handles only ciphertext plus the minimal routing metadata below,
> and stores no audio. The declarations below describe the relay path, which is
> the only path on which any data is transmitted off the device.

### Data Collection Summary

| Category | Collected | Shared | Ephemeral | Purpose |
|----------|-----------|--------|-----------|---------|
| Personal info | ❌ No | ❌ No | - | - |
| Financial info | ❌ No | ❌ No | - | - |
| Health and fitness | ❌ No | ❌ No | - | - |
| Messages | ❌ No | ❌ No | - | - |
| Photos and videos | ❌ No | ❌ No | - | - |
| Audio (voice) | ⚠️ Relay only | ❌ No | ✅ Yes | App functionality (real-time voice) |
| Files and docs | ❌ No | ❌ No | - | - |
| Calendar | ❌ No | ❌ No | - | - |
| Contacts | ❌ No | ❌ No | - | - |
| App activity (connection metadata) | ⚠️ Relay only | ❌ No | ✅ Yes | App functionality, abuse prevention |
| Web browsing | ❌ No | ❌ No | - | - |
| App info and performance | ❌ No | ❌ No | - | - |
| Device or other IDs | ⚠️ Relay only | ❌ No | Partly | App functionality (peer routing, push wake) |

### Detailed Explanations

#### Audio / Voice (relay path only — processed ephemerally)
- **What:** Voice audio during push-to-talk.
- **Why:** Core app functionality — walkie-talkie communication.
- **Local transports:** Never leaves the device/local network. Not collected.
- **Relay transport:** End-to-end encrypted (AES-256-GCM) on the device before
  sending; the relay forwards only ciphertext and **cannot decrypt it**.
- **Storage:** NOT stored. Forwarded in real time and immediately discarded.
- **Sharing:** NOT shared with third parties.

#### Device / Other IDs (relay path only)
- **What:** A randomly generated per-install peer ID and a random per-connection
  client ID. Plus, **only if you enable wake notifications**, a Firebase Cloud
  Messaging (FCM) token.
- **Why:** Routing the right peers into the same room, and (optionally) waking the
  app for an incoming transmission while it is backgrounded.
- **Local transports:** Not transmitted. Visible only to devices on the same
  local network/radio link.
- **Relay transport:** Peer/client IDs are held in memory during the session. If
  wake notifications are enabled, the FCM token is stored to map (room → peer →
  token); push delivery is performed by Google (FCM). The push payload carries
  only a room identifier, never audio or message content.
- **Sharing:** Not shared for advertising or analytics. FCM is a delivery
  service provider (Google).

#### App Activity — Connection Metadata (relay path only — ephemeral)
- **What:** Room open/close, peer join/leave, heartbeats, rate-limit events.
- **Why:** Managing room membership, detecting dropped peers, preventing abuse.
- **Storage:** Held in memory during the session; may appear in short-lived
  operational logs that are rotated out. Contains **no audio content**.

### Security Practices

✅ Data is end-to-end encrypted (AES-256-GCM) with X25519 key agreement
✅ Data is encrypted in transit
✅ On local transports, no data is transmitted off the device at all
✅ On the relay transport, only ciphertext + minimal routing metadata transits; the relay cannot decrypt audio
✅ No audio is ever recorded or persisted
✅ No third-party analytics or advertising SDKs
✅ Users can request that an encrypted invitation blob be invalidated
✅ Self-hosting is supported for organizations requiring full data sovereignty

### Data Deletion

There are no user accounts and no stored personal profiles. Users stop all
processing by leaving relay rooms, disabling push notifications, or uninstalling
the app (which removes all local data). Encrypted invitation blobs expire
automatically; contact us to invalidate one sooner.

### Contact

privacy@sassyconsultingllc.com
