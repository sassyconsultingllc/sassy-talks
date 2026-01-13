package com.sassyconsulting.sassytalkie.ui

import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*

@Composable
fun MainScreen() {
    var isTransmitting by remember { mutableStateOf(false) }
    var currentChannel by remember { mutableIntStateOf(1) }
    var isConnected by remember { mutableStateOf(true) }
    var peerCount by remember { mutableIntStateOf(0) }
    
    // Pulse animation for transmitting
    val infiniteTransition = rememberInfiniteTransition(label = "pulse")
    val pulseScale by infiniteTransition.animateFloat(
        initialValue = 1f,
        targetValue = 1.15f,
        animationSpec = infiniteRepeatable(
            animation = tween(600, easing = EaseInOut),
            repeatMode = RepeatMode.Reverse
        ),
        label = "pulseScale"
    )
    
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DarkBg)
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // Header
        HeaderSection(isConnected = isConnected, peerCount = peerCount)
        
        Spacer(modifier = Modifier.height(24.dp))
        
        // Channel Selector
        ChannelSelector(
            channel = currentChannel,
            onChannelChange = { newChannel ->
                currentChannel = newChannel
                SassyTalkNative.setChannel(newChannel)
            }
        )
        
        Spacer(modifier = Modifier.weight(1f))
        
        // PTT Button
        PTTButton(
            isTransmitting = isTransmitting,
            pulseScale = if (isTransmitting) pulseScale else 1f,
            onPressStart = {
                isTransmitting = true
                SassyTalkNative.pttStart()
            },
            onPressEnd = {
                isTransmitting = false
                SassyTalkNative.pttStop()
            }
        )
        
        Spacer(modifier = Modifier.weight(1f))
        
        // Status Bar
        StatusBar(isTransmitting = isTransmitting, channel = currentChannel)
        
        Spacer(modifier = Modifier.height(16.dp))
    }
}

@Composable
private fun HeaderSection(isConnected: Boolean, peerCount: Int) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        // App title
        Text(
            text = "Sassy-Talk",
            fontSize = 28.sp,
            fontWeight = FontWeight.Bold,
            color = Orange
        )
        
        // Connection status
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Box(
                modifier = Modifier
                    .size(12.dp)
                    .clip(CircleShape)
                    .background(if (isConnected) StatusConnected else StatusDisconnected)
            )
            Text(
                text = if (isConnected) "Connected" else "Offline",
                fontSize = 14.sp,
                color = TextGray
            )
        }
    }
}

@Composable
private fun ChannelSelector(
    channel: Int,
    onChannelChange: (Int) -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = CardBg),
        shape = RoundedCornerShape(16.dp)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            // Channel down
            IconButton(
                onClick = { if (channel > 1) onChannelChange(channel - 1) },
                modifier = Modifier
                    .size(56.dp)
                    .clip(CircleShape)
                    .background(SurfaceBg)
            ) {
                Icon(
                    Icons.Default.Remove,
                    contentDescription = "Channel Down",
                    tint = Cyan,
                    modifier = Modifier.size(32.dp)
                )
            }
            
            // Channel display
            Column(
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Text(
                    text = "CHANNEL",
                    fontSize = 12.sp,
                    color = TextMuted,
                    letterSpacing = 2.sp
                )
                Text(
                    text = "%02d".format(channel),
                    fontSize = 48.sp,
                    fontWeight = FontWeight.Bold,
                    color = Cyan
                )
            }
            
            // Channel up
            IconButton(
                onClick = { if (channel < 99) onChannelChange(channel + 1) },
                modifier = Modifier
                    .size(56.dp)
                    .clip(CircleShape)
                    .background(SurfaceBg)
            ) {
                Icon(
                    Icons.Default.Add,
                    contentDescription = "Channel Up",
                    tint = Cyan,
                    modifier = Modifier.size(32.dp)
                )
            }
        }
    }
}

@Composable
private fun PTTButton(
    isTransmitting: Boolean,
    pulseScale: Float,
    onPressStart: () -> Unit,
    onPressEnd: () -> Unit
) {
    val buttonColor = if (isTransmitting) Orange else SurfaceBg
    val ringColor = if (isTransmitting) Orange else Cyan
    val innerColor = if (isTransmitting) OrangeLight else CardBg
    
    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier
            .size(280.dp)
            .scale(pulseScale)
    ) {
        // Outer glow when transmitting
        if (isTransmitting) {
            Box(
                modifier = Modifier
                    .size(300.dp)
                    .clip(CircleShape)
                    .background(
                        Brush.radialGradient(
                            colors = listOf(
                                Orange.copy(alpha = 0.3f),
                                Color.Transparent
                            )
                        )
                    )
            )
        }
        
        // Main button
        Box(
            contentAlignment = Alignment.Center,
            modifier = Modifier
                .size(240.dp)
                .clip(CircleShape)
                .border(8.dp, ringColor, CircleShape)
                .background(buttonColor)
                .pointerInput(Unit) {
                    detectTapGestures(
                        onPress = {
                            onPressStart()
                            tryAwaitRelease()
                            onPressEnd()
                        }
                    )
                }
        ) {
            // Inner circle
            Box(
                contentAlignment = Alignment.Center,
                modifier = Modifier
                    .size(120.dp)
                    .clip(CircleShape)
                    .background(innerColor)
            ) {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Icon(
                        if (isTransmitting) Icons.Default.Mic else Icons.Default.MicNone,
                        contentDescription = null,
                        tint = if (isTransmitting) TextWhite else TextGray,
                        modifier = Modifier.size(40.dp)
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    Text(
                        text = if (isTransmitting) "TX" else "PTT",
                        fontSize = 18.sp,
                        fontWeight = FontWeight.Bold,
                        color = if (isTransmitting) TextWhite else TextGray
                    )
                }
            }
        }
    }
}

@Composable
private fun StatusBar(isTransmitting: Boolean, channel: Int) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = if (isTransmitting) Orange.copy(alpha = 0.2f) else CardBg
        ),
        shape = RoundedCornerShape(12.dp)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                if (isTransmitting) Icons.Default.RadioButtonChecked else Icons.Default.RadioButtonUnchecked,
                contentDescription = null,
                tint = if (isTransmitting) Orange else TextGray,
                modifier = Modifier.size(20.dp)
            )
            Spacer(modifier = Modifier.width(8.dp))
            Text(
                text = if (isTransmitting) "TRANSMITTING ON CH $channel" else "READY - HOLD TO TALK",
                fontSize = 14.sp,
                fontWeight = FontWeight.Medium,
                color = if (isTransmitting) Orange else TextGray,
                letterSpacing = 1.sp
            )
        }
    }
}
