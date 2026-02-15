package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*

@Composable
fun DevicePickerScreen(
    onConnected: () -> Unit,
    onBack: () -> Unit
) {
    var devices by remember { mutableStateOf(SassyTalkNative.getPairedDevices()) }
    var isConnecting by remember { mutableStateOf(false) }
    var connectingAddress by remember { mutableStateOf("") }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isListening by remember { mutableStateOf(false) }
    var isRefreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

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
                text = "Select Device",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )

            Box(
                modifier = Modifier.size(48.dp),
                contentAlignment = Alignment.Center
            ) {
                RainbowRefreshIndicator(
                    isRefreshing = isRefreshing,
                    onRefresh = {
                        scope.launch {
                            isRefreshing = true
                            delay(800) // Let the spinner show for a moment
                            devices = SassyTalkNative.getPairedDevices()
                            isRefreshing = false
                        }
                    },
                    size = 28.dp,
                    strokeWidth = 3.dp
                )
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = "Choose a paired Bluetooth device to connect",
            fontSize = 14.sp,
            color = TextGray,
            modifier = Modifier.fillMaxWidth(),
            textAlign = TextAlign.Center
        )

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
            Spacer(modifier = Modifier.height(12.dp))
        }

        // Device list
        if (devices.isEmpty()) {
            Spacer(modifier = Modifier.height(40.dp))
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.fillMaxWidth()
            ) {
                Icon(
                    Icons.Default.BluetoothSearching,
                    contentDescription = null,
                    tint = TextMuted,
                    modifier = Modifier.size(64.dp)
                )
                Spacer(modifier = Modifier.height(16.dp))
                Text("No paired devices found", color = TextGray, fontSize = 16.sp)
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    "Pair devices in Android Bluetooth settings first",
                    color = TextMuted,
                    fontSize = 13.sp,
                    textAlign = TextAlign.Center
                )
            }
        } else {
            LazyColumn(
                verticalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.weight(1f)
            ) {
                items(devices) { device ->
                    val isThisConnecting = isConnecting && connectingAddress == device.address

                    Card(
                        colors = CardDefaults.cardColors(containerColor = CardBg),
                        shape = RoundedCornerShape(12.dp),
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable(enabled = !isConnecting) {
                                isConnecting = true
                                connectingAddress = device.address
                                errorMessage = null

                                val success = SassyTalkNative.connectDevice(device.address)
                                if (success) {
                                    onConnected()
                                } else {
                                    errorMessage = "Failed to connect to ${device.name}"
                                    isConnecting = false
                                }
                            }
                    ) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(16.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            // BT icon
                            Box(
                                modifier = Modifier
                                    .size(48.dp)
                                    .clip(CircleShape)
                                    .background(SurfaceBg),
                                contentAlignment = Alignment.Center
                            ) {
                                Icon(
                                    Icons.Default.Bluetooth,
                                    contentDescription = null,
                                    tint = Cyan,
                                    modifier = Modifier.size(28.dp)
                                )
                            }

                            Spacer(modifier = Modifier.width(16.dp))

                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = device.name,
                                    fontSize = 16.sp,
                                    fontWeight = FontWeight.Medium,
                                    color = TextWhite
                                )
                                Text(
                                    text = device.address,
                                    fontSize = 12.sp,
                                    color = TextMuted
                                )
                            }

                            if (isThisConnecting) {
                                RainbowRefreshIndicator(
                                    isRefreshing = true,
                                    onRefresh = {},
                                    size = 24.dp,
                                    strokeWidth = 2.5.dp
                                )
                            } else {
                                Icon(
                                    Icons.Default.ChevronRight,
                                    contentDescription = null,
                                    tint = TextMuted
                                )
                            }
                        }
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        // Listen mode button
        OutlinedButton(
            onClick = {
                isListening = true
                errorMessage = null
                val success = SassyTalkNative.startListening()
                if (success) {
                    onConnected()
                } else {
                    errorMessage = "Failed to start listening"
                    isListening = false
                }
            },
            enabled = !isConnecting && !isListening,
            shape = RoundedCornerShape(25.dp),
            colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            if (isListening) {
                RainbowRefreshIndicator(
                    isRefreshing = true,
                    onRefresh = {},
                    size = 20.dp,
                    strokeWidth = 2.dp
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text("Listening...", fontSize = 16.sp)
            } else {
                Icon(Icons.Default.Hearing, contentDescription = null)
                Spacer(modifier = Modifier.width(8.dp))
                Text("Listen for Connection", fontSize = 16.sp)
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        // Session info — use status to drive display text
        val sessionJson = SassyTalkNative.getSessionStatus()
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
