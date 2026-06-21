package com.sassyconsulting.sassytalkie.ui

import android.content.Context
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
import androidx.compose.runtime.collectAsState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.SessionShareLink
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.ui.theme.*
import android.widget.Toast

enum class Screen {
    Profile,
    Auth,
    Main,
    Users,
    Activity,
    About,
    Settings,
}

/**
 * Root navigation composable.
 *
 * Startup sequence:
 *   1. Wait for [permissionsGranted] = true  (MainActivity handles the request)
 *   2. Initialize native Rust library on IO thread
 *   3. If first launch (no saved profile): navigate to Profile setup
 *   4. Otherwise: navigate to Auth screen
 *
 * This ensures AudioRecord/AudioTrack JNI calls never happen before
 * RECORD_AUDIO is granted, eliminating the permission race condition.
 */
@Composable
fun AppNavigation(
    permissionsGranted: Boolean,
    walkieService: WalkieService?,
    onRequestPermissions: () -> Unit,
    pendingShareUri: android.net.Uri? = null,
    onShareConsumed: () -> Unit = {},
) {
    val context = LocalContext.current
    var currentScreen by remember { mutableStateOf(Screen.Auth) }
    var nativeReady by remember { mutableStateOf(false) }
    var initFailed by remember { mutableStateOf(false) }
    var bleReady by remember { mutableStateOf(false) }

    // AutoConnectManager lives here (singleton for the session) — NOT in MainScreen
    val autoConnect = remember { AutoConnectManager(context) }

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
                SassyTalkNative.appContext = context.applicationContext
                val initOk = if (!SassyTalkNative.isInitialized()) {
                    SassyTalkNative.init()
                } else {
                    true
                }
                // Hand a Context down to the native layer so audio routing
                // (MODE_IN_COMMUNICATION + speakerphone override) can obtain
                // AudioManager. Safe to call even if already initialized.
                if (initOk) {
                    try { SassyTalkNative.initContext(context.applicationContext) } catch (_: Throwable) {}
                }
                initOk
            }
            if (success) {
                // Restore persisted session (survives app restart)
                val sessionRestored = withContext(Dispatchers.IO) {
                    val restored = SassyTalkNative.restoreSession()
                    SassyTalkNative.restoreCohortHistory()
                    restored
                }

                // Determine starting screen: profile setup on first launch
                val prefs = context.getSharedPreferences("sassy_profile", Context.MODE_PRIVATE)
                val profileSet = prefs.getBoolean(KEY_PROFILE_SET, false)
                val savedName = getSavedProfileName(context)

                // Apply saved profile name to native library
                withContext(Dispatchers.IO) {
                    if (profileSet) {
                        SassyTalkNative.setDeviceName(savedName)
                    } else {
                        SassyTalkNative.setDeviceName(Build.MODEL)
                        currentScreen = Screen.Profile
                    }
                }

                // If session was restored from disk, skip auth and go straight to main
                if (sessionRestored && profileSet) {
                    currentScreen = Screen.Main
                }
                nativeReady = true

                // Initialize the activity bridge for incoming-audio detection + notifications
                TranscriptionBridge.initialize(context)
                // v2.7.2: rehydrate the persisted timeline so the user's
                // "who spoke when" log survives process death. Cheap I/O —
                // a single small JSON file read.
                TranscriptionBridge.restoreEntries()
                TranscriptionBridge.setEnabled(true)

                // Restore persisted mic settings (gain + squelch). Defaults
                // are unity/disabled so first-run behavior is unchanged.
                val micPrefs = context.getSharedPreferences("sassy_settings", android.content.Context.MODE_PRIVATE)
                SassyTalkNative.setMicGain(micPrefs.getFloat("mic_gain", 1.0f))
                SassyTalkNative.setSquelchDbfs(micPrefs.getInt("squelch_dbfs", 0))
                // Restore group-mix preference. Default false preserves the
                // classic walkie-talkie behavior for users who never opt in.
                SassyTalkNative.setMixModeEnabled(micPrefs.getBoolean("enable_mix_mode", false))
                // v2.7.5: restore RX gain, speakerphone routing, jitter
                // buffer preset so the user's prior choices stick across
                // process death without requiring a Settings visit.
                SassyTalkNative.setRxGain(micPrefs.getFloat("rx_gain", 1.0f))
                SassyTalkNative.setSpeakerphone(micPrefs.getBoolean("speakerphone_on", true))
                SassyTalkNative.setJitterPrebufferFrames(micPrefs.getInt("jitter_prebuffer_frames", 5))
            } else {
                initFailed = true
            }
        }
    }

    // Process an incoming share-link URI once the native side is ready.
    // Fires for both initial launch (cold start via VIEW intent) and warm
    // re-entry (onNewIntent → pendingShareUri changes while running).
    LaunchedEffect(nativeReady, pendingShareUri) {
        val uri = pendingShareUri
        if (nativeReady && uri != null) {
            val result = withContext(Dispatchers.IO) {
                SessionShareLink.importFromShareUri(uri)
            }
            when (result) {
                is SessionShareLink.Result.Ok -> {
                    val imported = withContext(Dispatchers.IO) {
                        SassyTalkNative.importSessionFromQR(result.json)
                    }
                    if (imported) {
                        // Force a cellular WS teardown+reconnect so the relay
                        // attaches to the imported session_id's room. Rust
                        // updates the room target internally on import, but
                        // the live WS stays bound to the OLD room until
                        // explicitly reconnected — without this, joiner and
                        // host land on different rooms and never see audio.
                        walkieService?.forceCellularReconnect()
                        // Pull the host's device name + channel out of the
                        // just-imported QR JSON so the user knows WHICH
                        // session they joined. The old "Joined session"
                        // toast left them guessing.
                        val ctx = describeImportedSession(result.json)
                        Toast.makeText(context, ctx, Toast.LENGTH_LONG).show()
                        currentScreen = Screen.Main
                    } else {
                        Toast.makeText(
                            context,
                            "Invite was decrypted but session import failed",
                            Toast.LENGTH_LONG,
                        ).show()
                    }
                }
                is SessionShareLink.Result.Err -> {
                    Toast.makeText(context, result.message, Toast.LENGTH_LONG).show()
                }
            }
            onShareConsumed()
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
    BackHandler(enabled = currentScreen != Screen.Auth && currentScreen != Screen.Profile) {
        when (currentScreen) {
            Screen.Main -> {
                walkieService?.releaseMulticastLock()
                currentScreen = Screen.Auth
            }
            Screen.Users -> currentScreen = Screen.Main
            Screen.Activity -> currentScreen = Screen.Main
            Screen.About -> currentScreen = Screen.Main
            Screen.Settings -> currentScreen = Screen.Main
            else -> {}
        }
    }

    when (currentScreen) {
        Screen.Profile -> ProfileScreen(
            onDone = { currentScreen = Screen.Auth },
            showBackButton = false
        )
        Screen.Auth -> QRAuthScreen(
            onAuthenticated = {
                // Tear down the relay WS so MainScreen's auto-connect re-runs
                // with the just-imported session_id. Without this, importing
                // a new session leaves the WS bound to the previous room.
                autoConnect.disconnect()
                currentScreen = Screen.Main
            },
            onSessionMutated = {
                // Force-reconnect on EVERY session change (host generate,
                // joiner scan, joiner paste). Host doesn't go through
                // onAuthenticated until they tap Continue, so without this
                // their WS would stay on the old room while joiners try to
                // attach to the new one shown in the QR.
                walkieService?.forceCellularReconnect()
            },
        )
        Screen.Main -> MainScreen(
            onDisconnect = {
                autoConnect.disconnect()
                walkieService?.releaseMulticastLock()
                currentScreen = Screen.Auth
            },
            onShowUsers = { currentScreen = Screen.Users },
            onShowActivity = { currentScreen = Screen.Activity },
            onShowAbout = { currentScreen = Screen.About },
            onShowSettings = { currentScreen = Screen.Settings },
            onEndSession = {
                // Clean session kill without restarting app
                SassyTalkNative.pttStop()
                autoConnect.disconnect()
                walkieService?.releaseMulticastLock()
                SassyTalkNative.clearSession() // also clears encrypted per-channel session prefs
                SassyTalkNative.clearAudioCache()
                TranscriptionBridge.clearEntries()
                currentScreen = Screen.Auth
            },
            walkieService = walkieService,
            autoConnect = autoConnect
        )
        Screen.Users -> UsersScreen(
            onBack = { currentScreen = Screen.Main },
            onEditProfile = { currentScreen = Screen.Profile },
            walkieService = walkieService
        )
        Screen.Activity -> TranscriptionFeedScreen(
            entries = TranscriptionBridge.entries.collectAsState().value,
            onBack = { currentScreen = Screen.Main }
        )
        Screen.About -> AboutScreen(
            onBack = { currentScreen = Screen.Main }
        )
        Screen.Settings -> SettingsScreen(
            onBack = { currentScreen = Screen.Main },
            onTransportPrefsChanged = {
                // Tear the active transports down so MainScreen's auto-connect
                // re-evaluates with the new toggle state. The connection
                // status badge will briefly flicker through DETECTING.
                autoConnect.disconnect()
            }
        )
    }
}

/**
 * v2.7.0: extract a human-readable summary from the imported QR JSON so the
 * post-import toast tells the user WHICH session they joined instead of the
 * old generic "Joined session". Reads `device` + `channel` + optional
 * `group_name` fields from the QR payload (set by `nativeGenerateChannelQR`
 * on the host side).
 *
 * Defensive parsing — any field missing falls back to a sensible default,
 * never throws.
 */
private fun describeImportedSession(qrJson: String): String {
    return try {
        val obj = org.json.JSONObject(qrJson)
        val host = obj.optString("device", "").trim().ifEmpty { "host" }
        val channel = obj.optInt("channel", -1)
        val groupName = obj.optString("group_name", "").trim()
        val channelLabel = when {
            groupName.isNotEmpty() -> groupName
            channel >= 0           -> "channel $channel"
            else                   -> "shared channel"
        }
        "Joined $host on $channelLabel"
    } catch (_: Throwable) {
        "Joined session"
    }
}
