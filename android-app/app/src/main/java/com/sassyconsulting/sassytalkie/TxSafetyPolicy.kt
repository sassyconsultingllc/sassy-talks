// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

/** Pure TX safety decisions shared with focused JVM tests. */
object TxSafetyPolicy {
    const val MIN_MAX_TX_MS = 10_000L
    const val MAX_MAX_TX_MS = 300_000L

    fun normalizeMaxTxMs(value: Long): Long =
        value.coerceIn(MIN_MAX_TX_MS, MAX_MAX_TX_MS)

    fun shouldForceForTransportLoss(
        transmitting: Boolean,
        ipConnected: Boolean,
        rfcommPeers: Int,
    ): Boolean = transmitting && !ipConnected && rfcommPeers <= 0
}
