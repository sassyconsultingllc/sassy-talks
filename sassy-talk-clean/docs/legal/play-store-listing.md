<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-PASOSO7H2HMZ
-->
# Google Play Store Listing Content

## App Name
SassyTalk

## Short Description (80 chars max)
Encrypted push-to-talk walkie-talkie. No accounts. No servers storing your voice.

## Full Description (4000 chars max)

SassyTalk is a push-to-talk walkie-talkie that encrypts everything end-to-end. Your voice never touches a server in plaintext. No accounts, no sign-up, no tracking.

**HOW IT WORKS**
Scan a QR code to join a channel. Hold the button to talk. That's it.

Your voice is encrypted with AES-256-GCM on your device before it leaves. Only devices with the same session key can decrypt it. The relay server is a blind forwarder -- it cannot listen.

**TWO WAYS TO CONNECT**
- WiFi multicast: Same network, zero infrastructure, instant
- Cellular relay: Across networks, over cellular, encrypted through Cloudflare

Both run simultaneously. If WiFi drops, the relay keeps going. No interruption.

**8 CHANNELS, 3 SUBCHANNELS EACH**
Organize your team across channels. Each channel has its own encryption key. Channel isolation is enforced by cryptography, not trust.

**ON-DEVICE TRANSCRIPTION**
Whisper AI runs entirely on your phone. Speech is transcribed locally -- no audio is sent to any cloud transcription service. Get background notifications when someone speaks.

**REPLAY MESSAGES**
Missed something? Tap play on any transcription entry to hear it again from the audio cache.

**WHAT WE DON'T DO**
- No accounts or registration
- No cloud storage of your voice
- No analytics or tracking SDKs
- No ads
- No contact list access
- No location tracking

**PERMISSIONS EXPLAINED**
- Microphone: Push-to-talk voice capture
- Camera: QR code scanning for session auth
- Bluetooth: Peer discovery
- Internet: Cellular relay connection
- Foreground Service: Keep the radio on when screen is off

**SECURITY**
- AES-256-GCM end-to-end encryption
- Per-channel encryption keys
- QR-based key exchange (no server sees your key)
- Screenshot and screen recording blocked in-app
- Blind relay (server cannot decrypt your audio)
- On-device transcription (Whisper, never cloud)

Built by Sassy Consulting LLC.

---

## Category
Communication

## Content Rating
Everyone

## Tags/Keywords
walkie talkie, push to talk, PTT, voice chat, encrypted, secure communication, radio, team communication, end to end encryption, offline, no account, privacy

## Contact Email
support@sassyconsultingllc.com

## Privacy Policy URL
https://sassyconsultingllc.com/privacy/sassytalk/

## Website
https://sassyconsultingllc.com

---

## Data Safety Section

### Data collected
- Connection metadata (IP addresses, timestamps) -- collected by Cloudflare infrastructure as part of relay server operation. We do not access these logs.

### Data shared with other users in session
- Encrypted audio (AES-256-GCM ciphertext, decrypted only by session peers)
- Display name and emoji avatar (wire frame header, unencrypted)
- Device sender ID (wire frame header, unencrypted)

### Security practices
- Data encrypted in transit: Yes (AES-256-GCM end-to-end)
- Data deletion: No user data stored. Clear app data to remove local keys and profile.
- Independent security review: No

---

## Screenshots Descriptions

1. **Main Screen** -- PTT button, channel selector, connection status, encryption badge
2. **QR Auth** -- Generate or scan QR to join encrypted channel
3. **Users** -- Active peers with mute/favorite controls
4. **Transcription** -- Live speech-to-text feed with replay buttons
5. **About** -- Connection status, permissions, privacy policy link

## Feature Graphic Text
"ENCRYPTED PUSH-TO-TALK" -- Your voice, your keys, no servers listening.
