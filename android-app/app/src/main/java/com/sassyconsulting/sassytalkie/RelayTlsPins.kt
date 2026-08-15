// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import okhttp3.CertificatePinner
import okhttp3.OkHttpClient

/**
 * SPKI pins for `relay.sassyconsultingllc.com`.
 *
 * Cloudflare leaf certs rotate ~90 days; this pin-set is GTS WE1/WE2 + WR1/WR2
 * **intermediates** (captured 2026-08-14, not-after 2029-02-20), not a single
 * leaf. Production default is on because [pinsComplete] is true. MDM
 * `require_tls_pinning=false` or [processEnabled] false disables (platform TLS).
 * Mismatch is fail-closed when enabled. Rotation: `docs/TLS_PINNING.md`.
 *
 * Must stay in lockstep with `core/src/tls_pins.rs`.
 */
object RelayTlsPins {
    const val HOST = "relay.sassyconsultingllc.com"

    val SPKI_SHA256_B64 = listOf(
        "kIdp6NNEd8wsugYyyIYFsi1ylMCED3hZbSR8ZFsa/A4=", // GTS WE1
        "vh78KSg1Ry4NaqGDV10w/cTb9VH3BQUZoCWNa93W/EY=", // GTS WE2
        "yDu9og255NN5GEf+Bwa9rTrqFQ0EydZ0r1FCh9TdAW4=", // GTS WR1
        "YPtHaftLw6/0vnc2BnNKGF54xiCA28WFcccjkA4ypCM=", // GTS WR2
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
