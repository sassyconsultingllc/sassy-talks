// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-PBJ2CFYUT6OQ
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
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume
import com.sassyconsulting.sassytalkie.MainActivity
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.SessionShareLink
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.license.Entitlements
import com.sassyconsulting.sassytalkie.ui.theme.*
import android.widget.Toast

enum class Screen {
    Profile,
    Gate,
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
    onPipEligibilityChanged: (Boolean) -> Unit = {},
    onRadioUiVisible: (Boolean) -> Unit = {},
) {
    val context = LocalContext.current
    var currentScreen by remember { mutableStateOf(Screen.Auth) }
    // Bumped after a deep-link import so the Auth screen (whose session read is
    // a one-shot remember{}) is recomposed from scratch and shows the just-
    // loaded session instead of the stale pre-import state.
    var authRefreshNonce by remember { mutableStateOf(0) }
    var nativeReady by remember { mutableStateOf(false) }
    var initFailed by remember { mutableStateOf(false) }
    var bleReady by remember { mutableStateOf(false) }
    // Entitlement gate (paywall on Play flavor, license key on direct flavor).
    // Seeded from the encrypted cache for instant startup routing; a silent
    // refresh below reconciles with Play / the license server when online.
    var entitled by remember { mutableStateOf(false) }
    var profileSetState by remember { mutableStateOf(false) }

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
                    color = Teal
                )
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    text = "Sassy-Talk needs microphone and camera\npermissions to function.",
                    fontSize = 14.sp,
                    color = TextGray,
                    textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(6.dp))
                Text(
                    text = "Bluetooth and notifications are optional —\nyou can enable them later in system settings.",
                    fontSize = 12.sp,
                    color = TextMuted,
                    textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(32.dp))
                Button(
                    onClick = onRequestPermissions,
                    colors = ButtonDefaults.buttonColors(containerColor = PrimaryBlue),
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
                // (MODE_IN_COMMUNICATION + loudspeaker override) can obtain
                // AudioManager. Safe to call even if already initialized.
                if (initOk) {
                    try { SassyTalkNative.initContext(context.applicationContext) } catch (_: Throwable) {}
                }
                initOk
            }
            if (success) {
                // Post-update reset, BEFORE restoring anything. An in-place
                // update keeps all prior state while the binary underneath it
                // changed — stale native caches, counters describing a dead
                // process, a relay socket bound to the old room. This is the
                // cleanup the update path never did, which is why
                // "reinstall it" kept working as a fix.
                //
                // Runs here rather than only in AppUpdateReceiver because a
                // force-stopped app never receives MY_PACKAGE_REPLACED, and
                // OEM battery managers force-stop aggressively — the devices
                // most likely to need this are the ones least likely to get
                // the broadcast. Costs one int comparison when it is a no-op.
                withContext(Dispatchers.IO) {
                    com.sassyconsulting.sassytalkie.UpdateReset.runIfUpdated(context)
                }

                // Restore persisted session (survives app restart)
                val sessionRestored = withContext(Dispatchers.IO) {
                    val restored = SassyTalkNative.restoreSession()
                    SassyTalkNative.restoreCohortHistory()
                    restored
                }

                // Determine starting screen: profile setup on first launch
                val prefs = context.getSharedPreferences("sassy_profile", Context.MODE_PRIVATE)
                val profileSet = prefs.getBoolean(KEY_PROFILE_SET, false)
                profileSetState = profileSet
                val savedName = getSavedProfileName(context)

                // Apply saved profile name to native library. The install id
                // MUST go in first: the native sender identity is derived from
                // name + install id, and identical names alone (two defaults,
                // same model) made devices drop each other's audio as
                // self-echo and never show in the roster.
                withContext(Dispatchers.IO) {
                    SassyTalkNative.setInstallId(
                        com.sassyconsulting.sassytalkie.InstallId.get(context)
                    )
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

                // Entitlement gate overrides all of the above routing while
                // locked. EncryptedSharedPreferences read — cheap, still on
                // the IO-adjacent init path.
                entitled = withContext(Dispatchers.IO) {
                    // Trial users are "entitled" for routing purposes: the paywall
                    // waits until 5 sessions have actually had a peer in them.
                    com.sassyconsulting.sassytalkie.license.TrialGate.mayUseRadio(context)
                }
                if (!entitled) {
                    currentScreen = Screen.Gate
                }
                nativeReady = true

                // Initialize the activity bridge for incoming-audio detection + notifications.
                // Activity is session-scoped only — do NOT rehydrate disk history (favorites
                // and speech rows must not linger across days / process restarts).
                TranscriptionBridge.initialize(context)
                TranscriptionBridge.discardPersistedTimeline()
                TranscriptionBridge.clearEntries()
                TranscriptionBridge.setEnabled(true)

                // Restore persisted mic settings (gain + squelch). Defaults
                // are unity/disabled so first-run behavior is unchanged.
                val micPrefs = context.getSharedPreferences("sassy_settings", android.content.Context.MODE_PRIVATE)
                SassyTalkNative.setMicGain(micPrefs.getFloat("mic_gain", 1.0f))
                SassyTalkNative.setSquelchDbfs(micPrefs.getInt("squelch_dbfs", 0))
                // Restore group-mix preference. Default false preserves the
                // classic walkie-talkie behavior for users who never opt in.
                SassyTalkNative.setMixModeEnabled(micPrefs.getBoolean("enable_mix_mode", false))
                // Re-assert the noise-suppression preference (native defaults off).
                SassyTalkNative.setNoiseSuppressionEnabled(micPrefs.getBoolean("noise_suppression", false))
                // Re-assert sealed-sender. The sealed CONTEXT (key + peer id) is
                // (re)populated by restoreSession()/importSessionFromQR above;
                // this just re-arms the blinding toggle the user last chose.
                SassyTalkNative.setSealedSenderEnabled(micPrefs.getBoolean("sealed_sender", false))
                // v2.7.5: restore RX gain, speakerphone routing, jitter
                // buffer preset so the user's prior choices stick across
                // process death without requiring a Settings visit.
                SassyTalkNative.setRxGain(micPrefs.getFloat("rx_gain", 1.0f))
                SassyTalkNative.setSpeakerphonePreference(micPrefs.getBoolean("speakerphone_on", true))
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
            // Show the auth screen only WHILE the invite is being decrypted and
            // imported — it is the loading state, not the destination. On
            // success we go straight to the radio (below): tapping an invite is
            // already an explicit "join this" gesture, so asking the user to
            // then find and press Continue is a second confirmation of a
            // decision they just made. (Locked builds stay on the gate.)
            if (entitled) currentScreen = Screen.Auth
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
                        // Go straight onto the radio. Tapping an invite link is
                        // the join gesture; parking on Auth so the user could
                        // press Continue made every invite a two-step join and
                        // read as "the link didn't work" — the toast already
                        // names the host and channel, so nothing is hidden.
                        //
                        // autoConnect.disconnect() mirrors what the Continue
                        // button does: it tears the relay client down so
                        // MainScreen's auto-connect re-runs against the newly
                        // imported session_id instead of the previous room.
                        // Without it the joiner sits on the radio screen bound
                        // to the wrong room — connected, and silent.
                        //
                        // Invite links still don't bypass the entitlement gate:
                        // a locked build stays on the gate.
                        authRefreshNonce++
                        if (entitled) {
                            autoConnect.disconnect()
                            currentScreen = Screen.Main
                        } else {
                            currentScreen = Screen.Gate
                        }
                    } else {
                        Toast.makeText(
                            context,
                            "Invite was decrypted but session import failed",
                            Toast.LENGTH_LONG,
                        ).show()
                    }
                }
                is SessionShareLink.Result.Err -> {
                    val hint = if (result.message.contains("fragment", ignoreCase = true)) {
                        "Copy the full invite link and paste it in Authenticate → Enter Code"
                    } else {
                        result.message
                    }
                    Toast.makeText(context, hint, Toast.LENGTH_LONG).show()
                    // App opened but key was stripped — land on Auth so user can paste.
                    if (result.message.contains("fragment", ignoreCase = true)) {
                        currentScreen = Screen.Auth
                    }
                }
            }
            onShareConsumed()
        }
    }

    // Silent entitlement reconciliation once per launch: restores a Play
    // purchase after reinstall, slides the direct-license receipt window
    // forward, and drops the entitlement after a refund/revocation. Runs
    // after native init so it never delays startup.
    LaunchedEffect(nativeReady) {
        if (!nativeReady) return@LaunchedEffect
        val ok = suspendCancellableCoroutine { cont ->
            Entitlements.refresh(context) { result -> cont.resume(result) }
        }
        // The server/Play answer alone must NOT decide routing: a trial user
        // legitimately has no entitlement, and taking `ok` at face value here
        // would drop them onto the paywall a beat after launch — silently
        // undoing the trial from a background coroutine.
        val mayUse = ok || com.sassyconsulting.sassytalkie.license.TrialGate.inTrial(context)
        entitled = mayUse
        if (!mayUse) currentScreen = Screen.Gate
    }

    // Wait for both native init and the service binding before starting BLE/RFCOMM.
    // Keyed on `entitled` too: initBleTransport refuses while locked, and v3.1.5
    // latched bleReady=true on that refusal — PttCoordinator then never existed
    // for the whole session and the on-screen PTT went permanently dead. Only
    // latch on actual success; a false return (locked / BT off / no permission)
    // retries when the keys change, and MainScreen re-attempts on mount.
    LaunchedEffect(nativeReady, walkieService, entitled) {
        val service = walkieService
        if (nativeReady && entitled && service != null && !bleReady) {
            bleReady = withContext(Dispatchers.IO) {
                service.initBleTransport()
            }
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
                    CircularProgressIndicator(color = Teal)
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

    LaunchedEffect(currentScreen) {
        onPipEligibilityChanged(currentScreen == Screen.Main)
        onRadioUiVisible(
            currentScreen == Screen.Main ||
                currentScreen == Screen.Users ||
                currentScreen == Screen.Activity,
        )
        // Allow QR screenshots on non-radio screens (release builds only).
        val allowCapture = currentScreen != Screen.Main
        (context as? MainActivity)?.setScreenshotsAllowed(allowCapture)
    }

    // Hardware back button support
    BackHandler(
        enabled = currentScreen != Screen.Auth &&
            currentScreen != Screen.Profile &&
            currentScreen != Screen.Gate,
    ) {
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
        Screen.Gate -> Entitlements.GateScreen(
            onUnlocked = {
                entitled = true
                currentScreen = if (!profileSetState) Screen.Profile else Screen.Auth
            },
        )
        Screen.Profile -> ProfileScreen(
            onDone = { currentScreen = Screen.Auth },
            showBackButton = false
        )
        Screen.Auth -> key(authRefreshNonce) { QRAuthScreen(
            onAuthenticated = {
                // Tear down the relay WS so MainScreen's auto-connect re-runs
                // with the just-imported session_id. Without this, importing
                // a new session leaves the WS bound to the previous room.
                autoConnect.disconnect()
                TranscriptionBridge.clearEntries()
                currentScreen = Screen.Main
            },
            onSessionMutated = {
                // Force-reconnect on EVERY session change (host generate,
                // joiner scan, joiner paste). Host doesn't go through
                // onAuthenticated until they tap Continue, so without this
                // their WS would stay on the old room while joiners try to
                // attach to the new one shown in the QR.
                walkieService?.forceCellularReconnect()
                TranscriptionBridge.clearEntries()
            },
        ) }
        Screen.Main -> MainScreen(
            onDisconnect = {
                autoConnect.disconnect()
                walkieService?.releaseMulticastLock()
                TranscriptionBridge.clearEntries()
                currentScreen = Screen.Auth
            },
            onShowUsers = { currentScreen = Screen.Users },
            onShowActivity = { currentScreen = Screen.Activity },
            onShowAbout = { currentScreen = Screen.About },
            onShowSettings = { currentScreen = Screen.Settings },
            onEndSession = {
                walkieService?.pttCoordinator?.onPttReleased()
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
            walkieService = walkieService,
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
