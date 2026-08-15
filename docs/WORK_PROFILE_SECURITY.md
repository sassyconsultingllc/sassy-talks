<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-->
# Work profile, MDM, and managed configuration

SassyTalkie is intended for **agency MDM/EMM deploy**, including work profiles.
The app **does not block** work profiles, device-owner, or profile-owner installs.

Older drafts of this document described blocking non-primary profiles. That
would break agency rollout. Current code **honors RestrictionsManager** and
logs profile kind; it does not exit.

## What the OS actually provides

| Mechanism | What it does | What this app does |
|-----------|----------------|--------------------|
| `RestrictionsManager` / `app_restrictions.xml` | EMM pushes app-specific keys | Honored at runtime (not copied into user prefs) |
| Work profile (`UserManager.isManagedProfile`) | Separate managed user | Detected and logged; **not blocked** |
| Device / profile owner (`DevicePolicyManager`) | DPC can wipe the profile/device | Detected; silent full wipe is the **DPC**, not this APK |
| In-app `force_session_wipe` | Next launch clears session keys | Implemented; requires EMM to set the restriction |

Foreground-service notifications cannot be fully suppressed by MDM. Extra
lock-screen PTT actions can be disabled via `enable_notifications` and
`lock_screen_ptt`.

## Restriction keys (`res/xml/app_restrictions.xml`)

| Key | Type | Effect |
|-----|------|--------|
| `lock_screen_ptt` | bool | Extra notification PTT action (also needs `enable_notifications`) |
| `enable_wifi_multicast` | bool | Local Wi-Fi transport |
| `enable_cloudflare_relay` | bool | Cloudflare relay |
| `require_relay` | bool | Forces relay on; disables Wi-Fi and Bluetooth transports |
| `enable_bluetooth` | bool | Bluetooth transport |
| `enable_notifications` | bool | Extra notification actions (not the FGS required notification) |
| `enable_translation` | bool | Live translation |
| `enable_diagnostics_overlay` | bool | Diagnostics HUD |
| `max_tx_seconds` | int | Max PTT duration (10–300) |
| `session_idle_timeout_minutes` | int | 0 = off; otherwise wipe keys after idle |
| `enrollment_token` | string | Required to join when set. **Room ID is not authorization** |
| `operator_role` | string | `operator` or `supervisor` (local role only; no cloud IdP) |
| `force_session_wipe` | bool | Wipe session keys on next process start |
| `require_fips_provider` | bool | Fail-closed TX if no FIPS-capable provider is present (none ships in this APK) |
| `require_tls_pinning` | bool | TLS SPKI pinning for the relay (default **true** when backup pins are present; false = platform TLS) |

Removing a restriction restores the operator’s prior preference. Managed values
are not copied into `SharedPreferences`.

## Identity

- Device install-id is Keystore-wrapped.
- Join requires a 32-byte room secret (PSK). Optional MDM enrollment token.
- Roles are local (`operator` / `supervisor`). There is no cloud identity provider in this repo.
- In-app “Clear session keys” wipes this app’s keys. **Silent enterprise wipe of the device/profile requires a DPC.**

## Code

- `ManagedConfig.kt` — restriction overlay
- `ProfileChecker.kt` — `shouldBlock` is always false
- `EnrollmentProof.kt` — room id is not auth
- `SessionWipe.kt` — in-app + managed wipe hook
