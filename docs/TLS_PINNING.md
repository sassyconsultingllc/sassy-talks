<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
-->
# TLS pinning (Cloudflare relay)

SassyTalkie pins the **root keys of every CA Cloudflare issues Universal SSL
from**, plus the Google Trust Services intermediates of the current chain. It
never pins a leaf: leaf certs rotate about every 90 days.

Cloudflare chooses the Universal SSL CA itself: Let's Encrypt, Google Trust
Services or SSL.com ([certificate authorities](https://developers.cloudflare.com/ssl/reference/certificate-authorities/)).
A renewal can come from any of them. Until 2026-10-22 only the GTS
intermediates were pinned, so a renewal from another CA would have cut off
every client at once. Root keys outlive intermediate rotations by a decade,
and a root key matches whether the chain ends in the self-signed root or a
cross-signed copy (same key). Every platform matches pins against the
verified chain including its trust anchor (Android: OkHttp's cleaned chain;
iOS: the evaluated SecTrust; desktop: the webpki verified path).

## Pin-set (2026-10-22)

Host: `relay.sassyconsultingllc.com`. Live chain on 2026-10-22: leaf, GTS WE1,
GTS Root R4 (served cross-signed by GlobalSign Root CA).

| Cert | Role | Key | SPKI SHA-256 (base64) | Not after (UTC) |
|------|------|-----|------------------------|-----------------|
| GTS WE1 | ECDSA intermediate (current) | P-256 | `kIdp6NNEd8wsugYyyIYFsi1ylMCED3hZbSR8ZFsa/A4=` | 2029-02-20 |
| GTS WE2 | ECDSA backup | P-256 | `vh78KSg1Ry4NaqGDV10w/cTb9VH3BQUZoCWNa93W/EY=` | 2029-02-20 |
| GTS WR1 | RSA backup | RSA-2048 | `yDu9og255NN5GEf+Bwa9rTrqFQ0EydZ0r1FCh9TdAW4=` | 2029-02-20 |
| GTS WR2 | RSA backup | RSA-2048 | `YPtHaftLw6/0vnc2BnNKGF54xiCA28WFcccjkA4ypCM=` | 2029-02-20 |
| GTS Root R1 | root | RSA-4096 | `hxqRlPTu1bMS/0DITB1SSu0vd4u/8l8TjPgfaAp63Gc=` | 2036-06-22 |
| GTS Root R2 | root | RSA-4096 | `Vfd95BwDeSQo+NUYxVEEIlvkOlWY2SalKK1lPhzOx78=` | 2036-06-22 |
| GTS Root R3 | root | P-384 | `QXnt2YHvdHR3tJYmQIr0Paosp6t/nggsEGD4QJZ3Q0g=` | 2036-06-22 |
| GTS Root R4 | root (current anchor) | P-384 | `mEflZT5enoR1FuXLgYYGqnVEoZvmf9c2bVBpiOjYQ0c=` | 2036-06-22 |
| ISRG Root X1 | Let's Encrypt root | RSA-4096 | `C5+lpZ7tcVwmwQIMcRtPbsQtWLABXhQzejna0wHFr8M=` | 2035-06-04 |
| ISRG Root X2 | Let's Encrypt root | P-384 | `diGVwiVYbubAI3RW4hB9xU8e/CH2GnkuvVFZE8zmgzI=` | 2040-09-17 |
| ISRG Root YE | Let's Encrypt Gen Y root | P-384 | `sCkq5UWXjg+7mKu9lMhhYF5bGLsy7VI/UNW3tccdR7w=` | 2045-09-02 |
| ISRG Root YR | Let's Encrypt Gen Y root | RSA-4096 | `fk6IOKit1ild5647BH06ujSIq5XbCgqlbYl6ANhhi88=` | 2045-09-02 |
| SSL.com TLS RSA Root CA 2022 | root | RSA-4096 | `K89VOmb1cJAN3TK6bf4ezAbJGC1mLcG2Dh97dnwr3VQ=` | 2046-08-19 |
| SSL.com TLS ECC Root CA 2022 | root | P-384 | `G/ANXI8TwJTdF+AFBM8IiIUPEv0Gf6H5LA/b9guG4yE=` | 2046-08-19 |

Sources. Each root was downloaded from its CA and agreed with the Mozilla copy
in Debian `ca-certificates` where one exists (R2, YE and YR are not in trust
stores yet):
`https://pki.goog/repo/certs/gtsr{1,2,3,4}.pem`,
`https://letsencrypt.org/certs/isrgrootx1.pem`, `.../isrg-root-x2.pem`,
`.../gen-y/root-ye.pem`, `.../gen-y/root-yr.pem` (the cross-signed copies
`root-x2-by-x1`, `root-ye-by-x2`, `root-yr-by-x1` carry the same keys),
`https://ssl.com/repo/certs/SSLcom-TLS-Root-2022-{RSA,ECC}.pem`.

Source of truth: `core/src/tls_pins.rs`. A core test fails if
`RelayTlsPins.kt` differs. iOS reads the set from core over FFI.

Production **defaults on** because this set has backups (`pins_complete()`).
Mismatch is **fail-closed**.

## Disable

- Android MDM: `require_tls_pinning=false` (platform TLS)
- Desktop: `SASSYTALKIE_TLS_PINNING=0`

## Rotation

1. Capture the new chain: `echo | openssl s_client -connect relay.sassyconsultingllc.com:443 -servername relay.sassyconsultingllc.com 2>/dev/null | openssl crl2pkcs7 -nocrl -certfile /dev/stdin | openssl pkcs7 -print_certs -noout` (or browse the live chain).
2. Compute SPKI SHA-256 for each **intermediate or root** (never the leaf): `openssl x509 -in inter.pem -noout -pubkey | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary | openssl base64`.
3. Add the new pin **alongside** the existing backups. Do not ship a single pin.
   If Cloudflare adds a CA, add its root keys from the CA's own repository.
   iOS rebuilds SPKI per key type (`RelayPinning.swift` `wrapSPKI`: P-256,
   P-384, RSA-2048, RSA-4096); a new key type needs a header there too.
4. Release. After Google's overlap window, drop retired intermediates.
5. If a pin-set would be empty or a single leaf, leave pinning **off** (`pins_complete() == false`).
