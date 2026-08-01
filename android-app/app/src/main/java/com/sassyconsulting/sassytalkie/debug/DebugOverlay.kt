// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-WRWY2UL6WADE
package com.sassyconsulting.sassytalkie.debug

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.ui.theme.StatusOnline
import com.sassyconsulting.sassytalkie.ui.theme.StatusWarning
import com.sassyconsulting.sassytalkie.ui.theme.StatusErrorToken
import com.sassyconsulting.sassytalkie.ui.theme.TextSecondary
import com.sassyconsulting.sassytalkie.ui.theme.TextMutedToken

/**
 * Always-on debug overlay. Tap header to collapse / expand.
 *
 * Drop into your top-level Scaffold:
 *   Box(Modifier.fillMaxSize()) {
 *       YourMainContent()
 *       DebugOverlay(
 *           Modifier
 *               .align(Alignment.TopEnd)
 *               .padding(8.dp)
 *               .width(280.dp)
 *       )
 *   }
 *
 * Gate by BuildConfig.DEBUG or a settings toggle so it never ships to prod.
 */
@Composable
fun DebugOverlay(modifier: Modifier = Modifier) {
    // Start collapsed to a compact header strip; tap to expand the full readout.
    var expanded by remember { mutableStateOf(false) }
    val s by AudioTelemetry.state.collectAsState()

    val border = if (s.gateOpen) StatusOnline else TextMutedToken

    Box(
        modifier = modifier
            .clip(RoundedCornerShape(6.dp))
            .background(Color(0xE60F172A)) // slate (BgDark) ~90% opaque
            .border(1.dp, border, RoundedCornerShape(6.dp))
            .pointerInput(Unit) { detectTapGestures { expanded = !expanded } }
            .padding(8.dp)
    ) {
        Column {
            HeaderRow(s)
            if (expanded) {
                Spacer(Modifier.height(6.dp))
                Divider()
                Spacer(Modifier.height(4.dp))
                GateSection(s)
                Spacer(Modifier.height(4.dp))
                Divider()
                Spacer(Modifier.height(4.dp))
                CaptureSection(s)
                Spacer(Modifier.height(4.dp))
                Divider()
                Spacer(Modifier.height(4.dp))
                NetSection(s)
                Spacer(Modifier.height(4.dp))
                Divider()
                Spacer(Modifier.height(4.dp))
                RelaySection(s)
                Spacer(Modifier.height(4.dp))
                Divider()
                Spacer(Modifier.height(4.dp))
                DeviceSection(s)
            }
        }
    }
}

@Composable
private fun HeaderRow(s: AudioTelemetry.State) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Meter(
            currentDbfs = s.currentFrameDbfs,
            openDbfs = s.gateThresholdDbfs,
            noiseFloorDbfs = s.gateNoiseFloorDbfs,
            gateOpen = s.gateOpen,
            modifier = Modifier.width(140.dp).height(14.dp),
        )
        Spacer(Modifier.width(8.dp))
        Mono(
            "TX ${s.txPacketsPerSec}/s  RX ${s.rxPacketsPerSec}/s",
            color = if (s.gateOpen) StatusOnline else TextSecondary,
            bold = true,
        )
    }
}

@Composable
private fun GateSection(s: AudioTelemetry.State) {
    Mono("GATE")
    Mono(
        "  mode=${s.gateMode}  open=${onOff(s.gateOpen)}  " +
            "frame=${fmt1(s.currentFrameDbfs)} dBFS"
    )
    Mono(
        "  thr=${s.gateThresholdDbfs?.let { fmt1(it) } ?: "--"}  " +
            "floor=${s.gateNoiseFloorDbfs?.let { fmt1(it) } ?: "--"} dBFS"
    )
}

@Composable
private fun CaptureSection(s: AudioTelemetry.State) {
    Mono("CAPTURE / CODEC")
    Mono("  src=${s.audioSource}  frames ${s.framesCapturedPerSec}/s  gated ${s.framesGatedPerSec}/s")
    Mono(
        "  AEC=${onOff(s.aecActive)}  NS=${onOff(s.nsActive)}  " +
            "AGC=${onOff(s.agcActive)}  comm=${onOff(s.outputCommMode)}"
    )
    Mono(
        "  tx ${fmt1(s.txKbpsAvg)} kbps  last=${s.lastTxPacketBytes}B  " +
            "rx err ${s.rxErrorsPerSec}/s"
    )
}

@Composable
private fun NetSection(s: AudioTelemetry.State) {
    Mono("NET")
    Mono(
        "  path=${s.networkPath}  ws=${s.wsState}  " +
            "rtt=${s.rttMs?.let { "${it}ms" } ?: "--"}"
    )
    Mono(
        "  hb=${s.lastHeartbeatAgoMs?.let { "${it}ms ago" } ?: "--"}  " +
            "rx ${fmt1(s.rxKbpsAvg)} kbps"
    )
}

@Composable
private fun RelaySection(s: AudioTelemetry.State) {
    Mono("SESSION / RELAY")
    val roomColor = if (s.roomMatch) TextSecondary else StatusErrorToken
    Mono("  room=${s.relayRoom.ifEmpty { "--" }}", color = roomColor)
    if (!s.roomMatch) {
        Mono("  ROOM MISMATCH — reconnect or re-import session", color = StatusErrorToken)
    }
    Mono(
        "  cell=${s.cellularState.ifEmpty { "--" }}  ws=${if (s.wsRelayConnected) "up" else "down"}  " +
            "ch=${s.activeChannel}",
    )
    Mono(
        "  sent=${s.cellularSent}  rx=${s.cellularReceived}  " +
            "q in/out=${s.inboundQueue}/${s.outboundQueue}",
    )
    Mono(
        "  drop q=${s.droppedPackets}  ws_tx_drop=${s.wsSendDrops}  " +
            "jitter=${s.jitterPrebufferFrames}f",
        color = if (s.droppedPackets > 0 || s.wsSendDrops > 0) StatusWarning else TextSecondary,
    )
    // Adaptive prebuffer: base + N extra = effective (ewma of |gap − 20 ms|).
    // Highlight when the controller is actively boosting (>0) — that's the
    // signal that the link is jittering enough to warrant extra latency.
    if (s.jitterEffectiveFrames > 0 || s.jitterAdaptiveExtra > 0) {
        Mono(
            "  jb ${s.jitterPrebufferFrames}+${s.jitterAdaptiveExtra}=${s.jitterEffectiveFrames}f  " +
                "ewma=${String.format("%.1f", s.jitterEwmaMs)}ms",
            color = if (s.jitterAdaptiveExtra >= 2) StatusWarning else TextSecondary,
        )
    }
    // Rough local loss signal: queue overflows + OkHttp backpressure drops
    // relative to frames we believe we sent. Not end-to-end loss (peer RX).
    if (s.cellularSent > 20L) {
        val localLoss = s.droppedPackets + s.wsSendDrops
        val pct = (localLoss * 100.0 / s.cellularSent).coerceAtMost(100.0)
        if (pct >= 1.0) {
            Mono(
                "  local drop ~${String.format("%.0f", pct)}% of TX " +
                    "(queue/ws — not relay fan-out)",
                color = StatusWarning,
            )
        }
    }
    Mono("  peers=${s.peerCount}  users=${s.usersInRegistry}")
    if (s.cellularReceived > 0 && s.usersInRegistry == 0) {
        Mono("  RX pkts but no users — check PSK / device name", color = StatusWarning)
    }
}

@Composable
private fun DeviceSection(s: AudioTelemetry.State) {
    Mono("DEVICE")
    Mono("  ${s.deviceLabel}")
    if (s.quirkNotes.isNotBlank()) {
        Mono("  ${s.quirkNotes}", color = StatusWarning)
    }
}

@Composable
private fun Meter(
    currentDbfs: Float,
    openDbfs: Float?,
    noiseFloorDbfs: Float?,
    gateOpen: Boolean,
    modifier: Modifier = Modifier,
) {
    // -60..0 dBFS -> 0..1
    fun mapDb(db: Float): Float = ((db + 60f) / 60f).coerceIn(0f, 1f)
    val pct = mapDb(currentDbfs)
    val barColor = when {
        gateOpen -> StatusOnline
        pct > 0.6f -> StatusWarning
        else -> TextMutedToken
    }
    Box(modifier.background(Color(0x22FFFFFF))) {
        Box(Modifier.fillMaxHeight().fillMaxWidth(pct).background(barColor))
        noiseFloorDbfs?.let {
            Box(
                Modifier
                    .fillMaxHeight()
                    .fillMaxWidth(mapDb(it))
                    .background(Color(0x33888888))
            )
        }
        openDbfs?.let {
            Box(
                Modifier
                    .fillMaxHeight()
                    .fillMaxWidth(mapDb(it))
                    .background(Color.Transparent)
            ) {
                Box(
                    Modifier
                        .fillMaxHeight()
                        .width(2.dp)
                        .align(Alignment.CenterEnd)
                        .background(StatusErrorToken)
                )
            }
        }
    }
}

@Composable
private fun Mono(
    text: String,
    color: Color = TextSecondary,
    bold: Boolean = false,
) {
    Text(
        text,
        color = color,
        fontSize = 10.sp,
        fontFamily = FontFamily.Monospace,
        fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal,
    )
}

@Composable
private fun Divider() {
    Box(Modifier.fillMaxWidth().height(1.dp).background(Color(0x33FFFFFF)))
}

private fun onOff(b: Boolean) = if (b) "on" else "off"
private fun fmt1(f: Float) = String.format("%.1f", f)
