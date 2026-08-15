<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-->
# FCM cold wake and OEM limits

API 34+ forbids starting a microphone foreground service from an FCM
receiver. Cold wake posts a **high-priority, user-visible notification**;
the operator tap opens the app, which then starts the radio FGS legally.
A warm `WalkieService` may receive `ACTION_WAKE` without promoting FGS+mic.

The FCM handler fails closed (logs, no crash) if a killed/background
process hits an unexpected error.

## Remaining OEM limits (not solvable in-app)

- Xiaomi/HyperOS, Huawei, Oppo/ColorOS, OnePlus, Samsung: FCM data-only may
  be delayed or dropped unless autostart / battery-unrestricted is granted.
- Force-stop from Recents on some OEMs prevents FCM until the user opens
  the app again.
- Notification permission denied on API 33+ means the cold-wake bootstrap
  cannot surface; the user must open the app.
- Background activity starts from FCM are blocked on API 29+; this policy
  never calls `startActivity` from the receiver.

In-app: Settings → **Background wake (OEM)** opens the OEM autostart /
battery screen when a deep-link exists, otherwise app details.
