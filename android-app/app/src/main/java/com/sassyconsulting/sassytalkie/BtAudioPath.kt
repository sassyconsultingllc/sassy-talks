// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-BTAUDIOPATH01
package com.sassyconsulting.sassytalkie

/**
 * Pure helpers for Bluetooth audio readiness.
 *
 * BLE is control-plane only (PTT tones / heartbeats). Encrypted voice rides
 * RFCOMM (or an IP plane). Treating BLE discovery as "connected" caused silent
 * TX: UI said Bluetooth, but [BluetoothTransport.startTxPump] never ran.
 */
object BtAudioPath {

    /** True when RFCOMM sockets can carry encrypted PTT audio. */
    fun isBluetoothAudioReady(rfcommPeers: Int): Boolean = rfcommPeers > 0

    /**
     * PTT may open the mic only when an IP plane is up or at least one RFCOMM
     * peer is linked. BLE-only is not enough.
     */
    fun canTransmit(ipUp: Boolean, rfcommPeers: Int): Boolean =
        ipUp || isBluetoothAudioReady(rfcommPeers)

    /** User-facing status while BLE peers exist but RFCOMM is still dialing. */
    fun linkingStatus(blePeers: Int, rfcommPeers: Int): String? {
        if (rfcommPeers > 0) return null
        if (blePeers <= 0) return null
        return "Linking Bluetooth…"
    }

    fun connectedBluetoothStatus(rfcommPeers: Int): String =
        if (rfcommPeers == 1) "Connected via Bluetooth"
        else "Connected via Bluetooth ($rfcommPeers peers)"

    const val REJECT_NO_AUDIO_PATH =
        "No audio path — wait for WiFi, relay, or Bluetooth data link"
    const val REJECT_BT_LINKING =
        "Bluetooth data link not ready — still linking nearby peer"
}
