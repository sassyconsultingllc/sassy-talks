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
    var expanded by remember { mutableStateOf(true) }
    val s by AudioTelemetry.state.collectAsState()

    val border = if (s.gateOpen) Color(0xFF00FF88) else Color(0xFF666666)

    Box(
        modifier = modifier
            .clip(RoundedCornerShape(6.dp))
            .background(Color(0xCC0A0A0A))
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
            color = if (s.gateOpen) Color(0xFF00FF88) else Color(0xFFAAAAAA),
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
private fun DeviceSection(s: AudioTelemetry.State) {
    Mono("DEVICE")
    Mono("  ${s.deviceLabel}")
    if (s.quirkNotes.isNotBlank()) {
        Mono("  ${s.quirkNotes}", color = Color(0xFFFFAA00))
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
        gateOpen -> Color(0xFF00FF88)
        pct > 0.6f -> Color(0xFFFFAA00)
        else -> Color(0xFF666666)
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
                        .background(Color(0xFFFF4444))
                )
            }
        }
    }
}

@Composable
private fun Mono(
    text: String,
    color: Color = Color(0xFFE0E0E0),
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
