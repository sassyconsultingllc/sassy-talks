// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-LNTTE3PWOCXQ
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import com.sassyconsulting.sassytalkie.translate.LiveTranslationBridge
import com.sassyconsulting.sassytalkie.ui.theme.*
import kotlinx.coroutines.delay

/**
 * Compact UI shown while the app is in picture-in-picture mode — keeps channel,
 * transport, and TX/RX status visible while the user multitasks.
 */
@Composable
fun PipRadioOverlay(isTransmitting: Boolean) {
    val incoming by TranscriptionBridge.incomingAudio.collectAsState()
    val speaker by TranscriptionBridge.activeSpeakerName.collectAsState()
    val translationOn by LiveTranslationBridge.enabled.collectAsState()
    val translationText by LiveTranslationBridge.translation.collectAsState()
    val captionText by LiveTranslationBridge.caption.collectAsState()
    val translationPaused by LiveTranslationBridge.pausedForPtt.collectAsState()

    DisposableEffect(Unit) {
        LiveTranslationBridge.acquireUi()
        onDispose { LiveTranslationBridge.releaseUi() }
    }

    var channel by remember { mutableIntStateOf(SassyTalkNative.getChannel()) }
    var transport by remember { mutableStateOf(SassyTalkNative.getTransportName()) }
    var connected by remember { mutableStateOf(SassyTalkNative.isConnected()) }

    LaunchedEffect(Unit) {
        while (true) {
            delay(1_000L)
            channel = SassyTalkNative.getChannel()
            transport = SassyTalkNative.getTransportName()
            connected = SassyTalkNative.isConnected()
        }
    }

    val pulse = rememberInfiniteTransition(label = "pipPulse")
    val pulseScale by pulse.animateFloat(
        initialValue = 1f,
        targetValue = 1.12f,
        animationSpec = infiniteRepeatable(
            animation = tween(700, easing = EaseInOut),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "pipPulseScale",
    )

    val active = isTransmitting || incoming

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Brush.radialGradient(listOf(BgMedium, BgDark)))
            .padding(12.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Text(
                text = "CH %02d".format(channel),
                fontSize = 22.sp,
                fontWeight = FontWeight.Bold,
                color = TealLight,
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = if (connected) transport else "Disconnected",
                fontSize = 11.sp,
                color = if (connected) StatusConnected else StatusDisconnected,
                textAlign = TextAlign.Center,
                maxLines = 2,
            )
            Spacer(modifier = Modifier.height(12.dp))
            Box(
                contentAlignment = Alignment.Center,
                modifier = Modifier
                    .size(56.dp)
                    .scale(if (active) pulseScale else 1f)
                    .clip(CircleShape)
                    .background(
                        when {
                            isTransmitting -> Brush.linearGradient(GradientWarm)
                            incoming -> Brush.linearGradient(GradientCool)
                            else -> Brush.linearGradient(GradientCool)
                        },
                    ),
            ) {
                Icon(
                    imageVector = if (isTransmitting) Icons.Default.Mic else Icons.Default.MicOff,
                    contentDescription = if (isTransmitting) "Transmitting" else "Microphone off",
                    tint = if (active) TextWhite else TextGray,
                    modifier = Modifier.size(28.dp),
                )
            }
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = when {
                    isTransmitting -> "Transmitting"
                    incoming && speaker.isNotBlank() -> "$speaker speaking"
                    connected -> "Standby"
                    else -> "Sassy-Talk"
                },
                fontSize = 12.sp,
                color = if (active) Orange else TextMuted,
                textAlign = TextAlign.Center,
            )
            if (translationOn) {
                Spacer(modifier = Modifier.height(6.dp))
                val translationStatus by LiveTranslationBridge.status.collectAsState()
                Text(
                    text = when {
                        translationPaused || isTransmitting -> "Translation paused"
                        translationText.isNotBlank() -> translationText
                        captionText.isNotBlank() -> captionText
                        translationStatus ==
                            com.sassyconsulting.sassytalkie.translate.LiveCaptionTranslator.Status.LISTENING ->
                            "Listening…"
                        else -> "Quiet — resuming…"
                    },
                    fontSize = 11.sp,
                    color = Orange,
                    textAlign = TextAlign.Center,
                    maxLines = 2,
                )
            }
        }
    }
}
