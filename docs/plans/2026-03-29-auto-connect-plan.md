<!--
   Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
   Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
   CodeMark: SCLLC1-sassytalkie-3ECQLMNNDAW6
-->
# Auto-Connect & UX Simplification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove manual protocol selection, auto-detect and connect to the best available transport, shrink QR code so Continue button is always visible.

**Architecture:** New `AutoConnectManager` class encapsulates smart transport detection. AppNavigation skips DevicePicker, going Auth -> Main directly. MainScreen shows inline connection progress with PTT disabled until connected.

**Tech Stack:** Kotlin, Jetpack Compose, Android ConnectivityManager, existing SassyTalkNative JNI bridge, existing CellularWebSocketClient.

---

### Task 1: Shrink QR Code in QRAuthScreen

**Files:**
- Modify: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/QRAuthScreen.kt:256-303`

**Step 1: Reduce QR code display size and spacing**

In `QRAuthScreen.kt`, in the `ShowQRTab` composable, change three values:

Line 265: `.size(280.dp)` -> `.size(200.dp)`
Line 266: `.padding(16.dp)` -> `.padding(12.dp)`
Line 279: `Spacer(modifier = Modifier.height(20.dp))` -> `Spacer(modifier = Modifier.height(12.dp))`

The QR bitmap is still generated at 600x600 pixels internally (line 393 `generateQRBitmap`), so scanning quality is unaffected.

**Step 2: Verify visually**

Build and run the app. On the QR screen, the Continue button should now be visible without scrolling on a 5" screen. The QR code should still scan reliably.

**Step 3: Commit**

```bash
git add android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/QRAuthScreen.kt
git commit -m "fix: shrink QR code to 200dp so Continue button is always visible"
```

---

### Task 2: Create AutoConnectManager

**Files:**
- Create: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/AutoConnectManager.kt`

**Step 1: Write AutoConnectManager class**

```kotlin
package com.sassyconsulting.sassytalkie.ui

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.CellularWebSocketClient
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.WalkieService

enum class ConnectState {
    IDLE,
    DETECTING,
    TRYING_WIFI,
    TRYING_CELLULAR,
    CONNECTED,
    FAILED
}

class AutoConnectManager(private val context: Context) {

    companion object {
        private const val TAG = "AutoConnect"
        private const val WIFI_PEER_TIMEOUT_MS = 3000L
        private const val CELLULAR_TIMEOUT_MS = 5000L
    }

    private val _state = MutableStateFlow(ConnectState.IDLE)
    val state: StateFlow<ConnectState> = _state

    private val _statusText = MutableStateFlow("")
    val statusText: StateFlow<String> = _statusText

    private var cellularClient: CellularWebSocketClient? = null

    private fun hasWifi(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = cm.activeNetwork ?: return false
        val caps = cm.getNetworkCapabilities(network) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
    }

    suspend fun autoConnect(walkieService: WalkieService?): Boolean {
        _state.value = ConnectState.DETECTING
        _statusText.value = "Detecting network..."

        return withContext(Dispatchers.IO) {
            if (hasWifi()) {
                // Try WiFi multicast first
                _state.value = ConnectState.TRYING_WIFI
                _statusText.value = "Trying WiFi..."
                Log.d(TAG, "WiFi detected, trying multicast")

                walkieService?.acquireMulticastLock()
                val wifiOk = SassyTalkNative.connectWifiMulticast()

                if (wifiOk) {
                    Log.d(TAG, "WiFi multicast connected")
                    walkieService?.updateNotification("Radio active — WiFi")
                    _state.value = ConnectState.CONNECTED
                    _statusText.value = "Connected via WiFi"
                    return@withContext true
                }

                // WiFi multicast failed, release lock and fall through
                Log.d(TAG, "WiFi multicast failed, falling back to cellular")
                walkieService?.releaseMulticastLock()
            }

            // Try cellular relay
            _state.value = ConnectState.TRYING_CELLULAR
            _statusText.value = "Connecting via relay..."
            Log.d(TAG, "Trying cellular relay")

            val sessionId = SassyTalkNative.getSessionId()
            if (sessionId.isNullOrBlank()) {
                Log.e(TAG, "No session ID for cellular")
                _state.value = ConnectState.FAILED
                _statusText.value = "Connection failed — no session"
                return@withContext false
            }

            SassyTalkNative.cellularSetRoom(sessionId)
            val client = CellularWebSocketClient()
            cellularClient = client
            client.connect()

            // Wait for WebSocket connection
            val iterations = (CELLULAR_TIMEOUT_MS / 100).toInt()
            for (i in 0 until iterations) {
                if (client.isConnected()) {
                    Log.d(TAG, "Cellular relay connected")
                    walkieService?.updateNotification("Radio active — Cellular")
                    _state.value = ConnectState.CONNECTED
                    _statusText.value = "Connected via Cellular"
                    return@withContext true
                }
                kotlinx.coroutines.delay(100)
            }

            // Both failed
            client.disconnect()
            _state.value = ConnectState.FAILED
            _statusText.value = "Connection failed"
            Log.e(TAG, "All transports failed")
            return@withContext false
        }
    }

    fun reset() {
        _state.value = ConnectState.IDLE
        _statusText.value = ""
    }

    fun disconnect() {
        cellularClient?.disconnect()
        cellularClient = null
        _state.value = ConnectState.IDLE
    }
}
```

**Step 2: Commit**

```bash
git add android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/AutoConnectManager.kt
git commit -m "feat: add AutoConnectManager for smart transport detection"
```

---

### Task 3: Update AppNavigation to skip DevicePicker

**Files:**
- Modify: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/AppNavigation.kt`

**Step 1: Remove DevicePicker from Screen enum**

At line 27, remove `DevicePicker,` from the enum:

```kotlin
enum class Screen {
    Auth,
    Main,
    Users,
}
```

**Step 2: Update the BackHandler**

Replace lines 165-176 with:

```kotlin
    BackHandler(enabled = currentScreen != Screen.Auth) {
        when (currentScreen) {
            Screen.Main -> {
                walkieService?.releaseMulticastLock()
                currentScreen = Screen.Auth
            }
            Screen.Users -> currentScreen = Screen.Main
            else -> {}
        }
    }
```

**Step 3: Update the screen routing**

Replace lines 178-198 with:

```kotlin
    when (currentScreen) {
        Screen.Auth -> QRAuthScreen(
            onAuthenticated = { currentScreen = Screen.Main }
        )
        Screen.Main -> MainScreen(
            onDisconnect = {
                walkieService?.releaseMulticastLock()
                currentScreen = Screen.Auth
            },
            onShowUsers = { currentScreen = Screen.Users },
            walkieService = walkieService
        )
        Screen.Users -> UsersScreen(
            onBack = { currentScreen = Screen.Main }
        )
    }
```

Key changes:
- Auth now goes directly to Main (not DevicePicker)
- Main disconnect goes back to Auth (not DevicePicker)
- DevicePicker screen entry is removed entirely

**Step 4: Commit**

```bash
git add android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/AppNavigation.kt
git commit -m "feat: remove DevicePicker from navigation, Auth goes directly to Main"
```

---

### Task 4: Update MainScreen with auto-connect and connection states

**Files:**
- Modify: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/MainScreen.kt`

**Step 1: Add imports**

Add these imports at the top of MainScreen.kt (after line 30):

```kotlin
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.launch as coroutineLaunch
```

**Step 2: Add auto-connect state to MainScreen composable**

After line 41 (`val scope = rememberCoroutineScope()`), add:

```kotlin
    val context = LocalContext.current
    val autoConnect = remember { AutoConnectManager(context) }
    val connectState by autoConnect.state.collectAsState()
    val connectStatusText by autoConnect.statusText.collectAsState()

    // Auto-connect on first composition
    LaunchedEffect(Unit) {
        autoConnect.autoConnect(walkieService)
    }
```

Also add this import at the top:
```kotlin
import androidx.compose.runtime.collectAsState
```

**Step 3: Replace connection status display**

Replace lines 92-111 (the connection status Row) with:

```kotlin
        // Connection status
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
            modifier = Modifier.fillMaxWidth()
        ) {
            when (connectState) {
                ConnectState.CONNECTED -> {
                    Box(
                        modifier = Modifier
                            .size(10.dp)
                            .clip(CircleShape)
                            .background(StatusConnected)
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = "Connected via ${SassyTalkNative.getTransportName()}",
                        fontSize = 13.sp,
                        color = TextGray
                    )
                }
                ConnectState.FAILED -> {
                    Box(
                        modifier = Modifier
                            .size(10.dp)
                            .clip(CircleShape)
                            .background(StatusDisconnected)
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = connectStatusText,
                        fontSize = 13.sp,
                        color = Color(0xFFFF6B6B)
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    TextButton(onClick = {
                        scope.launch { autoConnect.autoConnect(walkieService) }
                    }) {
                        Text("Retry", fontSize = 12.sp, color = Cyan)
                    }
                }
                else -> {
                    CircularProgressIndicator(
                        modifier = Modifier.size(14.dp),
                        strokeWidth = 2.dp,
                        color = Cyan
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = connectStatusText,
                        fontSize = 13.sp,
                        color = TextGray
                    )
                }
            }
        }
```

**Step 4: Disable PTT when not connected**

Replace lines 146-166 (the PTTButton call and its callbacks) with:

```kotlin
        val pttEnabled = connectState == ConnectState.CONNECTED

        PTTButton(
            isTransmitting = isTransmitting,
            pulseScale = if (isTransmitting) pulseScale else 1f,
            onPressStart = {
                if (!pttEnabled) return@PTTButton
                if (!SassyTalkNative.isEncrypted()) {
                    showEncryptionWarning = true
                } else {
                    showEncryptionWarning = false
                    isTransmitting = true
                    SassyTalkNative.pttStart()
                    walkieService?.updateNotification("Transmitting on CH $currentChannel")
                }
            },
            onPressEnd = {
                if (isTransmitting) {
                    isTransmitting = false
                    SassyTalkNative.pttStop()
                    walkieService?.updateNotification("Radio active — ${SassyTalkNative.getTransportName()}")
                }
            }
        )
```

**Step 5: Update PTTButton to show disabled state**

In the `PTTButton` composable (around line 262), add `enabled: Boolean = true` parameter and dim the button when disabled. Change the function signature:

```kotlin
@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun PTTButton(
    isTransmitting: Boolean,
    pulseScale: Float,
    enabled: Boolean = true,
    onPressStart: () -> Unit,
    onPressEnd: () -> Unit
)
```

Then update the PTTButton call to pass `enabled = pttEnabled`:

```kotlin
        PTTButton(
            isTransmitting = isTransmitting,
            pulseScale = if (isTransmitting) pulseScale else 1f,
            enabled = pttEnabled,
            onPressStart = { ... },
            onPressEnd = { ... }
        )
```

In the PTTButton body, wrap the `pointerInteropFilter` callback with an enabled check, and reduce the alpha of the outer circle when disabled:

Change the outer Box modifier alpha:
```kotlin
            .background(
                Brush.radialGradient(
                    colors = listOf(
                        if (isTransmitting) Cyan.copy(alpha = 0.3f) else Cyan.copy(alpha = if (enabled) 0.15f else 0.05f),
                        Color.Transparent
                    )
                )
            )
```

In the `pointerInteropFilter` block, add early return:
```kotlin
            .pointerInteropFilter { event ->
                if (!enabled) return@pointerInteropFilter false
                when (event.action) {
```

**Step 6: Update onDisconnect to clean up AutoConnectManager**

The `onDisconnect` callback in MainScreen should call `autoConnect.disconnect()`. But since `onDisconnect` navigates away, the composable will be destroyed and cleaned up. Add a `DisposableEffect` after the `LaunchedEffect`:

```kotlin
    DisposableEffect(Unit) {
        onDispose {
            autoConnect.disconnect()
        }
    }
```

Add import:
```kotlin
import androidx.compose.runtime.DisposableEffect
```

**Step 7: Commit**

```bash
git add android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/MainScreen.kt
git commit -m "feat: add auto-connect with inline status and PTT disable until connected"
```

---

### Task 5: Build and verify

**Step 1: Build the APK**

```bash
cd android-app
./gradlew assembleDebug
```

Expected: BUILD SUCCESSFUL. If compile errors, fix them (most likely missing imports).

**Step 2: Test on device**

1. Install debug APK
2. Open app, grant permissions
3. Generate or use existing QR session
4. Tap Continue -> should go directly to MainScreen
5. MainScreen should show "Detecting network..." -> "Trying WiFi..." -> "Connected via WiFi" (if on WiFi)
6. PTT button should be disabled (dimmed) during connection, enabled once connected
7. If not on WiFi, should fall through to cellular and connect
8. Verify QR code fits on screen with Continue button visible

**Step 3: Test failure path**

1. Turn off WiFi and mobile data
2. Go through auth flow
3. MainScreen should show "Connection failed" with Retry button
4. Turn WiFi back on, tap Retry
5. Should connect successfully

**Step 4: Final commit**

```bash
git add -A
git commit -m "chore: auto-connect UX complete — removed DevicePicker, smart transport detection"
```

---

### Task 6: Clean up DevicePickerScreen (optional)

**Files:**
- Delete or archive: `android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/DevicePickerScreen.kt`

**Step 1: Remove DevicePickerScreen.kt**

The file is no longer referenced anywhere. Delete it:

```bash
git rm android-app/app/src/main/java/com/sassyconsulting/sassytalkie/ui/DevicePickerScreen.kt
git commit -m "chore: remove unused DevicePickerScreen"
```
