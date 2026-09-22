// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-Z3QVWGD7IMAO
package com.sassyconsulting.sassytalkie.ui

/**
 * Scores available PTT audio planes and surfaces user-facing guidance on which
 * path is active and whether a better one is reachable.
 *
 * Ranking (best → worst): Relay+WiFi dual-path > Cloudflare relay > WiFi
 * multicast > Bluetooth. Relay is the common meeting point so mixed-protocol
 * peers can hear each other; WiFi is additive for nearby devices; Bluetooth
 * is last resort when no IP path is up.
 */
enum class AudioPlane {
    BOTH_WIFI_RELAY,
    WIFI,
    RELAY,
    BLUETOOTH,
    NONE,
}

enum class AdvisorySeverity { OK, UPGRADE, DEGRADED }

data class TransportAvailability(
    val wifiActive: Boolean,
    val relayActive: Boolean,
    val bluetoothPeers: Int,
    val osHasWifi: Boolean,
    val osHasCellular: Boolean,
    val osHasInternet: Boolean = osHasWifi || osHasCellular,
    val wifiAllowed: Boolean,
    val relayAllowed: Boolean,
    val bluetoothAllowed: Boolean,
)

data class TransportAdvisory(
    val activePlane: AudioPlane,
    val activeLabel: String,
    val availablePlanes: List<AudioPlane>,
    val betterPlane: AudioPlane?,
    val message: String?,
    val severity: AdvisorySeverity,
)

object TransportAdvisor {

    private fun rank(plane: AudioPlane): Int = when (plane) {
        AudioPlane.BOTH_WIFI_RELAY -> 4
        AudioPlane.RELAY -> 3
        AudioPlane.WIFI -> 2
        AudioPlane.BLUETOOTH -> 1
        AudioPlane.NONE -> 0
    }

    private fun label(plane: AudioPlane): String = when (plane) {
        AudioPlane.BOTH_WIFI_RELAY -> "WiFi + Relay"
        AudioPlane.WIFI -> "WiFi"
        AudioPlane.RELAY -> "Cloudflare Relay"
        AudioPlane.BLUETOOTH -> "Bluetooth"
        AudioPlane.NONE -> "None"
    }

    /** Map AutoConnectManager's internal activeTransport string to [AudioPlane]. */
    fun planeFromActiveTransport(activeTransport: String): AudioPlane = when (activeTransport) {
        "both" -> AudioPlane.BOTH_WIFI_RELAY
        "wifi" -> AudioPlane.WIFI
        "cellular" -> AudioPlane.RELAY
        "bluetooth" -> AudioPlane.BLUETOOTH
        else -> AudioPlane.NONE
    }

    /** List every plane that could carry encrypted audio right now. */
    fun availablePlanes(avail: TransportAvailability): List<AudioPlane> = buildList {
        if (avail.wifiActive) add(AudioPlane.WIFI)
        if (avail.relayActive) add(AudioPlane.RELAY)
        if (avail.bluetoothPeers > 0 && avail.bluetoothAllowed) add(AudioPlane.BLUETOOTH)
        if (avail.wifiActive && avail.relayActive) {
            // Dual-path is richer than either alone — represent explicitly.
            remove(AudioPlane.WIFI)
            remove(AudioPlane.RELAY)
            add(0, AudioPlane.BOTH_WIFI_RELAY)
        }
    }.distinctBy { it }

    /** Best plane reachable right now — active transports first, then OS-level potential. */
    fun bestReachablePlane(avail: TransportAvailability): AudioPlane {
        val activeBest = when {
            avail.wifiActive && avail.relayActive -> AudioPlane.BOTH_WIFI_RELAY
            avail.relayActive -> AudioPlane.RELAY
            avail.wifiActive -> AudioPlane.WIFI
            avail.bluetoothPeers > 0 && avail.bluetoothAllowed -> AudioPlane.BLUETOOTH
            else -> AudioPlane.NONE
        }

        val potentialCandidates = buildList {
            if (avail.relayAllowed && avail.osHasInternet) add(AudioPlane.RELAY)
            if (avail.wifiAllowed && avail.osHasWifi) add(AudioPlane.WIFI)
            if (avail.bluetoothAllowed && avail.bluetoothPeers > 0) add(AudioPlane.BLUETOOTH)
        }
        val potentialBest = when {
            avail.wifiAllowed && avail.relayAllowed && avail.osHasWifi && avail.osHasInternet ->
                AudioPlane.BOTH_WIFI_RELAY
            potentialCandidates.isEmpty() -> AudioPlane.NONE
            else -> potentialCandidates.maxByOrNull { rank(it) } ?: AudioPlane.NONE
        }

        return if (rank(potentialBest) > rank(activeBest)) potentialBest else activeBest
    }

    fun evaluate(activeTransport: String, avail: TransportAvailability): TransportAdvisory {
        val active = planeFromActiveTransport(activeTransport)
        val available = availablePlanes(avail)
        val bestReachable = bestReachablePlane(avail)

        val better = when {
            active == AudioPlane.NONE && bestReachable != AudioPlane.NONE -> bestReachable
            // Relay is the hub — WiFi alongside it is additive, not an upgrade nag.
            active == AudioPlane.RELAY &&
                (bestReachable == AudioPlane.BOTH_WIFI_RELAY || bestReachable == AudioPlane.WIFI) ->
                null
            rank(bestReachable) > rank(active) -> bestReachable
            else -> null
        }

        val (message, severity) = when {
            active == AudioPlane.NONE && avail.bluetoothPeers == 0 ->
                "No transport available — enable WiFi, relay, or move near a Bluetooth peer" to
                    AdvisorySeverity.DEGRADED

            active == AudioPlane.BLUETOOTH && avail.relayAllowed && avail.osHasInternet ->
                "Relay available — mixed-protocol peers can hear each other" to
                    AdvisorySeverity.UPGRADE

            active == AudioPlane.BLUETOOTH && avail.osHasWifi && avail.wifiAllowed ->
                "WiFi available — lower latency than Bluetooth" to AdvisorySeverity.UPGRADE

            active == AudioPlane.BLUETOOTH && !avail.osHasWifi && !avail.osHasInternet ->
                "No internet — encrypted audio over Bluetooth" to
                    AdvisorySeverity.OK

            active == AudioPlane.RELAY && avail.wifiActive ->
                "Relay + WiFi — common path for all peers, LAN for nearby" to
                    AdvisorySeverity.OK

            active == AudioPlane.RELAY ->
                "Cloudflare Relay — common path so every peer can hear" to AdvisorySeverity.OK

            active == AudioPlane.WIFI && avail.relayActive ->
                "WiFi + relay active — local peers on WiFi, remote peers on relay" to
                    AdvisorySeverity.OK

            active == AudioPlane.BOTH_WIFI_RELAY ->
                "Best path — relay for every peer, WiFi for nearby" to AdvisorySeverity.OK

            active == AudioPlane.WIFI ->
                "Encrypted PTT over WiFi multicast" to AdvisorySeverity.OK

            active == AudioPlane.BLUETOOTH ->
                "Encrypted PTT over Bluetooth — works when cellular is down" to AdvisorySeverity.OK

            better != null ->
                "${label(better)} would be better than ${label(active)}" to AdvisorySeverity.UPGRADE

            else -> null to AdvisorySeverity.OK
        }

        return TransportAdvisory(
            activePlane = active,
            activeLabel = label(active),
            availablePlanes = available,
            betterPlane = better,
            message = message,
            severity = severity,
        )
    }
}
