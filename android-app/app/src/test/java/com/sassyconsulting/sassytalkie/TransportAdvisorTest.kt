// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-I2PB7D2KLDYN
package com.sassyconsulting.sassytalkie

import com.sassyconsulting.sassytalkie.ui.AdvisorySeverity
import com.sassyconsulting.sassytalkie.ui.AudioPlane
import com.sassyconsulting.sassytalkie.ui.TransportAdvisor
import com.sassyconsulting.sassytalkie.ui.TransportAvailability
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TransportAdvisorTest {

    @Test
    fun bluetoothOnlyWhenCellularDown_advisesNoInternetNeeded() {
        val avail = TransportAvailability(
            wifiActive = false,
            relayActive = false,
            bluetoothPeers = 2,
            osHasWifi = false,
            osHasCellular = false,
            wifiAllowed = true,
            relayAllowed = true,
            bluetoothAllowed = true,
        )
        val advisory = TransportAdvisor.evaluate("bluetooth", avail)
        assertEquals(AudioPlane.BLUETOOTH, advisory.activePlane)
        assertNotNull(advisory.message)
        assert(advisory.message!!.contains("Bluetooth", ignoreCase = true))
    }

    @Test
    fun relayActive_wifiAvailable_recommendsUpgrade() {
        val avail = TransportAvailability(
            wifiActive = false,
            relayActive = true,
            bluetoothPeers = 0,
            osHasWifi = true,
            osHasCellular = true,
            wifiAllowed = true,
            relayAllowed = true,
            bluetoothAllowed = true,
        )
        val advisory = TransportAdvisor.evaluate("cellular", avail)
        assertEquals(AdvisorySeverity.UPGRADE, advisory.severity)
        assertEquals(AudioPlane.BOTH_WIFI_RELAY, advisory.betterPlane)
    }

    @Test
    fun dualPath_isBestRank() {
        val avail = TransportAvailability(
            wifiActive = true,
            relayActive = true,
            bluetoothPeers = 1,
            osHasWifi = true,
            osHasCellular = true,
            wifiAllowed = true,
            relayAllowed = true,
            bluetoothAllowed = true,
        )
        val advisory = TransportAdvisor.evaluate("both", avail)
        assertEquals(AudioPlane.BOTH_WIFI_RELAY, advisory.activePlane)
        assertEquals(AdvisorySeverity.OK, advisory.severity)
    }

    @Test
    fun zeroRfcommPeers_bluetoothNotAvailable() {
        // bluetoothPeers must mean RFCOMM (audio), not BLE control peers.
        val noneAvail = TransportAdvisor.availablePlanes(
            TransportAvailability(
                wifiActive = false,
                relayActive = false,
                bluetoothPeers = 0,
                osHasWifi = false,
                osHasCellular = false,
                wifiAllowed = true,
                relayAllowed = true,
                bluetoothAllowed = true,
            ),
        )
        assertFalse(noneAvail.contains(AudioPlane.BLUETOOTH))

        val withRfcomm = TransportAdvisor.availablePlanes(
            TransportAvailability(
                wifiActive = false,
                relayActive = false,
                bluetoothPeers = 1,
                osHasWifi = false,
                osHasCellular = false,
                wifiAllowed = true,
                relayAllowed = true,
                bluetoothAllowed = true,
            ),
        )
        assertTrue(withRfcomm.contains(AudioPlane.BLUETOOTH))
    }
}
