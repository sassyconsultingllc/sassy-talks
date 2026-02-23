package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material.icons.outlined.CellTower
import androidx.compose.material.icons.outlined.Wifi
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.CellularWebSocketClient
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.ui.theme.*

@Composable
fun DevicePickerScreen(
    onConnected: () -> Unit,
    onBack: () -> Unit,
    walkieService: WalkieService? = null
) {
    var isJoiningWifi by remember { mutableStateOf(false) }
    var isJoiningCellular by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    // Keep a single CellularWebSocketClient per composition
    val cellularClient = remember { CellularWebSocketClient() }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DarkBg)
            .padding(16.dp)
    ) {
        // Header
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.Default.ArrowBack, contentDescription = "Back", tint = TextGray)
            }

            Text(
                text = "Connect",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )

            // Spacer for symmetry
            Spacer(modifier = Modifier.size(48.dp))
        }

        Spacer(modifier = Modifier.height(24.dp))

        // Error message
        if (errorMessage != null) {
            Card(
                colors = CardDefaults.cardColors(containerColor = StatusDisconnected.copy(alpha = 0.2f)),
                shape = RoundedCornerShape(8.dp),
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = errorMessage!!,
                    color = StatusDisconnected,
                    fontSize = 14.sp,
                    modifier = Modifier.padding(12.dp)
                )
            }
            Spacer(modifier = Modifier.height(16.dp))
        }

        // ── WiFi Multicast ──
        Card(
            colors = CardDefaults.cardColors(containerColor = Cyan.copy(alpha = 0.12f)),
            shape = RoundedCornerShape(16.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Outlined.Wifi, contentDescription = null, tint = Cyan, modifier = Modifier.size(28.dp))
                    Spacer(modifier = Modifier.width(8.dp))
                    Text("WiFi Multicast", fontSize = 18.sp, fontWeight = FontWeight.Bold, color = Cyan)
                }
                Spacer(modifier = Modifier.height(6.dp))
                Text(
                    "All devices on the same WiFi network can talk.\nWorks with Android, iOS, Windows, Mac.",
                    fontSize = 13.sp, color = TextGray, textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(14.dp))
                Button(
                    onClick = {
                        isJoiningWifi = true
                        errorMessage = null
                        scope.launch {
                            val success = withContext(Dispatchers.IO) {
                                // CRITICAL: Acquire multicast lock BEFORE joining.
                                // Without this, Android's WiFi driver silently drops
                                // all multicast/broadcast UDP packets.
                                walkieService?.acquireMulticastLock()
                                SassyTalkNative.connectWifiMulticast()
                            }
                            if (success) {
                                walkieService?.updateNotification("Radio active — WiFi")
                                onConnected()
                            } else {
                                walkieService?.releaseMulticastLock()
                                errorMessage = "WiFi multicast failed — are you on WiFi?"
                                isJoiningWifi = false
                            }
                        }
                    },
                    enabled = !isJoiningWifi && !isJoiningCellular,
                    shape = RoundedCornerShape(25.dp),
                    colors = ButtonDefaults.buttonColors(containerColor = Cyan, contentColor = DarkBg),
                    modifier = Modifier.fillMaxWidth().height(52.dp)
                ) {
                    if (isJoiningWifi) {
                        RainbowRefreshIndicator(isRefreshing = true, onRefresh = {}, size = 20.dp, strokeWidth = 2.dp)
                        Spacer(modifier = Modifier.width(8.dp))
                        Text("Joining...", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                    } else {
                        Text("Join WiFi Channel", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        // ── Cellular Relay ──
        Card(
            colors = CardDefaults.cardColors(containerColor = Orange.copy(alpha = 0.12f)),
            shape = RoundedCornerShape(16.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Outlined.CellTower, contentDescription = null, tint = Orange, modifier = Modifier.size(28.dp))
                    Spacer(modifier = Modifier.width(8.dp))
                    Text("Cellular Relay", fontSize = 18.sp, fontWeight = FontWeight.Bold, color = Orange)
                }
                Spacer(modifier = Modifier.height(6.dp))
                Text(
                    "Talk over the internet — works anywhere with\ncellular or WiFi data. No shared network needed.",
                    fontSize = 13.sp, color = TextGray, textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(14.dp))
                Button(
                    onClick = {
                        isJoiningCellular = true
                        errorMessage = null
                        scope.launch {
                            val success = withContext(Dispatchers.IO) {
                                connectCellular(cellularClient)
                            }
                            if (success) {
                                walkieService?.updateNotification("Radio active — Cellular")
                                onConnected()
                            } else {
                                errorMessage = "Cellular relay failed — check your internet connection"
                                isJoiningCellular = false
                            }
                        }
                    },
                    enabled = !isJoiningWifi && !isJoiningCellular,
                    shape = RoundedCornerShape(25.dp),
                    colors = ButtonDefaults.buttonColors(containerColor = Orange, contentColor = DarkBg),
                    modifier = Modifier.fillMaxWidth().height(52.dp)
                ) {
                    if (isJoiningCellular) {
                        RainbowRefreshIndicator(isRefreshing = true, onRefresh = {}, size = 20.dp, strokeWidth = 2.dp)
                        Spacer(modifier = Modifier.width(8.dp))
                        Text("Connecting...", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                    } else {
                        Text("Join Cellular Relay", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                    }
                }
            }
        }

        Spacer(modifier = Modifier.weight(1f))

        // Session info
        val isAuth = SassyTalkNative.isAuthenticated()
        Text(
            text = if (isAuth) "Authenticated session active" else "No active session",
            fontSize = 11.sp,
            color = if (isAuth) Green else TextMuted,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth()
        )
    }
}

/**
 * Connect to the cellular relay.
 * Sets the room ID from session, then opens a WebSocket.
 * Waits briefly for the async connection to complete.
 */
private suspend fun connectCellular(client: CellularWebSocketClient): Boolean {
    // Get session_id to use as room ID
    val sessionId = SassyTalkNative.getSessionId()
    if (sessionId.isNullOrBlank()) {
        return false
    }

    // Tell Rust the room ID
    SassyTalkNative.cellularSetRoom(sessionId)

    // Start WebSocket connection (async — onOpen callback will notify Rust)
    client.connect()

    // Wait up to 5 seconds for connection
    for (i in 0 until 50) {
        if (client.isConnected()) return true
        kotlinx.coroutines.delay(100)
    }

    // Timed out
    client.disconnect()
    return false
}
