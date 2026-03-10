package com.sassyconsulting.sassytalkie.ui

import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.ui.theme.*

enum class Screen {
    Auth,
    DevicePicker,
    Main,
    Users,
}

/**
 * Root navigation composable.
 *
 * Startup sequence:
 *   1. Wait for [permissionsGranted] = true  (MainActivity handles the request)
 *   2. Initialize native Rust library on IO thread
 *   3. Navigate to Auth screen
 *
 * This ensures AudioRecord/AudioTrack JNI calls never happen before
 * RECORD_AUDIO is granted, eliminating the permission race condition.
 */
@Composable
fun AppNavigation(
    permissionsGranted: Boolean,
    walkieService: WalkieService?,
    onRequestPermissions: () -> Unit
) {
    var currentScreen by remember { mutableStateOf(Screen.Auth) }
    var nativeReady by remember { mutableStateOf(false) }
    var initFailed by remember { mutableStateOf(false) }
    var bleReady by remember { mutableStateOf(false) }

    // ── Phase 1: Wait for permissions ──
    if (!permissionsGranted) {
        Box(
            modifier = Modifier.fillMaxSize().background(DarkBg),
            contentAlignment = Alignment.Center
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    text = "🎤",
                    fontSize = 64.sp
                )
                Spacer(modifier = Modifier.height(24.dp))
                Text(
                    text = "Permissions Required",
                    fontSize = 24.sp,
                    fontWeight = FontWeight.Bold,
                    color = Orange
                )
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    text = "Sassy-Talk needs microphone and camera\npermissions to function.",
                    fontSize = 14.sp,
                    color = TextGray,
                    textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(32.dp))
                Button(
                    onClick = onRequestPermissions,
                    colors = ButtonDefaults.buttonColors(containerColor = Orange),
                    shape = RoundedCornerShape(25.dp),
                    modifier = Modifier.height(52.dp).width(220.dp)
                ) {
                    Text("Grant Permissions", fontSize = 16.sp)
                }
            }
        }
        return
    }

    // ── Phase 2: Initialize native library (only after permissions granted) ──
    LaunchedEffect(permissionsGranted) {
        if (permissionsGranted && !nativeReady) {
            val success = withContext(Dispatchers.IO) {
                if (!SassyTalkNative.isInitialized()) {
                    SassyTalkNative.init()
                } else {
                    true
                }
            }
            if (success) {
                // Set device name to the Android model
                withContext(Dispatchers.IO) {
                    SassyTalkNative.setDeviceName(Build.MODEL)
                }
                nativeReady = true
            } else {
                initFailed = true
            }
        }
    }

    // Wait for both native init and the service binding before starting BLE/RFCOMM
    LaunchedEffect(nativeReady, walkieService) {
        val service = walkieService
        if (nativeReady && service != null && !bleReady) {
            withContext(Dispatchers.IO) {
                service.initBleTransport()
            }
            bleReady = true
        }
    }

    if (!nativeReady) {
        Box(
            modifier = Modifier.fillMaxSize().background(DarkBg),
            contentAlignment = Alignment.Center
        ) {
            if (initFailed) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        text = "Initialization Failed",
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold,
                        color = StatusDisconnected
                    )
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(
                        text = "The audio engine could not start.\nPlease restart the app.",
                        fontSize = 14.sp,
                        color = TextGray,
                        textAlign = TextAlign.Center
                    )
                }
            } else {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    CircularProgressIndicator(color = Orange)
                    Spacer(modifier = Modifier.height(16.dp))
                    Text(
                        text = "Starting radio...",
                        fontSize = 14.sp,
                        color = TextGray
                    )
                }
            }
        }
        return
    }

    // ── Phase 3: Main navigation ──

    // Hardware back button support
    BackHandler(enabled = currentScreen != Screen.Auth) {
        when (currentScreen) {
            Screen.DevicePicker -> currentScreen = Screen.Auth
            Screen.Main -> {
                // Release multicast lock when leaving main screen
                walkieService?.releaseMulticastLock()
                currentScreen = Screen.DevicePicker
            }
            Screen.Users -> currentScreen = Screen.Main
            else -> {}
        }
    }

    when (currentScreen) {
        Screen.Auth -> QRAuthScreen(
            onAuthenticated = { currentScreen = Screen.DevicePicker }
        )
        Screen.DevicePicker -> DevicePickerScreen(
            onConnected = { currentScreen = Screen.Main },
            onBack = { currentScreen = Screen.Auth },
            walkieService = walkieService
        )
        Screen.Main -> MainScreen(
            onDisconnect = {
                walkieService?.releaseMulticastLock()
                currentScreen = Screen.DevicePicker
            },
            onShowUsers = { currentScreen = Screen.Users },
            walkieService = walkieService
        )
        Screen.Users -> UsersScreen(
            onBack = { currentScreen = Screen.Main }
        )
    }
}
