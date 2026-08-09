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
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.layout.layout
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.pointerInteropFilter
import kotlin.math.roundToInt
import kotlinx.coroutines.withTimeoutOrNull
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
    var showSosConfirm by remember { mutableStateOf(false) }
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

    // Life-safety: our own beacon state + any peer currently in distress.
    val emergencyFallback = remember {
        kotlinx.coroutines.flow.MutableStateFlow(
            emptyMap<String, com.sassyconsulting.sassytalkie.PttCoordinator.PeerEmergency>()
        )
    }
    val peerEmergencies by (walkieService?.pttCoordinator?.peerEmergencies ?: emergencyFallback)
        .collectAsState()
    val selfEmergencyActive by (walkieService?.pttCoordinator?.selfEmergencyActive ?: falseFallback)
        .collectAsState()

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

    // Hoisted JNI reads. These were called straight from the composition body,
    // so every recompose crossed JNI — and this screen recomposes constantly
    // (pulse animation, TX/RX state, peer roster, emergency banners). Refreshed
    // on the advisory tick and whenever the connection state changes, which is
    // when either value can actually differ. The PTT press-time
    // `isEncrypted()` check below is deliberately NOT hoisted: that one gates
    // transmission and must read live state, not a cached copy.
    var isEncrypted by remember { mutableStateOf(SassyTalkNative.isEncrypted()) }
    var transportName by remember { mutableStateOf(SassyTalkNative.getTransportName()) }

    // PTT geometry, re-read on every entry to this screen so a Settings change
    // is visible the moment the user comes back rather than next launch.
    val pttShape = remember { readPttShape(settingsPrefs) }

    // Transport advisory refresh only — cache status polled centrally in TranscriptionBridge.
    // Woke every 2s but only did work on every 5th tick, so four of every five
    // wakeups computed nothing. Same effective 10s refresh, 1/5 the wakeups.
    LaunchedEffect(Unit) {
        while (true) {
            autoConnect.refreshTransportAdvisory()
            isEncrypted = SassyTalkNative.isEncrypted()
            transportName = SassyTalkNative.getTransportName()
            kotlinx.coroutines.delay(10_000L)
        }
    }

    // Connection transitions are the other moment these can change — pick them
    // up immediately rather than waiting out the 10s tick.
    LaunchedEffect(connectState) {
        isEncrypted = SassyTalkNative.isEncrypted()
        transportName = SassyTalkNative.getTransportName()
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
                        // Emergency SOS lives at menu level: on the radio
                        // screen it competed for space with the PTT target and
                        // sat where a sliding thumb lands. Reaching it is now
                        // menu -> item -> confirm, which is deliberate enough
                        // that the press itself needs no hold gesture.
                        DropdownMenuItem(
                            text = { Text("Emergency SOS", color = EmergencyRed) },
                            onClick = { showMenu = false; showSosConfirm = true },
                            leadingIcon = { Icon(Icons.Default.Warning, contentDescription = null, tint = EmergencyRed, modifier = Modifier.size(20.dp)) }
                        )
                        DropdownMenuItem(
                            text = { Text("End Session", color = Color(0xFFFF6B6B)) },
                            onClick = { showMenu = false; showEndSessionDialog = true },
                            leadingIcon = { Icon(Icons.Default.StopCircle, contentDescription = null, tint = Color(0xFFFF6B6B), modifier = Modifier.size(20.dp)) }
                        )
                    }
                }
            }
        }

        // ── Life-safety banners ──
        // Deliberately the FIRST thing under the header, above connection
        // status: a distress call must never be below the fold or competing
        // with routine transport chatter for attention.
        peerEmergencies.values.sortedByDescending { it.timestampMs }.forEach { em ->
            PeerEmergencyCard(em)
            Spacer(modifier = Modifier.height(6.dp))
        }

        if (selfEmergencyActive) {
            SelfEmergencyBanner(
                onClear = { walkieService?.pttCoordinator?.clearEmergency() }
            )
            Spacer(modifier = Modifier.height(6.dp))
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
                    val planeLabel = transportAdvisory?.activeLabel ?: transportName
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

        // Live translation — slim full-bleed bar above PTT (cancels the
        // Column's 16.dp inset).
        //
        // This CANNOT be `.padding(horizontal = (-16).dp)`: Compose's padding
        // modifier requires non-negative values and throws
        // `IllegalArgumentException: Padding must be non-negative` at modifier
        // construction — i.e. on every composition of this screen, which took
        // the whole radio screen down (shipped that way in 3.1.15/3.1.16).
        // The supported way to draw outside a parent's inset is a custom
        // layout that measures wider and places itself back by the inset.
        LiveTranslationOverlay(
            modifier = Modifier
                .fillMaxWidth()
                .layout { measurable, constraints ->
                    val bleed = 16.dp.roundToPx()
                    val widened = constraints.copy(
                        maxWidth = constraints.maxWidth + bleed * 2,
                        minWidth = (constraints.minWidth + bleed * 2)
                            .coerceAtMost(constraints.maxWidth + bleed * 2),
                    )
                    val placeable = measurable.measure(widened)
                    // Report the ORIGINAL width so siblings lay out unchanged;
                    // only the drawn bar extends past the inset.
                    layout(constraints.maxWidth, placeable.height) {
                        placeable.place(-bleed, 0)
                    }
                }
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
            // Debug builds only: log the first few frames before and after AEAD
            // so the encryption can be verified from outside the process. Frame-
            // budgeted in native, and compiled out of the user's reasoning
            // entirely in release — this puts mic-audio fragments in logcat.
            if (com.sassyconsulting.sassytalkie.BuildConfig.DEBUG) {
                SassyTalkNative.setCryptoTrace(true, 3)
            }
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
            shape = pttShape,
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

        Spacer(modifier = Modifier.height(12.dp))

        // SOS — long-press only. A tap-to-fire distress control next to a
        // 240dp PTT button would be triggered by accident constantly, and a
        // false SOS on a shared channel is expensive. Held for ~600ms it
        // raises the beacon; clearing is a separate deliberate tap on the
        // banner above. Hidden while our own beacon is already live.
        // Encryption warning only when session is not encrypted
        if (!isEncrypted) {
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

    // SOS confirmation. The menu route replaced the long-press: reaching this
    // point already took menu -> item, so one explicit confirm is the right
    // amount of friction for a broadcast everyone on the channel sees.
    if (showSosConfirm) {
        AlertDialog(
            onDismissRequest = { showSosConfirm = false },
            confirmButton = {
                TextButton(onClick = {
                    showSosConfirm = false
                    val ok = walkieService?.pttCoordinator?.raiseEmergency() ?: false
                    if (!ok) {
                        scope.launch {
                            snackbarHost.showSnackbar("SOS failed — no transport available")
                        }
                    }
                }) {
                    Text("BROADCAST SOS", color = EmergencyRed, fontWeight = FontWeight.Bold)
                }
            },
            dismissButton = {
                TextButton(onClick = { showSosConfirm = false }) {
                    Text("Cancel", color = Cyan)
                }
            },
            title = { Text("Broadcast emergency?", color = EmergencyRed) },
            text = {
                Text(
                    "Every device on this channel will show a full-screen alert with your " +
                        "name until you stand down. Use \"I'M OK\" on the banner to clear it.",
                    color = TextGray,
                    fontSize = 14.sp,
                )
            },
            containerColor = CardBg,
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

// ── PTT geometry ────────────────────────────────────────────────────────────

/**
 * User-configurable PTT target geometry.
 *
 * A fixed 240dp circle centred in the column assumes one grip. In practice the
 * radio is held left-handed, right-handed, thumbed from the bottom edge, or
 * pressed with the whole hand while the device sits in a mount — and a circle
 * is the shape with the least horizontal reach per unit of area. Width is
 * therefore scaled independently of height, and everything is adjustable.
 *
 * [circularity] is a spectrum, not a toggle: 100 = fully round (circle when
 * square, stadium when wide), 0 = hard rectangle, anything between = rounded
 * rect. Stored as 0..100 so the Settings slider maps 1:1 to a percentage.
 */
data class PttShape(
    val sizeDp: Float = PTT_DEFAULT_SIZE,
    val widthScale: Float = PTT_DEFAULT_WIDTH_SCALE,
    val circularity: Float = PTT_DEFAULT_CIRCULARITY,
    val offsetXDp: Float = 0f,
    val offsetYDp: Float = 0f,
)

const val PTT_DEFAULT_SIZE = 240f
const val PTT_DEFAULT_WIDTH_SCALE = 1.25f   // wider than tall by default: more reach
const val PTT_DEFAULT_CIRCULARITY = 100f

const val KEY_PTT_SIZE = "ptt_size_dp"
const val KEY_PTT_WIDTH_SCALE = "ptt_width_scale"
const val KEY_PTT_CIRCULARITY = "ptt_circularity"
const val KEY_PTT_OFFSET_X = "ptt_offset_x"
const val KEY_PTT_OFFSET_Y = "ptt_offset_y"

/** Read the persisted PTT geometry. Shared by the radio screen and Settings. */
fun readPttShape(prefs: android.content.SharedPreferences): PttShape = PttShape(
    sizeDp = prefs.getFloat(KEY_PTT_SIZE, PTT_DEFAULT_SIZE),
    widthScale = prefs.getFloat(KEY_PTT_WIDTH_SCALE, PTT_DEFAULT_WIDTH_SCALE),
    circularity = prefs.getFloat(KEY_PTT_CIRCULARITY, PTT_DEFAULT_CIRCULARITY),
    offsetXDp = prefs.getFloat(KEY_PTT_OFFSET_X, 0f),
    offsetYDp = prefs.getFloat(KEY_PTT_OFFSET_Y, 0f),
)

// ── Life-safety UI ──────────────────────────────────────────────────────────

/** Distress red. Deliberately not reused for any routine warning state so the
 *  colour alone reads as "emergency", not "degraded transport". */
private val EmergencyRed = Color(0xFFD32F2F)
private val EmergencyRedDim = Color(0xFF7F1D1D)

/**
 * A peer is in distress. Maximum prominence: filled red, top of the screen,
 * shows who, what kind, how long ago, and any note or coordinates they sent.
 */
@Composable
private fun PeerEmergencyCard(em: com.sassyconsulting.sassytalkie.PttCoordinator.PeerEmergency) {
    val ageSec = ((System.currentTimeMillis() - em.timestampMs) / 1000).coerceAtLeast(0)
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = EmergencyRed),
        shape = RoundedCornerShape(12.dp),
    ) {
        Column(modifier = Modifier.fillMaxWidth().padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Default.Warning,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.size(22.dp),
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = if (em.kind == "mandown") "MAN DOWN" else "EMERGENCY",
                    color = Color.White,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Bold,
                )
            }
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "${em.senderId} · ${ageSec}s ago",
                color = Color.White,
                fontSize = 14.sp,
                fontWeight = FontWeight.Medium,
            )
            em.note?.let {
                Spacer(modifier = Modifier.height(2.dp))
                Text(text = it, color = Color.White, fontSize = 13.sp, maxLines = 3)
            }
            if (em.lat != null && em.lon != null) {
                Spacer(modifier = Modifier.height(2.dp))
                Text(
                    text = "%.5f, %.5f".format(em.lat, em.lon),
                    color = Color.White,
                    fontSize = 12.sp,
                )
            }
            // An unsealed beacon means the sender had no session key, so the
            // name on it is unauthenticated. Say so rather than implying the
            // identity was verified.
            if (!em.sealed) {
                Spacer(modifier = Modifier.height(2.dp))
                Text(
                    text = "unverified sender (no session key)",
                    color = Color.White.copy(alpha = 0.85f),
                    fontSize = 11.sp,
                )
            }
        }
    }
}

/** Our own beacon is live. Persistent, with the only way to stand down. */
@Composable
private fun SelfEmergencyBanner(onClear: () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = EmergencyRedDim),
        shape = RoundedCornerShape(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                Icons.Default.Warning,
                contentDescription = null,
                tint = EmergencyRed,
                modifier = Modifier.size(20.dp),
            )
            Spacer(modifier = Modifier.width(8.dp))
            Text(
                text = "Broadcasting SOS",
                color = Color.White,
                fontSize = 14.sp,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = onClear) {
                Text("I'M OK", color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.Bold)
            }
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
    shape: PttShape = PttShape(),
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

    // User-shaped target. Height stays the configured size; width is scaled
    // independently because a thumb reaching across the screen contacts a wide
    // area, not a tall one — a pure circle forces the hand into one position.
    val h = shape.sizeDp.dp
    val w = (shape.sizeDp * shape.widthScale).dp
    // Circularity as a spectrum: 100% = fully rounded (a circle when square,
    // a stadium when wide), 0% = hard rectangle. RoundedCornerShape's percent
    // is relative to the shorter side, so 50% is the fully-round end.
    val cornerPct = (shape.circularity / 2f).roundToInt().coerceIn(0, 50)
    val btnShape = RoundedCornerShape(percent = cornerPct)

    // Clamp the offset so the target can never leave the screen.
    //
    // `offset` does not participate in layout: nothing reflows around it and
    // nothing bounds it. Driving X to its extreme with a 2x-wide button pushed
    // the control off the left edge and on top of the hint row — visible only
    // by actually setting the sliders to their limits. Clamp against the real
    // available width so any slider combination stays reachable.
    BoxWithConstraints(
        modifier = Modifier.fillMaxWidth(),
        contentAlignment = Alignment.Center,
    ) {
        val slackX = ((maxWidth - w) / 2).coerceAtLeast(0.dp)
        val clampedX = shape.offsetXDp.dp.coerceIn(-slackX, slackX)
        val yDp = shape.offsetYDp.dp

    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier
            // Vertical placement is PADDING, not offset. `offset` shifts only
            // the drawing, so a downward Y slid the button straight over the
            // hint row below it — the neighbours never moved. Padding consumes
            // the Column's slack instead, so everything below is pushed along
            // and nothing can overlap at any slider value. X stays a true
            // offset: it has no horizontal neighbours to disturb, and it is
            // clamped above so it cannot leave the screen.
            .padding(
                top = if (yDp > 0.dp) yDp else 0.dp,
                bottom = if (yDp < 0.dp) -yDp else 0.dp,
            )
            .offset(x = clampedX)
            .size(width = w + 40.dp, height = h + 40.dp)
            .scale(pulseScale)
            .alpha(if (dimmed) 0.45f else 1f)
    ) {
        if (isTransmitting) {
            Box(
                modifier = Modifier
                    .size(width = w + 60.dp, height = h + 60.dp)
                    .clip(btnShape)
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
                .size(width = w, height = h)
                .clip(btnShape)
                .border(8.dp, ringColor, btnShape)
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
}
