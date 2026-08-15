<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-->
# SassyTalkie — FIPS 140 / CJIS control matrix

**This product is not FIPS 140 validated and is not CJIS certified.**
There is **no CMVP-validated cryptographic module in this repository**. Algorithms
used (AES-256-GCM, SHA-256, HMAC-SHA256, HKDF-SHA256, X25519, ML-KEM-768) are
FIPS-*approved algorithms* in many cases; that is **not** a validated-module claim.

Status values:

| Status | Meaning |
|--------|---------|
| **Implemented** | Present in shipped code in this repo |
| **Partial** | Present with documented gaps |
| **Not implemented** | Not in this repo |
| **Requires FIPS module or agency policy** | Cannot be closed in application code alone |

## Control matrix

| Control | Status | Where / notes |
|---------|--------|----------------|
| AES-256-GCM for audio | Implemented | `core/src/crypto.rs`. RustCrypto — **not** a CMVP module |
| Authenticated control envelope (opcode 0x18) | Implemented | `core/src/control_auth.rs`, Android/iOS/desktop. AAD binds version, opcode, room, sender, epoch, seq. Replay window. Fail-closed for raw 0x10..0x1F privileged ops |
| Hybrid/PQC rekey only after CONFIRM 0x1F + CONFIRM_ACK 0x20 | Implemented | `core/src/hybrid_rekey.rs` four-way. Auto-PQC remains **off**. Initiator installs only after ACK; responder TX stays on the old key until peer new-key RX. Lost CONFIRM/ACK cannot permanently split keys |
| Fail-closed TX without session keys | Implemented | Android `SassyTalkNative.pttStart`, native TX refuse, desktop/iOS transport |
| Android Keystore / StrongBox when available | Partial | Install-id wrap, EncryptedSharedPreferences, audit HMAC. Audio AEAD is still RustCrypto, not Keystore AEAD |
| iOS Keychain for PSK | Implemented | `KeychainStore.swift` (`AfterFirstUnlockThisDeviceOnly`) |
| Desktop secret store | Implemented | Windows Credential Manager, macOS Keychain, Linux libsecret/secret-service first; AES file fallback (`secret_store.rs` / `os_vault.rs`). iOS Keychain via `KeychainStore.swift` |
| TLS to relay | Implemented (pin-set) | Production default **on** for `relay.sassyconsultingllc.com` using GTS WE1/WE2/WR1/WR2 **intermediate** SPKI pins (backups present). MDM `require_tls_pinning=false` or `SASSYTALKIE_TLS_PINNING=0` uses platform TLS. Mismatch fail-closed. **Not** a leaf pin. Rotation: `docs/TLS_PINNING.md` |
| Optional FIPS provider selection | Partial | MDM `require_fips_provider` fail-closes TX if Conscrypt is absent. Conscrypt is **not** shipped and is **not** a FIPS 140 certificate |
| Production diagnostic plaintext prefixes | Implemented | `setCryptoTrace` is a no-op unless `BuildConfig.DEBUG` |
| No plaintext PSK in logs | Implemented | Audit redaction; session prefs EncryptedSharedPreferences; crypto-trace debug-only |
| Security-event technical audit | Implemented | Hash-chained local store. **Not** a legal chain of custody |
| Session idle timeout / lock | Implemented | MDM `session_idle_timeout_minutes` wipes keys in-app. Screen lock is OS |
| Remote wipe of whole device | Requires FIPS module or agency policy | In-app/MDM `force_session_wipe` clears **app session keys**. Silent enterprise wipe requires a DPC/EMM |
| FIPS 140-3 validated module | Requires FIPS module or agency policy | Needs AWS-LC-FIPS / OEM Keystore cert per SKU / lab submission. **Not in this repo** |
| CJIS Security Policy certification / ATO | Requires FIPS module or agency policy | Agency Authority to Operate, MFA/IdP, facility, personnel. **Not claimed** |
| Operator MFA / Advanced Authentication | Requires FIPS module or agency policy | Transport uses room-secret + optional enrollment token. User MFA is the agency IdP/MDM |

## Honest limits

- **No FIPS 140 validated module in this repo.** A CJIS auditor asking for a CMVP certificate number has no answer today.
- **Not CJIS certified.** Self-hostable relay and E2E audio are engineering controls, not a certification.
- **No “Certified” badges in the UI.**
- FIPS 203 ML-KEM is an approved *algorithm* here; the implementation is not a validated module.
- `require_fips_provider` is a fail-closed policy hook, not a certified mode.

## Remediation (outside this repo)

1. Integrate a CMVP-validated module (AWS-LC-FIPS or equivalent) behind a real FIPS mode.
2. Agency ATO against the contracted CJIS Security Policy version.
3. IdP/MDM for operator MFA; DPC for remote wipe and restriction push.
