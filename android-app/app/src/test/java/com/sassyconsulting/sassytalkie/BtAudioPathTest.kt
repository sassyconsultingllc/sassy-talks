// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-BTAUDIOPATH02
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BtAudioPathTest {

    @Test
    fun rfcommRequiredForBluetoothAudio() {
        assertFalse(BtAudioPath.isBluetoothAudioReady(0))
        assertTrue(BtAudioPath.isBluetoothAudioReady(1))
    }

    @Test
    fun bleAloneCannotTransmit() {
        assertFalse(BtAudioPath.canTransmit(ipUp = false, rfcommPeers = 0))
        assertTrue(BtAudioPath.canTransmit(ipUp = true, rfcommPeers = 0))
        assertTrue(BtAudioPath.canTransmit(ipUp = false, rfcommPeers = 2))
    }

    @Test
    fun linkingStatusOnlyWhenBleWithoutRfcomm() {
        assertEquals("Linking Bluetooth…", BtAudioPath.linkingStatus(1, 0))
        assertNull(BtAudioPath.linkingStatus(0, 0))
        assertNull(BtAudioPath.linkingStatus(2, 1))
    }

    @Test
    fun connectedStatusNamesPeerCount() {
        assertEquals("Connected via Bluetooth", BtAudioPath.connectedBluetoothStatus(1))
        assertEquals("Connected via Bluetooth (3 peers)", BtAudioPath.connectedBluetoothStatus(3))
    }
}
