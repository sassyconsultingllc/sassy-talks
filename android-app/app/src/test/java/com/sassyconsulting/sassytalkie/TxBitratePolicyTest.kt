// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Test

class TxBitratePolicyTest {

    @Test
    fun relayOverMobileDataUsesTheLowerRate() {
        assertEquals(
            TxBitratePolicy.CELLULAR_BPS,
            TxBitratePolicy.forUplink(relayActive = true, osHasCellular = true, osHasWifi = false),
        )
    }

    @Test
    fun relayOverWifiKeepsFullQuality() {
        assertEquals(
            TxBitratePolicy.FULL_BPS,
            TxBitratePolicy.forUplink(relayActive = true, osHasCellular = false, osHasWifi = true),
        )
    }

    @Test
    fun mobileDataWithoutTheRelayKeepsFullQuality() {
        // Bluetooth-only or Wi-Fi LAN session on a phone that happens to have
        // mobile data: no audio leaves over the cell.
        assertEquals(
            TxBitratePolicy.FULL_BPS,
            TxBitratePolicy.forUplink(relayActive = false, osHasCellular = true, osHasWifi = false),
        )
    }

    @Test
    fun noNetworkAtAllKeepsFullQuality() {
        assertEquals(
            TxBitratePolicy.FULL_BPS,
            TxBitratePolicy.forUplink(relayActive = false, osHasCellular = false, osHasWifi = false),
        )
    }

    @Test
    fun aNetworkReportingBothTransportsIsTreatedAsWifi() {
        // A VPN can report both underlying transports; Wi-Fi is carrying it.
        assertEquals(
            TxBitratePolicy.FULL_BPS,
            TxBitratePolicy.forUplink(relayActive = true, osHasCellular = true, osHasWifi = true),
        )
    }

    @Test
    fun ratesMatchTheNativeCodecConstants() {
        assertEquals(64_000, TxBitratePolicy.FULL_BPS)
        assertEquals(24_000, TxBitratePolicy.CELLULAR_BPS)
    }
}
