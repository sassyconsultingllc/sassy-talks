# SassyTalk v1 Play Store Launch — Design Document

> Date: 2026-03-30
> Status: Approved

---

## Overview

Six additions for Play Store launch. No bloat — each item is either a store requirement or a direct user-facing feature that's been validated.

---

## 1. Privacy Policy (All Jurisdictions)

**URL:** `https://sassyconsultingllc.com/privacy/sassytalk/`
**Format:** Single HTML page served by main Cloudflare worker.

**Jurisdictions:** GDPR (EU/EEA/UK), CCPA/CPRA (California), COPPA (children), PIPEDA (Canada), LGPD (Brazil), POPIA (South Africa), Google Play data safety requirements.

**Honest data disclosure:**

Data collected (via Cloudflare infrastructure):
- Usage timestamps (relay connection times)
- IP addresses (WebSocket connections to relay.sassy-consults.com)
- Relay room metadata (room IDs, peer counts — Durable Object routing)

Data shared with other users:
- Encrypted audio bundles (AES-256-GCM, decrypted only by session key holders)
- Display name + emoji avatar (wire frame header, unencrypted)
- Device sender ID (wire frame header, unencrypted)

Data NOT collected:
- Voice content (relay cannot decrypt)
- Transcription text (never leaves device)
- Account info (no accounts exist)

Cloudflare disclosure: Cloudflare as infrastructure provider logs request metadata (timestamps, IPs, request counts) per their own privacy policy. We don't control or access those logs.

**Permissions explained:**
- RECORD_AUDIO — capture voice for push-to-talk
- CAMERA — scan QR codes for session authentication
- BLUETOOTH (Connect, Scan, Admin) — peer discovery
- INTERNET — cellular relay connection
- FOREGROUND_SERVICE — keep radio active when screen off

---

## 2. In-App About Screen

**File:** `AboutScreen.kt` in existing `ui/` package.
**Access:** Info/gear icon in MainScreen header bar.
**Type:** Read-only information screen, no settings toggles.

**Content:**
- App name + version (from BuildConfig)
- "Encrypted Push-to-Talk" tagline
- Connection status summary (WiFi/Relay/BT at a glance)
- Privacy Policy link (opens system browser)
- Permission explanations with status indicators
- Whisper model status (downloaded/not, size on disk)
- Relay server status (connected/disconnected)
- Credits: "Built by Sassy Consulting LLC"

**Anti-bloat:** No theme picker, no notification settings, no account management, no toggle switches.

---

## 3. Push Notification on Transcription

**Integration:** `WalkieService.kt` foreground service + `TranscriptionBridge.kt`.

**Behavior:**
- Fires only when app is NOT in foreground
- Format: "[Name]: [transcription text]"
- Tap opens app to TranscriptionFeedScreen
- Uses existing foreground service notification channel
- Respects mute state (muted users = no notification)

**Anti-bloat:** No per-user notification settings, no grouping, no reply-from-notification, no sound customization.

---

## 4. Message Replay from Audio Cache

**Integration:** `audio_cache.rs` (Rust) + `TranscriptionFeedScreen.kt` (Kotlin).

**Behavior:**
- Play button on transcription entries with cached audio
- Plays cached PCM through existing AudioTrack path
- Button hidden for entries where audio has been evicted from cache
- Live incoming audio takes priority over replay playback

**JNI:** New export `nativeReplayAudio(sender_id: String)` in `jni_bridge.rs`.

**Anti-bloat:** No save/export, no scrubbing, no speed controls, no playlist mode.

---

## 5. FLAG_SECURE (Screenshot/Recording Block)

**File:** `MainActivity.kt`
**Implementation:** `window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)`
**Effect:** Blocks screenshots, screen recording, and app preview in recent apps.

One line of code. High security value.

---

## 6. Play Store Listing

**App name:** SassyTalk
**Short description:** Encrypted push-to-talk walkie-talkie. No accounts. No servers storing your voice.
**Category:** Communication
**Content rating:** Everyone
**Price:** Free (future paid tier planned)
**Privacy policy URL:** `https://sassyconsultingllc.com/privacy/sassytalk/`
**Contact:** support@sassyconsulting.com

**Data safety section:**
- Data collected: Usage timestamps, IP addresses (Cloudflare infrastructure)
- Data shared: Encrypted audio (E2E), display name, device ID (with session peers)
- Encryption: Yes (AES-256-GCM)
- Data deletion: N/A for audio (never stored); Cloudflare infrastructure logs per their policy

**Screenshots needed:** Main PTT screen, QR auth, Users list, Transcription feed, About screen.
**Feature graphic:** 1024x500 banner, retro walkie-talkie aesthetic.

---

## Not In v1

- Roger beep / sound effects
- Licensing / payment / Stripe integration in app
- Bluetooth-only mode (not fully wired)
- iOS / macOS / desktop builds
- Settings toggles (audio, notifications, theme)
- Per-user notification preferences

## Future Monetization Strategy

Change relay URL when paid tier launches. Old free APK points to `relay.sassy-consults.com` and stops working. New paid APK points to new relay domain. Clean break, no licensing code needed in app.
