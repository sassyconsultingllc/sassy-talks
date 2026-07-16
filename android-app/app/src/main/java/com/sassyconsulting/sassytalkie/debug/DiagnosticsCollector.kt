// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-4O33K647FYAN
package com.sassyconsulting.sassytalkie.debug

import android.content.Context
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.WalkieService
import org.json.JSONObject

/**
 * One-shot diagnostic snapshot for Settings → Copy, and live overlay polling.
 * Never includes session keys or encrypted payloads.
 */
object DiagnosticsCollector {

    data class CellularStats(
        val state: String = "",
        val room: String = "",
        val sent: Long = 0,
        val received: Long = 0,
        val inboundQueue: Int = 0,
        val outboundQueue: Int = 0,
        val dropped: Long = 0,
    )

    fun parseCellularStats(json: String): CellularStats {
        return try {
            val o = JSONObject(json)
            CellularStats(
                state = o.optString("state", ""),
                room = o.optString("room", ""),
                sent = o.optLong("sent", 0),
                received = o.optLong("received", 0),
                inboundQueue = o.optInt("inbound_queue", 0),
                outboundQueue = o.optInt("outbound_queue", 0),
                dropped = o.optLong("dropped_inbound_overflow", 0) +
                    o.optLong("dropped_outbound_overflow", 0) +
                    o.optLong("dropped_oversize", 0),
            )
        } catch (_: Throwable) {
            CellularStats()
        }
    }

    fun pushLiveTelemetry(walkieService: WalkieService?) {
        if (!SassyTalkNative.isInitialized()) return
        try {
            val sessionRoom = SassyTalkNative.getSessionId().orEmpty()
            val cell = parseCellularStats(SassyTalkNative.cellularGetStats())
            val wsOk = walkieService?.pttCoordinator?.cellularClient?.isConnected() == true
            val peers = walkieService?.pttCoordinator?.peerIds?.value?.size ?: 0
            val users = try { SassyTalkNative.getUsers().size } catch (_: Throwable) { 0 }
            val channel = try { SassyTalkNative.getChannel() } catch (_: Throwable) { 0 }
            val roomMatch = sessionRoom.isEmpty() || cell.room.isEmpty() ||
                sessionRoom == cell.room

            AudioTelemetry.updateRelay(
                relayRoom = sessionRoom.ifEmpty { cell.room },
                cellularState = cell.state,
                wsRelayConnected = wsOk,
                sent = cell.sent,
                received = cell.received,
                inboundQ = cell.inboundQueue,
                outboundQ = cell.outboundQueue,
                dropped = cell.dropped,
                activeChannel = channel,
                peerCount = peers,
                usersInRegistry = users,
                roomMatch = roomMatch,
            )
        } catch (_: Throwable) { /* native not ready */ }
    }

    fun buildTextDump(context: Context, walkieService: WalkieService? = null): String {
        val sb = StringBuilder()
        fun line(k: String, v: Any?) { sb.append(String.format("%-22s : %s%n", k, v)) }

        val prefs = context.getSharedPreferences("sassy_settings", Context.MODE_PRIVATE)
        val sessionRoom = try { SassyTalkNative.getSessionId() } catch (_: Throwable) { null }
        val cell = parseCellularStats(
            try { SassyTalkNative.cellularGetStats() } catch (_: Throwable) { "{}" },
        )
        val wsOk = walkieService?.pttCoordinator?.cellularClient?.isConnected()

        line("App", "SassyTalkie")
        line("versionName", com.sassyconsulting.sassytalkie.BuildConfig.VERSION_NAME)
        line("versionCode", com.sassyconsulting.sassytalkie.BuildConfig.VERSION_CODE)
        line("buildType", com.sassyconsulting.sassytalkie.BuildConfig.BUILD_TYPE)
        line("Device", "${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL}")
        line("Android", "${android.os.Build.VERSION.RELEASE} (SDK ${android.os.Build.VERSION.SDK_INT})")

        line("---", "--- Audio path ---")
        line("Native init", try { SassyTalkNative.isInitialized() } catch (_: Throwable) { false })
        line("Authenticated", try { SassyTalkNative.isAuthenticated() } catch (_: Throwable) { false })
        line("Encrypted", try { SassyTalkNative.isEncrypted() } catch (_: Throwable) { false })
        line("Transport", try { SassyTalkNative.getTransportName() } catch (_: Throwable) { "?" })
        line("Channel", try { SassyTalkNative.getChannel() } catch (_: Throwable) { "?" })
        line("WiFi pref", prefs.getBoolean("enable_wifi_multicast", true))
        line("Relay pref", prefs.getBoolean("enable_cloudflare_relay", true))

        line("---", "--- Relay room ---")
        line("session_id", sessionRoom ?: "(none)")
        line("cellular room", cell.room.ifEmpty { "(none)" })
        line("room match", when {
            sessionRoom.isNullOrEmpty() || cell.room.isEmpty() -> "n/a"
            sessionRoom == cell.room -> "YES"
            else -> "NO — audio may be on wrong room"
        })
        line("cellular state", cell.state.ifEmpty { "?" })
        line("WS connected", wsOk ?: "(service not bound)")
        line("sent / received", "${cell.sent} / ${cell.received}")
        line("in / out queue", "${cell.inboundQueue} / ${cell.outboundQueue}")
        line("dropped packets", cell.dropped)

        line("---", "--- Peers ---")
        val peers = walkieService?.pttCoordinator?.peerIds?.value?.size
        line("active peers (liveness)", peers ?: "(service not bound)")
        line("users in registry", try { SassyTalkNative.getUsers().size } catch (_: Throwable) { 0 })

        line("---", "--- Session JSON ---")
        sb.append(prettyJson(try { SassyTalkNative.getSessionStatus() } catch (_: Throwable) { null }))
            .append('\n')

        line("---", "--- Cache ---")
        sb.append(prettyJson(try { SassyTalkNative.getCacheStatus()?.toString(2) } catch (_: Throwable) { null }))
            .append('\n')

        line("---", "--- Cellular stats JSON ---")
        sb.append(prettyJson(try { SassyTalkNative.cellularGetStats() } catch (_: Throwable) { null }))
            .append('\n')

        return sb.toString()
    }

    private fun prettyJson(raw: String?): String {
        if (raw.isNullOrBlank()) return "(empty)"
        return try { JSONObject(raw).toString(2) } catch (_: Throwable) { raw }
    }
}
