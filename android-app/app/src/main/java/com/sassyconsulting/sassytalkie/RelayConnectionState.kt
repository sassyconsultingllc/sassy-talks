// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

/**
 * Single definition of "relay is live" so the radio UI, SESSION overlay,
 * and TX gating cannot disagree. Kotlin's OkHttp socket and native
 * cellular stats are two views of the same bearer.
 *
 * Socket-up is not the same as Durable Object welcome / sealed-sender room
 * confirm. The radio chrome must not say "waiting for relay" when the
 * WebSocket is already up.
 */
object RelayConnectionState {
    fun isLive(
        kotlinWsConnected: Boolean,
        nativeCellularState: String?,
        nativeLiveAudioPath: Boolean = false,
    ): Boolean {
        if (kotlinWsConnected) return true
        if (nativeLiveAudioPath) return true
        val state = nativeCellularState?.trim().orEmpty()
        return state.equals("connected", ignoreCase = true) ||
            state.equals("connecting", ignoreCase = true)
    }

    fun overlayWsLabel(live: Boolean): String = if (live) "up" else "down"

    fun telemetryWsState(
        kotlinWsConnected: Boolean,
        nativeCellularState: String?,
        nativeLiveAudioPath: Boolean,
    ): String = when {
        kotlinWsConnected -> "connected"
        nativeCellularState.equals("connected", ignoreCase = true) -> "connected"
        nativeLiveAudioPath -> "transport-up"
        else -> "idle"
    }

    /**
     * User-visible radio status. Never claims the relay socket is down when
     * [socketLive] is true.
     */
    fun radioStatusLine(
        socketLive: Boolean,
        roomConfirmed: Boolean,
        sealedSenderPending: Boolean = false,
        usingRelay: Boolean = true,
    ): String? {
        if (!usingRelay) return null
        return when {
            socketLive && roomConfirmed && sealedSenderPending ->
                "Relay connected — sealed-sender room pending"
            socketLive && roomConfirmed -> null
            socketLive && !roomConfirmed ->
                "Relay connected — confirming room…"
            !socketLive ->
                "Connecting to relay…"
            else -> null
        }
    }
}
