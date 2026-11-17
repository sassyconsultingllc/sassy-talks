// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

/**
 * Chooses the transmit Opus bitrate from the uplink the audio leaves on.
 *
 * One encoder feeds every transport, so there is a single rate to pick. Mobile
 * data is the only uplink that cannot be relied on to carry the full-quality
 * stream: at 64 kbps a weak cell queues frames faster than it sends them, they
 * go stale and are dropped, and the room hears that sender in pieces while
 * everyone on Wi-Fi sounds fine. Receivers need no signalling, since every
 * Opus packet states its own configuration.
 *
 * Must match `android-native/src/codec.rs` (VOICE_BITRATE / CELLULAR_BITRATE).
 */
internal object TxBitratePolicy {
    /** Full quality: Wi-Fi, Bluetooth, and any session that is not using the relay. */
    const val FULL_BPS = 64_000

    /** While relay audio is leaving over mobile data. */
    const val CELLULAR_BPS = 24_000

    /**
     * @param relayActive the relay socket is up, so audio is leaving over the internet
     * @param osHasCellular the phone's active network is mobile data
     * @param osHasWifi the phone's active network is Wi-Fi
     */
    fun forUplink(relayActive: Boolean, osHasCellular: Boolean, osHasWifi: Boolean): Int =
        if (relayActive && osHasCellular && !osHasWifi) CELLULAR_BPS else FULL_BPS
}
