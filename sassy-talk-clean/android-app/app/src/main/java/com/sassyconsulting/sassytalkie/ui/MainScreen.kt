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
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.ui.theme.*
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.ui.platform.LocalContext
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

    val connectState by autoConnect.state.collectAsState()
    val connectStatusText by autoConnect.statusText.collectAsState()

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

    // Reaching-peer indicator (Task 4.2)
    val reachingPeer by (walkieService?.pttCoordinator?.reachingPeer ?: falseFallback).collectAsState()

    // Delivery state indicator (Task 4.3)
    val deliveryState by (walkieService?.pttCoordinator?.deliveredState ?: idleDeliveryFallback).collectAsState()

    // Audio path degraded indicator (Task 7.1)
    val audioPathDegraded by (walkieService?.pttCoordinator?.audioPathDegraded ?: falseFallback).collectAsState()

    // Stale-peer banner (Task 6.2)
    val anyPeerStale by (walkieService?.pttCoordinator?.anyPeerStale ?: falseFallback).collectAsState()

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

    // v2.7.1: active peer roster — set of HEALTHY/DEGRADED peer IDs.
    val peerIdsFallback = remember { kotlinx.coroutines.flow.MutableStateFlow<Set<String>>(emptySet()) }
    val activePeerIds by (walkieService?.pttCoordinator?.peerIds ?: peerIdsFallback).collectAsState()

    // v2.7.2: current network transport type ("wifi" / "cellular" / "none" / …)
    val netTypeFallback = remember { kotlinx.coroutines.flow.MutableStateFlow("none") }
    val networkType by (walkieService?.networkType ?: netTypeFallback).collectAsState()

    // v2.7.1: peer name lookup — re-resolved when peerIds changes.
    // getUsers() returns the cached UserRegistry roster; we filter to active.
    val peerNames = remember(activePeerIds) {
        val users = try { SassyTalkNative.getUsers() } catch (_: Throwable) { emptyList() }
        val byId = users.associate { it.id to it.name }
        activePeerIds.map { id -> byId[id] ?: id.take(6) }
    }

    // v2.7.1: snackbar host for peer join/leave toasts + the cache mini-strip.
    // `scope` is already declared earlier in this composable for the existing
    // disconnect handler — reuse it; don't redeclare.
    val snackbarHost = remember { androidx.compose.material3.SnackbarHostState() }

    // v2.7.1: subscribe to peer join/leave events → snackbar.
    // Only collects while this composable is in the tree; key = pttCoordinator
    // so we re-subscribe if the service rebinds (preserves identity otherwise).
    val coord = walkieService?.pttCoordinator
    LaunchedEffect(coord) {
        val users = try { SassyTalkNative.getUsers() } catch (_: Throwable) { emptyList() }
        val byId = users.associate { it.id to it.name }
        coord?.peerEvents?.collect { ev ->
            val name = when (ev) {
                is com.sassyconsulting.sassytalkie.PeerEvent.Joined -> byId[ev.peerId] ?: ev.peerId.take(6)
                is com.sassyconsulting.sassytalkie.PeerEvent.Left   -> byId[ev.peerId] ?: ev.peerId.take(6)
            }
            val verb = if (ev is com.sassyconsulting.sassytalkie.PeerEvent.Joined) "joined" else "left"
            scope.launch { snackbarHost.showSnackbar("$name $verb") }
        }
    }

    // v2.7.1: cache mini-status polled at 1 Hz for the bottom strip.
    var cacheMode by remember { mutableStateOf("Live") }
    var cacheQueued by remember { mutableIntStateOf(0) }
    var cacheNowName by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        while (true) {
            val st = SassyTalkNative.getCacheStatus()
            if (st != null) {
                cacheMode = st.optString("mode", "Live")
                cacheQueued = st.optInt("queued_utterances", 0)
                cacheNowName = st.optString("current_speaker_name", "").ifEmpty { null }
            }
            kotlinx.coroutines.delay(1000L)
        }
    }

    // Auto-connect and set cache to queue mode (cache-first)
    LaunchedEffect(Unit) {
        if (connectState != ConnectState.CONNECTED) {
            autoConnect.reset()
            autoConnect.autoConnect(walkieService)
        }
        withContext(Dispatchers.IO) {
            SassyTalkNative.restoreSession()
            SassyTalkNative.restoreCohortHistory()
            // Default to queue mode so incoming audio caches until user is done speaking
            SassyTalkNative.setCacheMode(SassyTalkNative.CACHE_MODE_QUEUE)
        }
    }

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
            // Subtle slate radial wash (BgMedium center → BgDark edge) to mirror
            // the Tauri desktop background's depth instead of a flat fill.
            .background(Brush.radialGradient(listOf(BgMedium, BgDark)))
            .safeDrawingPadding()
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
                scope.launch {
                    withContext(Dispatchers.IO) { SassyTalkNative.disconnect() }
                    onDisconnect()
                }
            }) {
                Icon(Icons.Default.ArrowBack, contentDescription = "Disconnect", tint = TextGray)
            }

            // Group name (editable per-channel) — defaults to "Sassy-Talk" if no name set
            val channelGroupName = remember(currentChannel) {
                SassyTalkNative.getGroupName(currentChannel)
            }
            // Brand title — blue→purple gradient, matching the Tauri reference
            // (app.css --gradient-primary). GradientPrimary is defined in
            // ui/theme/Color.kt.
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
                            leadingIcon = { Icon(Icons.Default.ExitToApp, contentDescription = null, tint = Color(0xFFFF6B6B), modifier = Modifier.size(20.dp)) }
                        )
                    }
                }
            }
        }

        // v2.7.2: network-type badge — small icon + label under the header.
        // "wifi" = green, "cellular" = orange, others = grey. Single source
        // of truth from WalkieService.networkType (registered ConnectivityManager
        // callback). Hidden when "none" so the chip doesn't yell at the user
        // during the brief boot window before the network resolves.
        if (networkType != "none") {
            val (badgeIcon, badgeColor, badgeText) = when (networkType) {
                "wifi"     -> Triple(Icons.Default.Wifi, Color(0xFF4CD964), "WiFi")
                "cellular" -> Triple(Icons.Default.SignalCellular4Bar, Color(0xFFFF8C00), "Cellular")
                "ethernet" -> Triple(Icons.Default.SettingsEthernet, Cyan, "Ethernet")
                "vpn"      -> Triple(Icons.Default.VpnLock, Cyan, "VPN")
                else       -> Triple(Icons.Default.NetworkCheck, TextMuted, networkType)
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 2.dp),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(badgeIcon, contentDescription = null, tint = badgeColor, modifier = Modifier.size(12.dp))
                Spacer(modifier = Modifier.width(4.dp))
                Text(badgeText, fontSize = 10.sp, color = badgeColor)
            }
        }

        // v2.7.1: peer roster chip — only visible while there are active peers.
        // Shows up to 3 names + "+N more"; full list available in the Users
        // screen via the existing menu.
        if (peerNames.isNotEmpty()) {
            val display = if (peerNames.size <= 3) {
                peerNames.joinToString(", ")
            } else {
                peerNames.take(3).joinToString(", ") + " +${peerNames.size - 3} more"
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 4.dp),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Default.People,
                    contentDescription = null,
                    tint = Cyan,
                    modifier = Modifier.size(14.dp),
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = "${peerNames.size} · $display",
                    fontSize = 12.sp,
                    color = TextGray,
                    maxLines = 1,
                )
            }
        }

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
                        text = "Reconnecting\u2026 peer out of contact",
                        fontSize = 13.sp,
                        color = Color(0xFFFFB300),
                        fontWeight = FontWeight.Medium
                    )
                }
            }
        }

        // Incoming audio indicator
        if (incomingAudio && !isTransmitting) {
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

        PTTButton(
            isTransmitting = isTransmitting,
            pulseScale = if (isTransmitting) pulseScale else 1f,
            enabled = pttEnabled,
            dimmed = anyPeerStale,
            holdMode = pttHoldMode,
            onPressStart = {
                if (!pttEnabled) return@PTTButton
                if (!SassyTalkNative.isEncrypted()) {
                    showEncryptionWarning = true
                } else if (pttHoldMode) {
                    // Toggle mode: tap to start/stop
                    if (isTransmitting) {
                        isTransmitting = false
                        SassyTalkNative.pttStop()
                        walkieService?.pttCoordinator?.notifyPttReleased()
                        walkieService?.updateNotification("Radio active \u2014 ${SassyTalkNative.getTransportName()}")
                    } else {
                        showEncryptionWarning = false
                        isTransmitting = true
                        SassyTalkNative.pttStart()
                        walkieService?.pttCoordinator?.notifyPttPressed()
                        walkieService?.updateNotification("Transmitting on CH $currentChannel")
                    }
                } else {
                    // Push-to-talk: press to start
                    showEncryptionWarning = false
                    isTransmitting = true
                    SassyTalkNative.pttStart()
                    walkieService?.pttCoordinator?.notifyPttPressed()
                    walkieService?.updateNotification("Transmitting on CH $currentChannel")
                }
            },
            onPressEnd = {
                // Only stop on release in push-to-talk mode (not hold mode)
                if (!pttHoldMode && isTransmitting) {
                    isTransmitting = false
                    SassyTalkNative.pttStop()
                    walkieService?.pttCoordinator?.notifyPttReleased()
                    walkieService?.updateNotification("Radio active \u2014 ${SassyTalkNative.getTransportName()}")
                }
            }
        )

        // Reaching-peer indicator (Task 4.2) — shown only while transmitting
        if (isTransmitting) {
            Spacer(modifier = Modifier.height(12.dp))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.Center,
                modifier = Modifier.fillMaxWidth()
            ) {
                Box(
                    modifier = Modifier
                        .size(10.dp)
                        .clip(CircleShape)
                        .background(if (reachingPeer) Color(0xFF4CAF50) else Color(0xFFFF5252))
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = if (reachingPeer) "Reaching peer" else "Not reaching",
                    fontSize = 13.sp,
                    color = if (reachingPeer) Color(0xFF4CAF50) else Color(0xFFFF5252)
                )
            }
        }

        // Audio path degraded warning (Task 7.1) — shown when PTT held and probe RTT exceeded
        if (isTransmitting && audioPathDegraded) {
            Spacer(modifier = Modifier.height(8.dp))
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = Color(0xFF3A2200)),
                shape = RoundedCornerShape(8.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.Center
                ) {
                    Icon(Icons.Default.Warning, contentDescription = null, tint = Color(0xFFFF9800), modifier = Modifier.size(16.dp))
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = "\u26A0\uFE0F Audio path slow",
                        fontSize = 13.sp,
                        color = Color(0xFFFF9800),
                        fontWeight = FontWeight.Medium
                    )
                }
            }
        }

        // Talk-over indicator (Task 6.2) — shown when we're transmitting and peer is also speaking
        if (isTransmitting && peerSpeaking) {
            Spacer(modifier = Modifier.height(8.dp))
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = Color(0xFF3A1A00)),
                shape = RoundedCornerShape(8.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.Center
                ) {
                    Icon(Icons.Default.Mic, contentDescription = null, tint = Color(0xFFFF9800), modifier = Modifier.size(16.dp))
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = "They are also talking",
                        fontSize = 13.sp,
                        color = Color(0xFFFF9800),
                        fontWeight = FontWeight.Medium
                    )
                }
            }
        }

        // Delivery state indicator (Task 4.3) — shown after PTT release
        if (!isTransmitting && deliveryState != com.sassyconsulting.sassytalkie.DeliveryState.Idle) {
            Spacer(modifier = Modifier.height(12.dp))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.Center,
                modifier = Modifier.fillMaxWidth()
            ) {
                when (deliveryState) {
                    com.sassyconsulting.sassytalkie.DeliveryState.Sending -> {
                        CircularProgressIndicator(
                            modifier = Modifier.size(14.dp),
                            strokeWidth = 2.dp,
                            color = Color(0xFFFF9800)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = "Sending...",
                            fontSize = 13.sp,
                            color = Color(0xFFFF9800)
                        )
                    }
                    com.sassyconsulting.sassytalkie.DeliveryState.Delivered -> {
                        Text(
                            text = "\u2713 Delivered",
                            fontSize = 13.sp,
                            color = Color(0xFF4CAF50),
                            fontWeight = FontWeight.Medium
                        )
                    }
                    else -> {}
                }
            }
        }

        Spacer(modifier = Modifier.weight(1f))

        // Status Bar
        StatusBar(isTransmitting = isTransmitting, channel = currentChannel)

        Spacer(modifier = Modifier.height(8.dp))

        // Transport + encryption badge
        val encStatus = if (SassyTalkNative.isEncrypted()) "AES-256-GCM" else "\uD83D\uDD13 UNENCRYPTED"
        val encColor = if (SassyTalkNative.isEncrypted()) TextMuted else Color(0xFFFF6B6B)
        Text(
            text = "$encStatus \u2022 Opus \u2022 ${SassyTalkNative.getTransportName()}",
            fontSize = 11.sp,
            color = encColor,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth()
        )

        // v2.7.1: cache mini-strip — only visible when cache is non-idle.
        // Compact mirror of TranscriptionFeedScreen's cache bar so the user
        // can see "audio is queued / playing" without leaving the main screen.
        if (cacheNowName != null || cacheQueued > 0 || cacheMode == "Queue" || cacheMode == "Mix") {
            val pipColor = when (cacheMode) {
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
                        cacheNowName != null -> "Playing $cacheNowName" +
                            (if (cacheQueued > 0) " · $cacheQueued queued" else "")
                        cacheQueued > 0      -> "$cacheQueued queued"
                        else                  -> cacheMode
                    },
                    fontSize = 11.sp,
                    color = TextMuted,
                )
            }
        }

        Spacer(modifier = Modifier.height(8.dp))
    }

    // v2.7.1: SnackbarHost overlay for peer join/leave toasts.
    Box(modifier = Modifier.fillMaxSize()) {
        androidx.compose.material3.SnackbarHost(
            hostState = snackbarHost,
            modifier = Modifier.align(Alignment.BottomCenter)
        )
    }

    // Show QR dialog for current session
    if (showQrDialog) {
        val sessionStatus = remember { SassyTalkNative.getSessionStatus() }
        val sessionId = remember { SassyTalkNative.getSessionId() ?: "" }
        val channelInfo = remember { SassyTalkNative.getChannelInfo() }

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
                        // Generate QR bitmap
                        val qrBitmap = remember(sessionJson) {
                            try {
                                val writer = com.google.zxing.qrcode.QRCodeWriter()
                                val matrix = writer.encode(sessionJson, com.google.zxing.BarcodeFormat.QR_CODE, 400, 400)
                                val bmp = android.graphics.Bitmap.createBitmap(400, 400, android.graphics.Bitmap.Config.RGB_565)
                                for (x in 0 until 400) for (y in 0 until 400)
                                    bmp.setPixel(x, y, if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE)
                                bmp
                            } catch (_: Exception) { null }
                        }
                        if (qrBitmap != null) {
                            Card(
                                colors = CardDefaults.cardColors(containerColor = Color.White),
                                shape = RoundedCornerShape(12.dp)
                            ) {
                                Image(
                                    bitmap = qrBitmap.asImageBitmap(),
                                    contentDescription = "Session QR",
                                    modifier = Modifier.size(180.dp).padding(8.dp)
                                )
                            }
                        }
                        Spacer(modifier = Modifier.height(8.dp))
                        Text("Scan to join CH $currentChannel", color = TextGray, fontSize = 13.sp)
                        if (sessionId.isNotEmpty()) {
                            Text("Session: ${sessionId.take(8)}", color = TextMuted, fontSize = 11.sp)
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
    holdMode: Boolean = false,
    onPressStart: () -> Unit,
    onPressEnd: () -> Unit
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
                    if (!enabled) return@pointerInteropFilter false
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
