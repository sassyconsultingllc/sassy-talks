// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import okhttp3.CertificatePinner
import okhttp3.OkHttpClient

/**
 * SPKI pins for `relay.sassyconsultingllc.com`.
 *
 * Cloudflare rotates the leaf about every 90 days and chooses the Universal
 * SSL CA itself (Let's Encrypt, Google Trust Services or SSL.com), so the set
 * pins the root keys of all three CAs plus the GTS WE1/WE2/WR1/WR2
 * intermediates. OkHttp matches against the cleaned chain, which ends in the
 * trust anchor, so a root pin holds whichever intermediate a renewal uses.
 * Production default is on because [pinsComplete] is true. MDM
 * `require_tls_pinning=false` or [processEnabled] false disables (platform TLS).
 * Mismatch is fail-closed when enabled. Sources and rotation: `docs/TLS_PINNING.md`.
 *
 * Must stay in lockstep with `core/src/tls_pins.rs` (core's tests compare them).
 */
object RelayTlsPins {
    const val HOST = "relay.sassyconsultingllc.com"

    val SPKI_SHA256_B64 = listOf(
        "kIdp6NNEd8wsugYyyIYFsi1ylMCED3hZbSR8ZFsa/A4=", // GTS WE1
        "vh78KSg1Ry4NaqGDV10w/cTb9VH3BQUZoCWNa93W/EY=", // GTS WE2
        "yDu9og255NN5GEf+Bwa9rTrqFQ0EydZ0r1FCh9TdAW4=", // GTS WR1
        "YPtHaftLw6/0vnc2BnNKGF54xiCA28WFcccjkA4ypCM=", // GTS WR2
        "hxqRlPTu1bMS/0DITB1SSu0vd4u/8l8TjPgfaAp63Gc=", // GTS Root R1
        "Vfd95BwDeSQo+NUYxVEEIlvkOlWY2SalKK1lPhzOx78=", // GTS Root R2
        "QXnt2YHvdHR3tJYmQIr0Paosp6t/nggsEGD4QJZ3Q0g=", // GTS Root R3
        "mEflZT5enoR1FuXLgYYGqnVEoZvmf9c2bVBpiOjYQ0c=", // GTS Root R4
        "C5+lpZ7tcVwmwQIMcRtPbsQtWLABXhQzejna0wHFr8M=", // ISRG Root X1
        "diGVwiVYbubAI3RW4hB9xU8e/CH2GnkuvVFZE8zmgzI=", // ISRG Root X2
        "sCkq5UWXjg+7mKu9lMhhYF5bGLsy7VI/UNW3tccdR7w=", // ISRG Root YE
        "fk6IOKit1ild5647BH06ujSIq5XbCgqlbYl6ANhhi88=", // ISRG Root YR
        "K89VOmb1cJAN3TK6bf4ezAbJGC1mLcG2Dh97dnwr3VQ=", // SSL.com TLS RSA Root CA 2022
        "G/ANXI8TwJTdF+AFBM8IiIUPEv0Gf6H5LA/b9guG4yE=", // SSL.com TLS ECC Root CA 2022
    )

    fun pinsComplete(): Boolean = SPKI_SHA256_B64.size >= 2

    /** Production default: pin when backups are present. */
    val productionDefaultEnabled: Boolean get() = pinsComplete()

    @Volatile
    var processEnabled: Boolean = productionDefaultEnabled

    fun pinMatch(presentedB64: Collection<String>, configuredB64: Collection<String> = SPKI_SHA256_B64): Boolean {
        if (presentedB64.isEmpty() || configuredB64.isEmpty()) return false
        return presentedB64.any { it in configuredB64 }
    }

    fun pinner(pins: Collection<String> = SPKI_SHA256_B64): CertificatePinner {
        val builder = CertificatePinner.Builder()
        for (pin in pins) {
            builder.add(HOST, "sha256/$pin")
        }
        return builder.build()
    }

    fun apply(builder: OkHttpClient.Builder, enabled: Boolean = processEnabled): OkHttpClient.Builder {
        if (enabled && pinsComplete()) builder.certificatePinner(pinner())
        return builder
    }
}
