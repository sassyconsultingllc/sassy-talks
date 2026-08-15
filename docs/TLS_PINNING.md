<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-->
# TLS pinning (Cloudflare relay)

SassyTalkie pins **Google Trust Services intermediates**, not a Cloudflare
leaf. Leaf certs rotate about every 90 days; a single leaf pin would brick
clients.

## Pin-set (captured 2026-08-14)

Host: `relay.sassyconsultingllc.com`

| Cert | Role | SPKI SHA-256 (base64) | Not after (UTC) |
|------|------|------------------------|-----------------|
| GTS WE1 | ECDSA intermediate (current) | `kIdp6NNEd8wsugYyyIYFsi1ylMCED3hZbSR8ZFsa/A4=` | 2029-02-20 |
| GTS WE2 | ECDSA backup | `vh78KSg1Ry4NaqGDV10w/cTb9VH3BQUZoCWNa93W/EY=` | 2029-02-20 |
| GTS WR1 | RSA backup | `yDu9og255NN5GEf+Bwa9rTrqFQ0EydZ0r1FCh9TdAW4=` | 2029-02-20 |
| GTS WR2 | RSA backup | `YPtHaftLw6/0vnc2BnNKGF54xiCA28WFcccjkA4ypCM=` | 2029-02-20 |

Source of truth: `core/src/tls_pins.rs` (must match `RelayTlsPins.kt`).

Production **defaults on** because this set has backups (`pins_complete()`).
Mismatch is **fail-closed**.

## Disable

- Android MDM: `require_tls_pinning=false` (platform TLS)
- Desktop: `SASSYTALKIE_TLS_PINNING=0`

## Rotation

1. Capture the new chain: `echo | openssl s_client -connect relay.sassyconsultingllc.com:443 -servername relay.sassyconsultingllc.com 2>/dev/null | openssl crl2pkcs7 -nocrl -certfile /dev/stdin | openssl pkcs7 -print_certs -noout` (or browse the live chain).
2. Compute SPKI SHA-256 for each **intermediate** (not the leaf): `openssl x509 -in inter.pem -noout -pubkey | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary | openssl base64`.
3. Add the new pin **alongside** the existing backups. Do not ship a single pin.
4. Release. After Google's overlap window, drop retired intermediates.
5. If a pin-set would be empty or a single leaf, leave pinning **off** (`pins_complete() == false`).
