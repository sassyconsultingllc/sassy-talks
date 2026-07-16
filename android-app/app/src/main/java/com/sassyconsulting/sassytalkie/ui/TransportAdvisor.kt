// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-Z3QVWGD7IMAO
package com.sassyconsulting.sassytalkie.ui

/**
 * Scores available PTT audio planes and surfaces user-facing guidance on which
 * path is active and whether a better one is reachable.
 *
 * Ranking (best → worst): WiFi+Relay dual-path > WiFi multicast > Cloudflare
 * relay > Bluetooth. Bluetooth is the intentional last resort when cellular
 * and WiFi are down — it needs no infrastructure but carries higher latency.
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
        AudioPlane.WIFI -> 3
        AudioPlane.RELAY -> 2
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
            avail.wifiActive -> AudioPlane.WIFI
            avail.relayActive -> AudioPlane.RELAY
            avail.bluetoothPeers > 0 && avail.bluetoothAllowed -> AudioPlane.BLUETOOTH
            else -> AudioPlane.NONE
        }

        val potentialCandidates = buildList {
            if (avail.wifiAllowed && avail.osHasWifi) add(AudioPlane.WIFI)
            if (avail.relayAllowed && avail.osHasCellular) add(AudioPlane.RELAY)
            if (avail.bluetoothAllowed && avail.bluetoothPeers > 0) add(AudioPlane.BLUETOOTH)
        }
        val potentialBest = when {
            avail.wifiAllowed && avail.relayAllowed && avail.osHasWifi && avail.osHasCellular ->
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
            rank(bestReachable) > rank(active) -> bestReachable
            else -> null
        }

        val (message, severity) = when {
            active == AudioPlane.NONE && avail.bluetoothPeers == 0 ->
                "No transport available — enable WiFi, relay, or move near a Bluetooth peer" to
                    AdvisorySeverity.DEGRADED

            active == AudioPlane.BLUETOOTH && avail.osHasWifi && avail.wifiAllowed ->
                "WiFi available — lower latency than Bluetooth" to AdvisorySeverity.UPGRADE

            active == AudioPlane.BLUETOOTH && !avail.osHasWifi && !avail.osHasCellular ->
                "Cellular down — encrypted audio over Bluetooth (no internet needed)" to
                    AdvisorySeverity.OK

            active == AudioPlane.RELAY && avail.osHasWifi && avail.wifiAllowed ->
                "WiFi multicast available — better latency and stays on your LAN" to
                    AdvisorySeverity.UPGRADE

            active == AudioPlane.WIFI && avail.relayActive ->
                "WiFi + relay active — local peers on WiFi, remote peers on relay" to
                    AdvisorySeverity.OK

            active == AudioPlane.BOTH_WIFI_RELAY ->
                "Best path — WiFi for local peers, relay for remote" to AdvisorySeverity.OK

            active == AudioPlane.WIFI ->
                "Encrypted PTT over WiFi multicast" to AdvisorySeverity.OK

            active == AudioPlane.RELAY && !avail.osHasCellular ->
                "Relay only — cellular/WiFi data path (no local multicast)" to AdvisorySeverity.OK

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
