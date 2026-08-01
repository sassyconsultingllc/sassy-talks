// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-53CVA2GFR6FK
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.animation.core.*
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInteropFilter
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.SessionShareLink
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.ui.theme.*
import com.sassyconsulting.sassytalkie.ui.util.QrBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.DisposableEffect
import androidx.compose.material3.TextButton
import androidx.compose.material3.CircularProgressIndicator

@Composable
fun MainScreen(
    onDisconnect: () -> Unit = {},
    onShowUsers: () -> Unit = {},
    onShowActivity: () -> Unit = {},
    onShowAbout: () -> Unit = {},
    onShowSettings: () -> Unit = {},
    onEndSession: () -> Unit = {},
    walkieService: WalkieService? = null,
    autoConnect: AutoConnectManager,
) {
    var isTransmitting by remember { mutableStateOf(false) }
    var showMenu by remember { mutableStateOf(false) }
    var showEndSessionDialog by remember { mutableStateOf(false) }
    var currentChannel by remember { mutableIntStateOf(1) }
    var currentSubchannel by remember { mutableIntStateOf(0) } // 0=Main, 1=A, 2=B
    var pttHoldMode by remember { mutableStateOf(false) } // toggle PTT vs hold-to-talk
    var showEncryptionWarning by remember { mutableStateOf(false) }
    var showQrDialog by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // Keep on-device captioning alive while the radio screen is visible.
    DisposableEffect(Unit) {
        com.sassyconsulting.sassytalkie.translate.LiveTranslationBridge.acquireUi()
        onDispose {
            com.sassyconsulting.sassytalkie.translate.LiveTranslationBridge.releaseUi()
        }
    }

    val connectState by autoConnect.state.collectAsState()
    val connectStatusText by autoConnect.statusText.collectAsState()
    val transportAdvisory by autoConnect.transportAdvisory.collectAsState()
    val context = LocalContext.current
    val settingsPrefs = remember {
        context.getSharedPreferences("sassy_settings", android.content.Context.MODE_PRIVATE)
    }
    // Sealed Sender — relay metadata blinding. Local state mirrors prefs so the
    // radio screen can show / one-tap enable without a Settings round-trip.
    var sealedSenderOn by remember {
        mutableStateOf(settingsPrefs.getBoolean("sealed_sender", false))
    }

    // Incoming audio indicator
    val incomingAudio by TranscriptionBridge.incomingAudio.collectAsState()
    val activeSpeaker by TranscriptionBridge.activeSpeakerName.collectAsState()

    // Relay readiness
    val relayReady by autoConnect.relayReady.collectAsState()

    // Per-screen fallback flows. `remember`d once so the same Flow instance
    // is reused across recompositions — otherwise every recompose builds a
    // brand-new MutableStateFlow and triggers a re-subscribe + coroutine
    // churn on collectAsState. (Previously: when pttCoordinator was null,
    // each recompose allocated a new flow → new subscription → cancel old →
    // launch new → repeat at every state change. Wasteful.)
    val falseFallback = remember { kotlinx.coroutines.flow.MutableStateFlow(false) }
    val idleDeliveryFallback = remember {
        kotlinx.coroutines.flow.MutableStateFlow(com.sassyconsulting.sassytalkie.DeliveryState.Idle)
    }

    // Reaching-peer indicator — use reach-failed (watchdog) not inverted reachingPeer
    val peerReachFailed by (walkieService?.pttCoordinator?.peerReachFailed ?: falseFallback).collectAsState()

    // Delivery state indicator (Task 4.3)
    val deliveryState by (walkieService?.pttCoordinator?.deliveredState ?: idleDeliveryFallback).collectAsState()

    // Audio path degraded indicator (Task 7.1)
    val audioPathDegraded by (walkieService?.pttCoordinator?.audioPathDegraded ?: falseFallback).collectAsState()

    // Stale-peer banner (Task 6.2)
    val anyPeerStale by (walkieService?.pttCoordinator?.anyPeerStale ?: falseFallback).collectAsState()

    // RFCOMM still dialing after BLE sighting
    val linkingBluetooth by (walkieService?.pttCoordinator?.linkingBluetooth ?: falseFallback).collectAsState()

    // Talk-over indicator (Task 6.2)
    val peerSpeaking by (walkieService?.pttCoordinator?.peerSpeaking ?: falseFallback).collectAsState()

    // Half-duplex: while WE transmit, hard-mute incoming RX playback so the
    // remote stream isn't played out the speaker into our hot mic (acoustic
    // feedback) — radio convention is you don't hear others while keyed. This is
    // an absolute cut (native rx_muted), not a duck, and it preserves the user's
    // configured RX gain. We intentionally do NOT block transmitting while a peer
    // speaks (this app has an emergency path — barge-in must always be possible).
    LaunchedEffect(isTransmitting) {
        SassyTalkNative.setRxMuted(isTransmitting)
    }

    // v2.7.1: snackbar host for peer join/leave toasts.
    // `scope` is already declared earlier in this composable for the existing
    // disconnect handler — reuse it; don't redeclare.
    val snackbarHost = remember { androidx.compose.material3.SnackbarHostState() }

    // v2.7.1: subscribe to peer join/leave events → snackbar.
    // Only collects while this composable is in the tree; key = pttCoordinator
    // so we re-subscribe if the service rebinds (preserves identity otherwise).
    val coord = walkieService?.pttCoordinator
    val cacheSnap by TranscriptionBridge.cacheSnapshot.collectAsState()
    DisposableEffect(Unit) {
        TranscriptionBridge.acquireCacheUi()
        onDispose { TranscriptionBridge.releaseCacheUi() }
    }

    LaunchedEffect(coord) {
        // Peers are keyed differently across transports (relay:<epoch> vs
        // relay:<clientId> vs BLE MAC), so an unresolved id must NEVER leak into
        // the UI as a raw "relay:1a9618" string. Also coalesce churn: a fast
        // rejoin cancels the pending "left", and identical messages are
        // rate-limited so a flapping peer doesn't spam the snackbar.
        val coalesceMs = 4_000L
        val pendingLeave = HashMap<String, kotlinx.coroutines.Job>()
        val lastShownAt = HashMap<String, Long>()
        suspend fun resolve(peerId: String): String {
            val users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
            return users.associate { it.id to it.name }[peerId]
                ?.takeIf { it.isNotBlank() && it != "null" } ?: "A device"
        }
        suspend fun show(msg: String) {
            val now = System.currentTimeMillis()
            if (now - (lastShownAt[msg] ?: 0L) > coalesceMs) {
                lastShownAt[msg] = now
                snackbarHost.showSnackbar(msg, duration = androidx.compose.material3.SnackbarDuration.Short)
            }
        }
        coord?.peerEvents?.collect { ev ->
            try {
                when (ev) {
                    is com.sassyconsulting.sassytalkie.PeerEvent.Joined -> {
                        pendingLeave.remove(ev.peerId)?.cancel()
                        // Sticky channel: no snackbar on first sighting —
                        // Users list / peer chip already reflect roster.
                    }
                    is com.sassyconsulting.sassytalkie.PeerEvent.Left -> {
                        pendingLeave.remove(ev.peerId)?.cancel()
                        pendingLeave[ev.peerId] = launch {
                            kotlinx.coroutines.delay(coalesceMs)
                            pendingLeave.remove(ev.peerId)
                            // Explicit session removal only (not HB silence).
                            show("${resolve(ev.peerId)} left the session")
                        }
                    }
                }
            } catch (t: Throwable) {
                android.util.Log.w("MainScreen", "peer event UI failed: ${t.message}")
            }
        }
    }

    // Transport advisory refresh only — cache status polled centrally in TranscriptionBridge.
    LaunchedEffect(Unit) {
        var tick = 0
        while (true) {
            if (tick % 5 == 0) autoConnect.refreshTransportAdvisory()
            kotlinx.coroutines.delay(2_000L)
            tick++
        }
    }

    // Auto-connect and set cache to queue mode (cache-first). Re-run when the
    // WalkieService binds — share-link cold starts often reach Main before the
    // service is ready, leaving the relay client unwired.
    LaunchedEffect(walkieService) {
        val service = walkieService ?: return@LaunchedEffect
        // Re-attempt BT init on every Main mount (idempotent, cheap guards).
        // Covers Bluetooth toggled on / permissions granted / entitlement
        // unlocked AFTER the one-shot AppNavigation init ran — before this,
        // a skipped init left pttCoordinator null for the whole session.
        // Must run BEFORE attachWalkieService so the relay client gets wired
        // to the freshly-created coordinator.
        withContext(Dispatchers.IO) { service.initBleTransport() }
        autoConnect.attachWalkieService(service)
        if (connectState != ConnectState.CONNECTED) {
            autoConnect.reset()
            autoConnect.autoConnect(service)
        }
        withContext(Dispatchers.IO) {
            SassyTalkNative.restoreCohortHistory()
            SassyTalkNative.setCacheMode(SassyTalkNative.CACHE_MODE_QUEUE)
        }
    }

    // Pulse only while transmitting — isolated so idle MainScreen is not
    // recomposed every animation frame.
    val pulseScale = rememberTxPulseScale(isTransmitting)

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Brush.radialGradient(listOf(BgMedium, BgDark)))
    ) {
    Column(
        modifier = Modifier
            .fillMaxHeight()
            // Cap + center content on large screens so a 7"+ tablet isn't a
            // stretched phone with big empty side gaps. No-op on phones (<560dp).
            .widthIn(max = 560.dp)
            .fillMaxWidth()
            .align(Alignment.TopCenter)
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // Header with back + users buttons
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = {
                // Plain navigation — deliberately NOT SassyTalkNative.disconnect().
                // That native call wipes transport crypto (TransportManager.
                // disconnect sets crypto=None), so backing out once and
                // Continue-ing back in landed on Main un-encrypted: every PTT
                // press was rejected ("Authenticate via QR first") while the
                // roster still showed peers. Full teardown = End Session menu.
                onDisconnect()
            }) {
                Icon(Icons.Default.ArrowBack, contentDescription = "Leave radio screen", tint = TextGray)
            }

            // Group name (editable per-channel) — defaults to "Sassy-Talk" if no name set
            val channelGroupName = remember(currentChannel) {
                SassyTalkNative.getGroupName(currentChannel)
            }
            // Brand title — blue→teal tactical gradient (see ui/theme/Color.kt).
            Text(
                text = channelGroupName.ifEmpty { "Sassy-Talk" },
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                style = androidx.compose.ui.text.TextStyle(
                    brush = Brush.linearGradient(GradientPrimary)
                ),
                maxLines = 1
            )

            Row {
                // Show QR for current session
                IconButton(onClick = { showQrDialog = true }) {
                    Icon(Icons.Default.QrCode2, contentDescription = "Show QR", tint = Cyan, modifier = Modifier.size(22.dp))
                }

                // Reconnect button
                IconButton(onClick = {
                    scope.launch {
                        autoConnect.reset()
                        autoConnect.autoConnect(walkieService)
                    }
                }) {
                    Icon(Icons.Default.Refresh, contentDescription = "Reconnect", tint = if (connectState != ConnectState.CONNECTED) Orange else Cyan, modifier = Modifier.size(22.dp))
                }

                // Hamburger menu for remaining options
                Box {
                    IconButton(onClick = { showMenu = true }) {
                        Icon(Icons.Default.MoreVert, contentDescription = "Menu", tint = Cyan, modifier = Modifier.size(22.dp))
                    }
                    DropdownMenu(
                        expanded = showMenu,
                        onDismissRequest = { showMenu = false },
                        modifier = Modifier.background(CardBg)
                    ) {
                        DropdownMenuItem(
                            text = { Text("Activity", color = TextWhite) },
                            onClick = { showMenu = false; onShowActivity() },
                            leadingIcon = { Icon(Icons.Default.History, contentDescription = null, tint = Cyan, modifier = Modifier.size(20.dp)) }
                        )
                        DropdownMenuItem(
                            text = { Text("Users", color = TextWhite) },
                            onClick = { showMenu = false; onShowUsers() },
                            leadingIcon = { Icon(Icons.Default.People, contentDescription = null, tint = Cyan, modifier = Modifier.size(20.dp)) }
                        )
                        DropdownMenuItem(
                            text = { Text("Settings", color = TextWhite) },
                            onClick = { showMenu = false; onShowSettings() },
                            leadingIcon = { Icon(Icons.Default.Settings, contentDescription = null, tint = Cyan, modifier = Modifier.size(20.dp)) }
                        )
                        DropdownMenuItem(
                            text = { Text("About", color = TextWhite) },
                            onClick = { showMenu = false; onShowAbout() },
                            leadingIcon = { Icon(Icons.Default.Info, contentDescription = null, tint = Cyan, modifier = Modifier.size(20.dp)) }
                        )
                        Divider(color = SurfaceBg)
                        DropdownMenuItem(
                            text = { Text("End Session", color = Color(0xFFFF6B6B)) },
                            onClick = { showMenu = false; showEndSessionDialog = true },
                            leadingIcon = { Icon(Icons.Default.StopCircle, contentDescription = null, tint = Color(0xFFFF6B6B), modifier = Modifier.size(20.dp)) }
                        )
                    }
                }
            }
        }

        // Connection status — single line for encrypted audio plane
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
            modifier = Modifier.fillMaxWidth()
        ) {
            when (connectState) {
                ConnectState.CONNECTED -> {
                    val planeColor = when (transportAdvisory?.activePlane) {
                        AudioPlane.BOTH_WIFI_RELAY -> StatusConnected
                        AudioPlane.WIFI -> Color(0xFF4CD964)
                        AudioPlane.RELAY -> Orange
                        AudioPlane.BLUETOOTH -> Cyan
                        else -> StatusConnected
                    }
                    Box(
                        modifier = Modifier
                            .size(10.dp)
                            .clip(CircleShape)
                            .background(planeColor)
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    val planeLabel = transportAdvisory?.activeLabel
                        ?: SassyTalkNative.getTransportName()
                    Text(
                        text = planeLabel,
                        fontSize = 13.sp,
                        color = TextGray
                    )
                    if (sealedSenderOn && autoConnect.isUsingRelay()) {
                        Spacer(modifier = Modifier.width(8.dp))
                        Text(
                            text = "· Sealed",
                            fontSize = 12.sp,
                            fontWeight = FontWeight.Medium,
                            color = TealLight
                        )
                    }
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
                        // Match the header Reconnect icon: clear the prior
                        // FAILED state before re-attempting so the two retry
                        // paths behave identically.
                        scope.launch { autoConnect.reset(); autoConnect.autoConnect(walkieService) }
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

        // Transport advisory — only when action may help (not routine OK states)
        val advisory = transportAdvisory
        if (connectState == ConnectState.CONNECTED &&
            advisory != null &&
            advisory.severity != AdvisorySeverity.OK &&
            advisory.message != null
        ) {
            Spacer(modifier = Modifier.height(4.dp))
            val advisoryColor = when (advisory.severity) {
                AdvisorySeverity.DEGRADED -> Color(0xFFFF6B6B)
                AdvisorySeverity.UPGRADE -> Color(0xFFFFB300)
                else -> TextMuted
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                    Icon(
                        when (advisory.severity) {
                            AdvisorySeverity.DEGRADED -> Icons.Default.Warning
                            AdvisorySeverity.UPGRADE -> Icons.Default.Wifi
                            else -> Icons.Default.Info
                        },
                    contentDescription = null,
                    tint = advisoryColor,
                    modifier = Modifier.size(12.dp),
                )
                Spacer(modifier = Modifier.width(4.dp))
                Text(
                    text = advisory.message,
                    fontSize = 11.sp,
                    color = advisoryColor,
                    textAlign = TextAlign.Center,
                    maxLines = 2,
                )
            }
        }

        // Relay readiness indicator
        if (connectState == ConnectState.CONNECTED && !relayReady && autoConnect.isUsingRelay()) {
            Spacer(modifier = Modifier.height(4.dp))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.Center,
                modifier = Modifier.fillMaxWidth()
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(10.dp),
                    strokeWidth = 1.5.dp,
                    color = Orange
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = "Waiting for relay confirmation...",
                    fontSize = 11.sp,
                    color = Orange
                )
            }
        }

        // Relay anonymization nudge — only when the relay plane is active and
        // Sealed Sender is off. One tap enables + reconnects; peers must match.
        if (connectState == ConnectState.CONNECTED &&
            autoConnect.isUsingRelay() &&
            !sealedSenderOn
        ) {
            Spacer(modifier = Modifier.height(4.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "Relay sees room id",
                    fontSize = 11.sp,
                    color = TextMuted,
                )
                TextButton(
                    onClick = {
                        sealedSenderOn = true
                        settingsPrefs.edit().putBoolean("sealed_sender", true).apply()
                        SassyTalkNative.setSealedSenderEnabled(true)
                        scope.launch {
                            snackbarHost.showSnackbar(
                                "Sealed Sender on — peers must enable it too",
                                duration = androidx.compose.material3.SnackbarDuration.Short,
                            )
                            autoConnect.reset()
                            autoConnect.autoConnect(walkieService)
                        }
                    },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                ) {
                    Text("Seal relay", fontSize = 11.sp, color = TealLight)
                }
            }
        }

        // Bluetooth data-plane linking (BLE seen, RFCOMM not yet up)
        if (linkingBluetooth) {
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Linking Bluetooth\u2026",
                fontSize = 11.sp,
                color = TealLight,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )
        }

        // Stale-peer banner (Task 6.2)
        if (anyPeerStale) {
            Spacer(modifier = Modifier.height(4.dp))
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = Color(0xFF3D2E00)),
                shape = RoundedCornerShape(8.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.Center
                ) {
                    Icon(Icons.Default.Warning, contentDescription = null, tint = Color(0xFFFFB300), modifier = Modifier.size(16.dp))
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = "Peer idle \u2014 waking their radio\u2026",
                        fontSize = 13.sp,
                        color = Color(0xFFFFB300),
                        fontWeight = FontWeight.Medium
                    )
                }
            }
        }

        // Incoming audio — hide when cache strip already shows the same speaker.
        if (incomingAudio && !isTransmitting && activeSpeaker.isNotBlank() &&
            cacheSnap.currentSpeakerName != activeSpeaker
        ) {
            Spacer(modifier = Modifier.height(4.dp))
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = Color(0xFF1A3A2A)),
                shape = RoundedCornerShape(8.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.Center
                ) {
                    Icon(Icons.Default.VolumeUp, contentDescription = null, tint = StatusConnected, modifier = Modifier.size(16.dp))
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = "$activeSpeaker is speaking",
                        fontSize = 13.sp,
                        color = StatusConnected,
                        fontWeight = FontWeight.Medium
                    )
                }
            }
        }

        Spacer(modifier = Modifier.height(24.dp))

        // Channel Selector
        ChannelSelector(
            channel = currentChannel,
            onChannelChange = { newChannel ->
                currentChannel = newChannel
                SassyTalkNative.setChannel(newChannel)
            }
        )

        Spacer(modifier = Modifier.height(12.dp))

        // Subchannel selector (Main / A / B)
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically
        ) {
            val subLabels = listOf("Main", "A", "B")
            subLabels.forEachIndexed { idx, label ->
                val selected = idx == currentSubchannel
                TextButton(
                    onClick = {
                        currentSubchannel = idx
                        SassyTalkNative.setSubchannel(idx)
                    },
                    colors = ButtonDefaults.textButtonColors(
                        contentColor = if (selected) Teal else TextMuted
                    )
                ) {
                    Text(
                        text = label,
                        fontSize = 14.sp,
                        fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal
                    )
                }
            }

            Spacer(modifier = Modifier.width(16.dp))

            // PTT Hold toggle
            Text(text = "Hold", color = TextMuted, fontSize = 12.sp)
            Spacer(modifier = Modifier.width(4.dp))
            Switch(
                checked = pttHoldMode,
                onCheckedChange = { pttHoldMode = it },
                colors = SwitchDefaults.colors(
                    checkedThumbColor = Teal,
                    checkedTrackColor = Teal.copy(alpha = 0.3f),
                    uncheckedThumbColor = TextMuted,
                    uncheckedTrackColor = SurfaceBg
                ),
                modifier = Modifier.height(24.dp)
            )
        }

        Spacer(modifier = Modifier.weight(1f))

        // Live translation — slim full-bleed bar above PTT (cancels Column's 16.dp inset).
        LiveTranslationOverlay(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = (-16).dp)
                .padding(bottom = 10.dp),
        )

        // PTT Button
        // Encryption warning snackbar
        if (showEncryptionWarning) {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = Color(0xFF442222)),
                shape = RoundedCornerShape(12.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Icon(Icons.Default.Lock, contentDescription = null, tint = Color(0xFFFF6B6B), modifier = Modifier.size(20.dp))
                    Spacer(modifier = Modifier.width(8.dp))
                    Text("Authenticate via QR first", color = Color(0xFFFF6B6B), fontSize = 13.sp)
                }
            }
            Spacer(modifier = Modifier.height(8.dp))
        }

        val pttEnabled = connectState == ConnectState.CONNECTED

        // Rejected-press feedback \u2014 a press that can't start TX must say WHY
        // on the hint line instead of silently doing nothing (auto-clears).
        var pressRejectedMsg by remember { mutableStateOf<String?>(null) }
        LaunchedEffect(pressRejectedMsg) {
            if (pressRejectedMsg != null) {
                kotlinx.coroutines.delay(3_000L)
                pressRejectedMsg = null
            }
        }

        // Start TX via the coordinator when the BT stack is up, else fall back
        // to the native IP pipeline directly \u2014 the same fallback the
        // notification-shade toggle and hardware PTT already use. Without it,
        // any device where initBleTransport skipped (Bluetooth off, BT
        // permission denied, entitlement not yet cached) had a dead PTT button
        // while the WiFi/relay roster still showed peers.
        val startTx = {
            val ptt = walkieService?.pttCoordinator
            val started = when {
                ptt != null -> ptt.onPttPressed()
                SassyTalkNative.isConnected() -> {
                    SassyTalkNative.pttStart()
                    true
                }
                else -> false
            }
            if (started) {
                isTransmitting = true
                walkieService?.updateNotification("Transmitting on CH $currentChannel")
            } else {
                pressRejectedMsg = walkieService?.pttCoordinator?.pttRejectReason?.value
                    ?: "No route to peers \u2014 check connection"
            }
        }
        val stopTx = {
            isTransmitting = false
            walkieService?.pttCoordinator?.onPttReleased() ?: SassyTalkNative.pttStop()
            walkieService?.updateNotification("Radio active \u2014 ${SassyTalkNative.getTransportName()}")
        }

        PTTButton(
            isTransmitting = isTransmitting,
            pulseScale = pulseScale,
            enabled = pttEnabled,
            dimmed = anyPeerStale,
            onPressStart = {
                if (!SassyTalkNative.isEncrypted()) {
                    showEncryptionWarning = true
                } else if (pttHoldMode && isTransmitting) {
                    stopTx()
                } else {
                    showEncryptionWarning = false
                    startTx()
                }
            },
            onPressEnd = {
                if (!pttHoldMode && isTransmitting) {
                    stopTx()
                }
            },
            onPressRejected = {
                pressRejectedMsg = "Not connected \u2014 tap \u21bb to reconnect"
            }
        )

        // One line of PTT feedback — replaces separate reaching/slow/talk-over cards + status bar.
        val pttHint = when {
            isTransmitting && audioPathDegraded -> "Slow audio path" to Color(0xFFFF9800)
            isTransmitting && peerSpeaking -> "Talk-over detected" to Color(0xFFFF9800)
            isTransmitting && peerReachFailed -> "Not reaching peer" to Color(0xFFFF5252)
            isTransmitting -> "Transmitting on CH $currentChannel" to Orange
            pressRejectedMsg != null -> pressRejectedMsg!! to Color(0xFFFF5252)
            deliveryState == com.sassyconsulting.sassytalkie.DeliveryState.Sending ->
                "Sending…" to Color(0xFFFF9800)
            deliveryState == com.sassyconsulting.sassytalkie.DeliveryState.Delivered ->
                "Delivered" to Color(0xFF4CAF50)
            else -> (if (pttHoldMode) "Ready — tap PTT to talk" else "Ready — hold PTT to talk") to TextGray
        }
        Spacer(modifier = Modifier.height(12.dp))
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(
                imageVector = when {
                    isTransmitting -> Icons.Default.Mic
                    pressRejectedMsg != null -> Icons.Default.Warning
                    deliveryState == com.sassyconsulting.sassytalkie.DeliveryState.Delivered ->
                        Icons.Default.CheckCircle
                    deliveryState == com.sassyconsulting.sassytalkie.DeliveryState.Sending ->
                        Icons.Default.Upload
                    else -> Icons.Default.Hearing
                },
                contentDescription = null,
                tint = pttHint.second,
                modifier = Modifier.size(18.dp),
            )
            Spacer(modifier = Modifier.width(8.dp))
            Text(
                text = pttHint.first,
                fontSize = 14.sp,
                fontWeight = FontWeight.Medium,
                color = pttHint.second,
            )
        }

        Spacer(modifier = Modifier.height(16.dp))

        // Encryption warning only when session is not encrypted
        if (!SassyTalkNative.isEncrypted()) {
            Text(
                text = "⚠ Not encrypted — set up a session to protect audio",
                fontSize = 11.sp,
                color = Color(0xFFFF6B6B),
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth()
            )
        }

        // v2.7.1: cache mini-strip — only visible when cache is non-idle.
        // Compact mirror of TranscriptionFeedScreen's cache bar so the user
        // can see "audio is queued / playing" without leaving the main screen.
        if (cacheSnap.currentSpeakerName != null || cacheSnap.queuedUtterances > 0 ||
            cacheSnap.mode == "Queue" || cacheSnap.mode == "Mix"
        ) {
            val pipColor = when (cacheSnap.mode) {
                "Queue"  -> Orange
                "Mix"    -> Cyan
                "Replay" -> OrangeLight
                else     -> TextMuted
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    modifier = Modifier
                        .size(8.dp)
                        .clip(CircleShape)
                        .background(pipColor),
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = when {
                        // Guard against a blank/"null" speaker name leaking to the
                        // UI as literal "Playing null" (unresolved relay peer name).
                        !cacheSnap.currentSpeakerName.isNullOrBlank() &&
                            cacheSnap.currentSpeakerName != "null" ->
                            "Playing ${cacheSnap.currentSpeakerName}" +
                                (if (cacheSnap.queuedUtterances > 0) " · ${cacheSnap.queuedUtterances} queued" else "")
                        cacheSnap.queuedUtterances > 0 -> "${cacheSnap.queuedUtterances} queued"
                        else -> cacheSnap.mode
                    },
                    fontSize = 11.sp,
                    color = TextMuted,
                )
            }
        }

        Spacer(modifier = Modifier.height(8.dp))
    }

    androidx.compose.material3.SnackbarHost(
        hostState = snackbarHost,
        modifier = Modifier.align(Alignment.BottomCenter)
    )
    }

    // Show QR dialog for current session
    if (showQrDialog) {
        val context = androidx.compose.ui.platform.LocalContext.current
        val sessionId = remember { SassyTalkNative.getSessionId() ?: "" }

        // Read the per-channel session JSON via the encrypted accessor —
        // bypassing it (e.g. by reading MODE_PRIVATE prefs directly) returns
        // empty because writes go to EncryptedSharedPreferences and the
        // cleartext file is purged on launch.
        val sessionJson = remember(currentChannel) {
            SassyTalkNative.getChannelSessionJson(currentChannel)
        }

        AlertDialog(
            onDismissRequest = { showQrDialog = false },
            confirmButton = {
                TextButton(onClick = { showQrDialog = false }) {
                    Text("Close", color = Cyan)
                }
            },
            title = { Text("Session QR", color = Orange) },
            text = {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    if (sessionJson.isNotEmpty()) {
                        val density = LocalDensity.current
                        val qrSizePx = with(density) { 180.dp.roundToPx() }
                        var qrBitmap by remember { mutableStateOf<android.graphics.Bitmap?>(null) }
                        LaunchedEffect(sessionJson, qrSizePx) {
                            qrBitmap = withContext(Dispatchers.Default) {
                                QrBitmap.generate(sessionJson, qrSizePx)
                            }
                        }
                        qrBitmap?.let { bmp ->
                            Card(
                                colors = CardDefaults.cardColors(containerColor = Color.White),
                                shape = RoundedCornerShape(12.dp)
                            ) {
                                Image(
                                    bitmap = bmp.asImageBitmap(),
                                    contentDescription = "Session QR",
                                    modifier = Modifier.size(180.dp).padding(8.dp)
                                )
                            }
                        }
                        Spacer(modifier = Modifier.height(8.dp))
                        Text("Scan to join CH $currentChannel", color = TextGray, fontSize = 13.sp)
                        if (sessionId.isNotEmpty()) {
                            Text(
                                "Room ${sessionId.take(8)} · peer must match this id",
                                color = Cyan,
                                fontSize = 12.sp,
                            )
                        }
                        Text(
                            "Copy/Share mint one-time links for this room (not a new session)",
                            color = TextMuted,
                            fontSize = 11.sp,
                        )

                        // Invite-link actions — same encrypted one-shot link the
                        // Auth screen offers. Without these, minting an invite
                        // from INSIDE a session meant backing out to the Auth
                        // screen, which is where "how do I share this?" died.
                        Spacer(modifier = Modifier.height(10.dp))
                        var linking by remember { mutableStateOf(false) }
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            OutlinedButton(
                                enabled = !linking,
                                onClick = {
                                    linking = true
                                    scope.launch {
                                        val result = withContext(Dispatchers.IO) {
                                            SessionShareLink.createShare(sessionJson)
                                        }
                                        linking = false
                                        when (result) {
                                            is SessionShareLink.Result.Ok -> {
                                                val cm = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE)
                                                    as android.content.ClipboardManager
                                                cm.setPrimaryClip(
                                                    android.content.ClipData.newPlainText(
                                                        "SassyTalk Invite", result.httpsUrl,
                                                    ),
                                                )
                                                android.widget.Toast.makeText(
                                                    context,
                                                    "Invite copied — one-time link for this room",
                                                    android.widget.Toast.LENGTH_LONG,
                                                ).show()
                                            }
                                            is SessionShareLink.Result.Err -> {
                                                android.widget.Toast.makeText(
                                                    context, "Copy failed: ${result.message}",
                                                    android.widget.Toast.LENGTH_LONG,
                                                ).show()
                                            }
                                        }
                                    }
                                },
                                shape = RoundedCornerShape(25.dp),
                                colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
                                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                                modifier = Modifier.weight(1f).height(36.dp)
                            ) {
                                Icon(Icons.Default.ContentCopy, contentDescription = null, modifier = Modifier.size(16.dp))
                                Spacer(modifier = Modifier.width(4.dp))
                                Text(if (linking) "Linking…" else "Copy Link", fontSize = 12.sp)
                            }
                            OutlinedButton(
                                enabled = !linking,
                                onClick = {
                                    linking = true
                                    scope.launch {
                                        val result = withContext(Dispatchers.IO) {
                                            SessionShareLink.createShare(sessionJson)
                                        }
                                        linking = false
                                        when (result) {
                                            is SessionShareLink.Result.Ok -> {
                                                val intent = android.content.Intent(android.content.Intent.ACTION_SEND).apply {
                                                    type = "text/plain"
                                                    putExtra(
                                                        android.content.Intent.EXTRA_TEXT,
                                                        "Join my SassyTalk session:\n${result.httpsUrl}\n\n" +
                                                            "One-time encrypted invite, expires shortly. If it opens a " +
                                                            "web page, tap \"Open in SassyTalk\" there — or paste the " +
                                                            "link in SassyTalk → Authenticate → Enter Code.",
                                                    )
                                                    putExtra(android.content.Intent.EXTRA_SUBJECT, "SassyTalk invite")
                                                }
                                                context.startActivity(
                                                    android.content.Intent.createChooser(intent, "Share invite link"),
                                                )
                                            }
                                            is SessionShareLink.Result.Err -> {
                                                android.widget.Toast.makeText(
                                                    context, "Share failed: ${result.message}",
                                                    android.widget.Toast.LENGTH_LONG,
                                                ).show()
                                            }
                                        }
                                    }
                                },
                                shape = RoundedCornerShape(25.dp),
                                colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
                                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                                modifier = Modifier.weight(1f).height(36.dp)
                            ) {
                                Icon(Icons.Default.Share, contentDescription = null, modifier = Modifier.size(16.dp))
                                Spacer(modifier = Modifier.width(4.dp))
                                Text("Share Link", fontSize = 12.sp)
                            }
                        }
                    } else {
                        Text("No active session for this channel", color = TextMuted)
                    }
                }
            },
            containerColor = CardBg
        )
    }

    // End Session confirmation dialog
    if (showEndSessionDialog) {
        AlertDialog(
            onDismissRequest = { showEndSessionDialog = false },
            confirmButton = {
                TextButton(onClick = {
                    showEndSessionDialog = false
                    onEndSession()
                }) {
                    Text("End Session", color = Color(0xFFFF6B6B))
                }
            },
            dismissButton = {
                TextButton(onClick = { showEndSessionDialog = false }) {
                    Text("Cancel", color = Cyan)
                }
            },
            title = { Text("End Session?", color = Orange) },
            text = {
                Text(
                    "This will kill all active encrypted sessions and disconnect all transports. You will need to re-authenticate via QR to resume.",
                    color = TextGray,
                    fontSize = 14.sp
                )
            },
            containerColor = CardBg
        )
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

            Column(horizontalAlignment = Alignment.CenterHorizontally) {
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
                    color = TealLight
                )
            }

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

/** Pulse scale that only animates while [transmitting]; idle stays at 1f with no infinite clock. */
@Composable
private fun rememberTxPulseScale(transmitting: Boolean): Float {
    val scale = remember { Animatable(1f) }
    LaunchedEffect(transmitting) {
        if (transmitting) {
            while (true) {
                scale.animateTo(1.15f, tween(600, easing = EaseInOut))
                scale.animateTo(1f, tween(600, easing = EaseInOut))
            }
        } else if (scale.value != 1f) {
            scale.snapTo(1f)
        }
    }
    return scale.value
}

/**
 * PTT Button using pointerInteropFilter for reliable press/release detection.
 *
 * detectTapGestures was unreliable on some devices — the gesture recognizer
 * introduces delays that swallow press events. pointerInteropFilter gives us
 * direct access to MotionEvent ACTION_DOWN/ACTION_UP without gesture delays.
 */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun PTTButton(
    isTransmitting: Boolean,
    pulseScale: Float,
    enabled: Boolean = true,
    dimmed: Boolean = false,
    onPressStart: () -> Unit,
    onPressEnd: () -> Unit,
    onPressRejected: () -> Unit = {}
) {
    // Gradient fill to match the Tauri desktop PTT: teal→purple (GradientCool)
    // idle, coral→purple (GradientWarm) while transmitting. Was a flat single-
    // color fill, which is why the Android PTT read as "unchanged" vs desktop.
    val buttonBrush = if (isTransmitting) {
        Brush.linearGradient(GradientWarm)
    } else {
        Brush.linearGradient(GradientCool)
    }
    val ringColor = if (isTransmitting) Coral else if (dimmed) TextMuted else Teal
    val innerColor = if (isTransmitting) CoralLight else BgCard

    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier
            .size(280.dp)
            .scale(pulseScale)
            .alpha(if (dimmed) 0.45f else 1f)
    ) {
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

        Box(
            contentAlignment = Alignment.Center,
            modifier = Modifier
                .size(240.dp)
                .clip(CircleShape)
                .border(8.dp, ringColor, CircleShape)
                .background(buttonBrush)
                .pointerInteropFilter { event ->
                    if (!enabled) {
                        // Surface WHY the button is inert instead of eating
                        // the touch silently — dead-feeling PTT reads as a bug.
                        if (event.action == android.view.MotionEvent.ACTION_DOWN) {
                            onPressRejected()
                        }
                        return@pointerInteropFilter false
                    }
                    when (event.action) {
                        android.view.MotionEvent.ACTION_DOWN -> {
                            onPressStart()
                            true
                        }
                        android.view.MotionEvent.ACTION_UP,
                        android.view.MotionEvent.ACTION_CANCEL -> {
                            onPressEnd()
                            true
                        }
                        else -> false
                    }
                }
        ) {
            Box(
                contentAlignment = Alignment.Center,
                modifier = Modifier
                    .size(120.dp)
                    .clip(CircleShape)
                    .background(innerColor)
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(
                        Icons.Default.Mic,
                        contentDescription = if (isTransmitting) "Transmitting" else "Push to talk",
                        tint = if (isTransmitting) TextWhite else TextGray,
                        modifier = Modifier.size(48.dp)
                    )
                    if (isTransmitting) {
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(
                            text = "TX",
                            fontSize = 16.sp,
                            fontWeight = FontWeight.Bold,
                            color = TextWhite
                        )
                    }
                }
            }
        }
    }
}
