# SassyTalkie — FIPS 140-3 / CJIS Security Policy Posture

**Status:** design / gap analysis (2027–2028 planning horizon)
**Scope:** the cryptography actually shipped in `sassytalkie-core` and the
Cloudflare relay, mapped against FIPS 140-3, FIPS 203, the SP 800-56/-90/-108
series, and the CJIS Security Policy.
**Audience:** engineering + anyone fielding a "is this CJIS-compliant?" question
from a public-safety / agency customer.

> **Bottom line up front.** SassyTalkie uses **FIPS-*approved algorithms*
> everywhere it matters** — AES-256-GCM, X25519, ML-KEM-768, HKDF-SHA256,
> HMAC-SHA256. It does **not** today use a **FIPS 140-3 *validated cryptographic
> module***. The algorithms are right; the *implementations* (RustCrypto, dalek,
> the `ml-kem` crate) carry no CMVP certificate. That single distinction —
> approved algorithm vs. validated module — is the central compliance gap, and
> CJIS §5.10.1.2 requires the **validated module**, not merely the approved
> algorithm. Everything below elaborates that gap and the paths to close it.

---

## 1. Cryptographic inventory

Every primitive the app relies on, where it lives, and its standards status.
"Approved" = on the FIPS-approved algorithm list. "Validated module" =
the *specific implementation* holds a CMVP certificate. These are independent
axes; SassyTalkie is **approved-algorithm, non-validated-module** across the board.

| # | Primitive | Where in code | Crate / impl | Standard | Algorithm status | Module status |
|---|-----------|---------------|--------------|----------|------------------|---------------|
| 1 | **AES-256-GCM** (AEAD for all audio) | `core/src/crypto.rs` — `CryptoSession::{encrypt,decrypt}`, `Aes256Gcm` | `aes-gcm` (RustCrypto) | FIPS 197 + SP 800-38D | **Approved** | **Not validated** (no CMVP cert) |
| 2 | **X25519 ECDH** (classical key agreement) | `core/src/crypto.rs` — `KeyExchange`, `EphemeralSecret::diffie_hellman`; also `core/src/pqc.rs` raw DH | `x25519-dalek` | SP 800-186 (Curve25519) / RFC 7748 | **Approved** (Curve25519 added to 800-186) | **Not validated** |
| 3 | **ML-KEM-768** (post-quantum KEM, hybrid half) | `core/src/pqc.rs` — `HybridKeyExchange`, `respond()`, `MlKem768::{generate_keypair,encapsulate,decapsulate}` | `ml-kem` (RustCrypto) | **FIPS 203** (ML-KEM, NIST Category 3) | **Approved** | **Not validated** |
| 4 | **HKDF-SHA256** (key derivation) | `core/src/crypto.rs` `derive_aes_key`; `core/src/pqc.rs` `combine_secrets`; `core/src/sealed.rs` `expand` | `hkdf` + `sha2` (RustCrypto) | SP 800-56C Rev. 2 (extract-then-expand) / RFC 5869 | **Approved** (as 800-56C KDF) | **Not validated** |
| 5 | **HMAC-SHA256** (relay capability tokens; also underlies HKDF) | `cloudflare-worker/src/relay-auth.js` `hmacSha256Hex` via WebCrypto `crypto.subtle`; inside HKDF via `hkdf` crate | WebCrypto (worker) / RustCrypto `hmac` (core) | FIPS 198-1 + FIPS 180-4 | **Approved** | **Not validated** (see §1.1 on WebCrypto) |
| 6 | **SHA-256** (hash under HKDF/HMAC) | transitively via #4/#5 | `sha2` / WebCrypto | FIPS 180-4 | **Approved** | **Not validated** |
| 7 | **CSPRNG — `OsRng`** (nonces, X25519 ephemerals, PSK) | `core/src/crypto.rs` `random_nonce_prefix`, `generate_psk`, `EphemeralSecret::random_from_rng(OsRng)`; `core/src/pqc.rs` ephemeral gen | `aes_gcm::aead::OsRng` → `getrandom` → OS RNG | SP 800-90A/B/C (the *OS DRBG* behind it) | DRBG construction **approved**; the OS entropy source varies | **Depends on platform** (see §1.2) |
| 8 | **ML-KEM internal RNG** (keygen/encaps randomness) | `core/src/pqc.rs` — `getrandom` feature path noted in code comments | `getrandom` → OS RNG | SP 800-90A/B/C | as #7 | as #7 |

### 1.1 The relay's HMAC is WebCrypto, not RustCrypto

`relay-auth.js` mints/verifies capability tokens with the Cloudflare Workers
WebCrypto `crypto.subtle` HMAC-SHA256 (`token = "<expSec>.<hexSig>"`,
`hexSig = HMAC-SHA256(`${roomId}.${expSec}`, AUTH_SECRET)`). The token is a
**transport-authorization** secret, not a confidentiality key for user audio —
audio is E2E-encrypted before it reaches the relay (#1). So even though the
Workers runtime's crypto is not a CMVP-validated module either, this primitive
is **out of the CJIS "protects CJI in transit/at rest" boundary**: it guards
*who may open a relay room*, not the data. That is worth stating explicitly in
any compliance package, because it shrinks the module-validation surface that
actually has to be validated to the **client-side AEAD + KEX** (#1–#4, #7).

### 1.2 The RNG is the most platform-dependent line item

`OsRng`/`getrandom` delegate to the platform CSPRNG:
- **Android:** `getrandom(2)` → kernel CSPRNG. Whether that counts as an
  SP 800-90 DRBG with a validated noise source depends on the SoC/Android build.
- **Desktop (Tauri):** `BCryptGenRandom` (Windows), `getrandom` (Linux),
  `getentropy` (macOS).
- **iOS:** `SecRandomCopyBytes` / `getentropy`.

For a validated posture the DRBG must be either a CMVP-validated platform module
or one we instantiate inside our own validated boundary (see §2). Today we
inherit the platform's, which is good engineering but **not a documented
validated source**.

### 1.3 What is *not* used (so reviewers don't have to look)

- No home-grown cipher, no ECB, no static IVs. Nonces are
  `8-byte random prefix ‖ 4-byte LE counter` with counter-exhaustion handled by
  refusing to encrypt (`next_nonce` returns `Err`, forcing re-key) — exactly the
  SP 800-38D nonce-uniqueness requirement.
- No MD5/SHA-1 anywhere in the crypto path.
- No XOR-combiner for the hybrid KEX — `pqc.rs` concatenates the two shared
  secrets and runs them through HKDF (a sound dual-PRF combiner), which is the
  construction a reviewer wants to see.
- Replay protection is post-tag-validation (`crypto.rs` `decrypt` inserts the
  nonce into `seen_nonces` only after the GCM tag verifies), so forged frames
  cannot poison the window — relevant to CJIS integrity expectations.

---

## 2. The validated-module gap and remediation paths

### 2.1 Restating the gap precisely

FIPS 140-3 validates a **cryptographic module** — a specific build of specific
code, tested by an accredited lab, issued a CMVP certificate with a boundary,
a security policy, self-tests (POST/CASTs), and approved-mode enforcement.
RustCrypto, `x25519-dalek`, and `ml-kem` are **excellent, audited, pure-Rust
implementations of approved algorithms** — but they are **not** submitted
modules and have **no certificate number**. A CJIS auditor asking "what is the
CMVP cert # for the module encrypting CJI?" has, today, **no answer**. That is
the gap. It is a *packaging/validation* gap, not an *algorithm* gap.

### 2.2 Remediation options (ranked, with trade-offs)

**Option A — Swap the symmetric/KEX backend to a FIPS-validated native module
(BoringSSL FIPS / AWS-LC-FIPS).**
Replace the `aes-gcm`, `x25519-dalek`, `hkdf`, `hmac`, and (where available) the
KEM with calls into AWS-LC-FIPS or BoringSSL's FIPS module (via `aws-lc-rs` or
an FFI shim). These modules carry active CMVP certificates and run in an
enforced FIPS mode with power-on self-tests.
- *Pros:* real certificate; the AEAD + X25519 + HKDF + HMAC + DRBG all land
  inside a validated boundary in one move; well-trodden path.
- *Cons:* **ML-KEM-768 may not yet be inside the validated boundary** of every
  such module on our timeline — FIPS 203 module validations are still maturing
  through 2027–2028. If the FIPS module lacks validated ML-KEM, we either (a)
  run hybrid with the classical half validated and PQ half "approved but
  outside the boundary," or (b) drop to classical-only in FIPS mode. The
  `KexSuite::negotiate` design (`pqc.rs`) already supports a clean
  classical fallback, so this is configurable rather than a rewrite. Also adds
  a C/native dependency and cross-compilation burden to the Android/iOS builds
  that today are pure-Rust.

**Option B — Platform Keystore / StrongBox / Secure Enclave.**
Perform the AEAD and (where the API allows) the key agreement inside
Android Keystore (StrongBox-backed where present) or iOS Secure Enclave, both of
which ride on **CMVP-validated platform crypto modules** on many devices.
- *Pros:* keys can be hardware-bound and non-exportable; strong story for "key
  at rest" and device-binding; leverages a cert the OEM already holds.
- *Cons:* the Keystore AEAD/KDF surface is narrower than what `crypto.rs` needs
  (e.g. our per-frame nonce discipline, the hybrid KEX combiner, the sealed-sender
  HKDF in `sealed.rs`) — not everything maps cleanly to Keystore primitives;
  coverage/validation **varies by device**, so "validated" becomes a per-SKU
  claim, not a universal one; no help on the desktop/relay side.

**Option C — Get a validated module's userspace DRBG + keep RustCrypto for
the algorithms not yet in any FIPS boundary (hybrid PQC).**
Use AWS-LC-FIPS for the DRBG, AES-GCM, X25519, HKDF, HMAC (Option A's covered
set) and keep the `ml-kem` crate for the PQ half *explicitly documented as
defense-in-depth outside the validated boundary*. Because `pqc.rs` combines
secrets so the session key is safe if **either** half holds, the *validated*
classical half alone meets the in-boundary confidentiality bar, and ML-KEM is a
**bonus** layer — a defensible position to write down.
- *Pros:* honest, shippable now, doesn't block on FIPS 203 module availability,
  keeps the PQ protection.
- *Cons:* requires careful security-policy wording so an auditor agrees the
  in-boundary half is what's "protecting CJI"; the PQ half being outside the
  boundary must be stated, not hidden.

**Option D — Submit our own module.**
Carve `sassytalkie-core`'s crypto into a defined boundary and put *it* through
CMVP.
- *Pros:* a cert that exactly matches our code, including hybrid ML-KEM.
- *Cons:* cost and calendar measured in **quarters-to-years**; ongoing
  re-validation burden on every crypto change. Almost certainly not worth it
  versus Option A/C for a product this size.

### 2.3 Recommendation

**Option A as the spine, Option C's framing for the PQ half, Option B where the
device offers it.** Concretely: move #1–#4, #7 into AWS-LC-FIPS for a real
certificate; document ML-KEM-768 as approved-algorithm defense-in-depth layered
*on top of* the validated classical KEX (which the `pqc.rs` combiner makes
trivially true); use StrongBox/Secure Enclave for at-rest key binding where
present. Gate all of this behind a build-time/runtime "FIPS mode" that forces
`KexSuite::Classical`-in-boundary + PQ-on-top and disables any non-approved path.

---

## 3. CJIS Security Policy mapping

CJIS controls referenced are from the CJIS Security Policy v5.9-era areas;
section numbers are indicative and should be reconfirmed against the customer's
contracted policy version.

| CJIS area | SassyTalkie mechanism | Where | Status |
|-----------|----------------------|-------|--------|
| **§5.10.1.2 Encryption (in transit)** — FIPS-validated, ≥128-bit | E2E **AES-256-GCM** on every audio frame; relay is a blind forwarder that only sees ciphertext | `core/src/crypto.rs`; relay `cloudflare-worker/src/ptt-relay.js` ("relay never sees plaintext") | **Algorithm meets/exceeds; module validation is the gap (§2)** |
| **§5.10.1.2 Encryption (at rest)** | Keys are ephemeral per session; replay history (`audio_cache.rs`) is RAM-only PCM. At-rest key material → Keystore/StrongBox (Option B) | `core/src/audio_cache.rs`; planned Keystore binding | **Partial — formalize via Option B** |
| **§5.6 Identification & Authentication / Advanced Authentication (MFA)** | Room access gated by HMAC-SHA256 **capability tokens** (`relay-auth.js`); device identity via stable peer id. App-level MFA (the *user* auth factor) is **out of current scope** | `cloudflare-worker/src/relay-auth.js` | **Transport auth: yes. Advanced Authentication for the operator: NOT provided — must be supplied by the deploying agency (MDM/IdP)** |
| **§5.10 / data sovereignty** | **Self-hostable relay** — agency runs its own Cloudflare Worker + DO + KV + AUTH_SECRET + Firebase; no Sassy infrastructure in the path. Federation via bridge peers keeps each agency's metadata in its own account | `cloudflare-worker/SELF_HOST.md` | **Strong — this is a differentiator for CJIS/FedRAMP/NDAA** |
| **§5.4 Auditing & Accountability** | Relay `[observability.logs]` capture connection events (room open, join/leave, rate-limit trips) — **never audio content**; retention tunable to policy | `SELF_HOST.md` "Operational notes"; `ptt-relay.js` | **Connection-level audit present; CJI-access audit must be added at the agency layer** |
| **§5.5 Access Control** | `MAX_PEERS_PER_ROOM=16`, per-socket 120 msg/s rate limit, fail-closed token check (`verifyCapabilityToken` returns error if `AUTH_SECRET` unset) | `ptt-relay.js`, `relay-auth.js` | **Present** |
| **§5.10.1.5 / key management** | HKDF-SHA256 (SP 800-56C) derivation; per-session ephemeral X25519 + ML-KEM keypairs; nonce-uniqueness enforced with re-key-on-exhaustion; `zeroize` on all secret material | `crypto.rs`, `pqc.rs`, `sealed.rs` | **Sound design; needs validated-module DRBG/KDF (§2) to be a CJIS-grade claim** |
| **Metadata minimization (beyond CJIS letter, supports §5.10 spirit)** | **Sealed sender** — per-epoch HKDF-blinded room/peer handles; relay sees only opaque rotating tokens, cannot correlate who-talks-to-whom across 15-min epochs | `core/src/sealed.rs` | **Implemented; defense-in-depth on top of E2E** |

### 3.1 The one sentence that matters for CJIS §5.10.1.2

CJIS requires that the **cryptographic module** protecting CJI be **FIPS
140-validated**. SassyTalkie's *algorithm* selection already satisfies (indeed
exceeds, via AES-256 and PQ-hybrid) the CJIS strength bar — **but until the
client's AEAD+KEX runs inside a CMVP-validated module (§2), a strict CJIS
auditor can reject the encryption control on module-validation grounds even
though the math is correct.** Self-hosting (`SELF_HOST.md`) cleanly satisfies the
*data-sovereignty* expectations independently of that, and the sealed-sender
layer (`sealed.rs`) exceeds the metadata-handling expectations.

---

## 4. Roadmap to a defensible compliance claim

Ordered. Each step is independently shippable and moves the needle.

1. **Write the algorithm-vs-module distinction into the customer-facing
   security statement (do now, zero code).** State plainly: approved algorithms
   everywhere; module validation in progress; relay self-hostable for
   sovereignty. Honesty here prevents an over-claim that an auditor later
   torpedoes.
2. **Introduce a build-time/runtime `FIPS_MODE` flag.** When set: force the
   approved-only paths, disable any non-approved fallback, and (eventually)
   route #1–#4/#7 through the validated backend. No behavior change yet — just
   the switch and the plumbing.
3. **Integrate AWS-LC-FIPS (Option A) for AES-256-GCM, X25519, HKDF, HMAC, and
   the DRBG** on at least one target (desktop first — easiest native linking),
   behind `FIPS_MODE`. Capture the CMVP cert # in the compliance package.
4. **Document the PQC half (Option C framing).** Security policy text:
   "Confidentiality of CJI is provided by the in-boundary validated classical
   AEAD/KEX; ML-KEM-768 (FIPS 203 approved algorithm) is layered on top via the
   `pqc.rs` HKDF combiner as harvest-now-decrypt-later defense-in-depth and is
   not relied upon for the in-boundary claim." This lets us keep PQ protection
   without waiting on FIPS 203 *module* availability.
5. **Bind at-rest keys to Keystore/StrongBox/Secure Enclave (Option B)** on
   mobile; document per-SKU validation coverage.
6. **Extend the audit story to CJI-access events at the agency layer** (the
   relay logs connection metadata, not access-to-CJI — the latter belongs to
   the deploying agency's IdP/MDM). Provide a reference logging schema.
7. **Document Advanced Authentication as an integration responsibility.** The
   app provides transport capability tokens; the agency supplies operator MFA
   via their IdP/MDM. State the boundary explicitly.
8. **Bring the validated backend to Android/iOS** (the harder native-linking
   targets) once the desktop integration is proven.

### 4.1 Honest limits

- **Without a validated module, a strict CJIS encryption claim is not
  achievable** — no amount of algorithm strength substitutes for the CMVP
  certificate. Steps 2–3 are the only things that close that, and they require
  real native-crypto integration work.
- **ML-KEM-768 module validation is on someone else's calendar.** We can ship
  it as approved-algorithm defense-in-depth (step 4) immediately, but an
  *in-boundary validated PQ* claim depends on FIPS 203 module availability we
  don't control.
- **"FIPS-validated" is per-build and per-platform.** A green check on desktop
  says nothing about the Android build until step 8 lands. Compliance language
  must scope the claim to the validated targets.
- **Self-hosting and sealed-sender are already strong and shippable today** —
  lead with those for sovereignty/metadata while the module work proceeds.
