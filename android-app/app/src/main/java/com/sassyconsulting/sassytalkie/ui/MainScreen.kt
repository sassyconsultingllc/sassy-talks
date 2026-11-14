// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-53CVA2GFR6FK
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.animation.core.*
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import com.sassyconsulting.sassytalkie.RelayConnectionState
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
    onShowUpgrade: () -> Unit = {},
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
    val emptyDeliveryProgressFallback = remember {
        kotlinx.coroutines.flow.MutableStateFlow(com.sassyconsulting.sassytalkie.DeliveryProgress())
    }

    // Reaching-peer indicator — use reach-failed (watchdog) not inverted reachingPeer
    val peerReachFailed by (walkieService?.pttCoordinator?.peerReachFailed ?: falseFallback).collectAsState()

    // Delivery state indicator (Task 4.3)
    val deliveryState by (walkieService?.pttCoordinator?.deliveredState ?: idleDeliveryFallback).collectAsState()
    val deliveryProgress by (
        walkieService?.pttCoordinator?.deliveryProgress ?: emptyDeliveryProgressFallback
    ).collectAsState()

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
    val trueFallback = remember { kotlinx.coroutines.flow.MutableStateFlow(true) }
    val selfEmergencyDelivered by (walkieService?.pttCoordinator?.selfEmergencyDelivered ?: trueFallback)
        .collectAsState()

    // Free-trial accounting: a session only counts once a peer is actually
    // present. Creating a room nobody joins proves nothing to the user, so it
    // must not burn a free session — see TrialGate for why "had a peer" is the
    // threshold rather than "opened the app" or "made a QR".
    val trialPeerIds by (walkieService?.pttCoordinator?.peerIds
        ?: remember { kotlinx.coroutines.flow.MutableStateFlow(emptySet<String>()) })
        .collectAsState()
    LaunchedEffect(trialPeerIds.isNotEmpty()) {
        if (trialPeerIds.isNotEmpty()) {
            withContext(Dispatchers.IO) {
                com.sassyconsulting.sassytalkie.license.TrialGate.noteQualifyingSession(
                    context,
                    SassyTalkNative.getSessionId(),
                )
            }
        }
    }

    // Trial countdown. Re-read whenever a peer appears, since that is the only
    // moment the count can change — see TrialGate.noteQualifyingSession above.
    var trialWarn by remember { mutableStateOf(false) }
    var trialLeft by remember { mutableIntStateOf(0) }
    LaunchedEffect(trialPeerIds.isNotEmpty()) {
        withContext(Dispatchers.IO) {
            val warn = com.sassyconsulting.sassytalkie.license.TrialGate.shouldWarn(context)
            val left = com.sassyconsulting.sassytalkie.license.TrialGate.sessionsRemaining(context)
            withContext(Dispatchers.Main) { trialWarn = warn; trialLeft = left }
        }
    }

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
            .background(Ui.Bg)
    ) {
    Column(
        modifier = Modifier
            .fillMaxHeight()
            // Cap + center content on large screens so a 7"+ tablet isn't a
            // stretched phone with big empty side gaps. No-op on phones (<560dp).
            .widthIn(max = 560.dp)
            .fillMaxWidth()
            .align(Alignment.TopCenter)
            .padding(horizontal = 20.dp, vertical = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // ── Header: channel, link status, invite, SOS ──
        // The overflow menu is gone: Activity / People / Settings live in the
        // bottom tab bar, End Session moved to People. SOS sits top-right, as
        // far from the talk button as the screen allows, and still needs the
        // confirm dialog, so a stray thumb cannot broadcast it.
        val channelGroupName = remember(currentChannel) {
            SassyTalkNative.getGroupName(currentChannel)
        }
        val subLabel = listOf("Main", "A", "B").getOrElse(currentSubchannel) { "Main" }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 4.dp, bottom = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = channelGroupName.ifEmpty { "Channel $currentChannel" } + " \u00b7 " + subLabel,
                    fontSize = 22.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = Ui.Text,
                    maxLines = 1,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
                Spacer(modifier = Modifier.height(4.dp))
                LinkStatusLine(
                    connectState = connectState,
                    statusText = connectStatusText,
                    planeLabel = transportAdvisory?.activeLabel ?: transportName,
                    encrypted = isEncrypted,
                    sealed = sealedSenderOn && autoConnect.isUsingRelay(),
                    peers = trialPeerIds.size,
                    onReconnect = {
                        scope.launch {
                            autoConnect.reset()
                            autoConnect.autoConnect(walkieService)
                        }
                    },
                )
            }
            IconButton(
                onClick = { showQrDialog = true },
                modifier = Modifier
                    .size(44.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(Ui.Surface)
                    .border(1.dp, Ui.Line, RoundedCornerShape(12.dp)),
            ) {
                Icon(Icons.Default.QrCode2, contentDescription = "Invite with QR code", tint = Ui.Text, modifier = Modifier.size(20.dp))
            }
            OutlinedButton(
                onClick = { showSosConfirm = true },
                modifier = Modifier.height(44.dp),
                shape = RoundedCornerShape(12.dp),
                border = androidx.compose.foundation.BorderStroke(1.5.dp, Ui.Sos),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = Ui.SosText),
                contentPadding = PaddingValues(horizontal = 14.dp),
            ) {
                Text("SOS", fontWeight = FontWeight.Bold, fontSize = 15.sp, letterSpacing = 1.sp)
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
                delivered = selfEmergencyDelivered,
                onClear = { walkieService?.pttCoordinator?.clearEmergency() }
            )
            Spacer(modifier = Modifier.height(6.dp))
        }

        // Trial countdown. Shown only for the last two free sessions: the
        // paywall used to arrive with no warning at all, and a countdown from
        // five would nag someone who has barely started.
        if (trialWarn) {
            TrialCountdownBanner(sessionsLeft = trialLeft, onUpgrade = onShowUpgrade)
            Spacer(modifier = Modifier.height(6.dp))
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

        // Relay readiness — socket-up vs Durable Object / room confirm.
        if (connectState == ConnectState.CONNECTED && autoConnect.isUsingRelay()) {
            val status = RelayConnectionState.radioStatusLine(
                socketLive = true,
                roomConfirmed = relayReady,
                sealedSenderPending = sealedSenderOn && !relayReady,
                usingRelay = true,
            )
            if (status != null) {
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
                        text = status,
                        fontSize = 11.sp,
                        color = Orange
                    )
                }
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

        // ── Clip / channel area ──
        // Whole-clip playback: a transmission is heard only once it has fully
        // arrived, so the screen shows what is playing and what is still
        // arriving. When nothing is, the channel controls take this slot.
        val playingName = cacheSnap.currentSpeakerName?.takeIf { it.isNotBlank() && it != "null" }
        val arriving = cacheSnap.receiving
        val rxBusy = !isTransmitting && (playingName != null || arriving.isNotEmpty())
        var txElapsedMs by remember { mutableLongStateOf(0L) }
        LaunchedEffect(isTransmitting) {
            val start = System.currentTimeMillis()
            txElapsedMs = 0L
            while (isTransmitting) {
                txElapsedMs = System.currentTimeMillis() - start
                kotlinx.coroutines.delay(250L)
            }
        }
        Spacer(modifier = Modifier.height(4.dp))
        when {
            isTransmitting -> OnAirCard(
                elapsedMs = txElapsedMs,
                peers = trialPeerIds.size,
                limitMs = walkieService?.pttCoordinator?.maxTxMs,
            )
            rxBusy -> Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                if (playingName != null) {
                    NowPlayingCard(
                        name = playingName,
                        positionMs = cacheSnap.playingMs,
                        totalMs = cacheSnap.playingTotalMs,
                        onSkip = { SassyTalkNative.skipCurrentUtterance() },
                    )
                }
                arriving.take(2).forEach { clip ->
                    ArrivingCard(name = clip.name, ms = clip.ms, playsNext = playingName != null)
                }
            }
            else -> ChannelSelector(
                channel = currentChannel,
                subchannel = currentSubchannel,
                onChannelChange = { newChannel ->
                    currentChannel = newChannel
                    SassyTalkNative.setChannel(newChannel)
                },
                onSubchannelChange = { idx ->
                    currentSubchannel = idx
                    SassyTalkNative.setSubchannel(idx)
                },
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
            busy = rxBusy,
            holdMode = pttHoldMode,
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
            deliveryState == com.sassyconsulting.sassytalkie.DeliveryState.Acknowledged ->
                if (deliveryProgress.expected > 0) {
                    "Acknowledged by ${deliveryProgress.acknowledged}/${deliveryProgress.expected}" to Color(0xFF4CAF50)
                } else {
                    "Acknowledged by ${deliveryProgress.acknowledged}/?" to Color(0xFF4CAF50)
                }
            rxBusy -> "You can still talk over them" to Ui.TextDim
            else -> "Ready" to Ui.TextDim
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
                    deliveryState == com.sassyconsulting.sassytalkie.DeliveryState.Acknowledged ->
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
            if (!isTransmitting) {
                Spacer(modifier = Modifier.width(12.dp))
                OutlinedButton(
                    onClick = { pttHoldMode = !pttHoldMode },
                    modifier = Modifier.height(32.dp),
                    shape = RoundedCornerShape(16.dp),
                    border = androidx.compose.foundation.BorderStroke(1.dp, if (pttHoldMode) Ui.Accent else Ui.Line),
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = if (pttHoldMode) Ui.Accent else Ui.TextMid),
                    contentPadding = PaddingValues(horizontal = 12.dp),
                ) {
                    Text(if (pttHoldMode) "Tap mode: on" else "Tap mode: off", fontSize = 13.sp)
                }
            }
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

        // Room for the diagnostics chip that sits above the tab bar.
        Spacer(modifier = Modifier.height(36.dp))
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
                            snackbarHost.showSnackbar("SOS not sent yet — no connection. Retrying every second.")
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
private fun SelfEmergencyBanner(delivered: Boolean, onClear: () -> Unit) {
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
                text = if (delivered) "Broadcasting SOS" else "SOS NOT SENT — no connection, retrying",
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
    subchannel: Int,
    onChannelChange: (Int) -> Unit,
    onSubchannelChange: (Int) -> Unit,
) {
    UiCard {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(
                    onClick = { if (channel > 1) onChannelChange(channel - 1) },
                    modifier = Modifier.size(52.dp).clip(RoundedCornerShape(16.dp)).background(Ui.Surface2),
                ) {
                    Icon(Icons.Default.Remove, contentDescription = "Channel Down", tint = Ui.Text, modifier = Modifier.size(28.dp))
                }
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("CHANNEL", fontSize = 12.sp, color = Ui.TextDim, letterSpacing = 1.5.sp)
                    Text("%02d".format(channel), fontSize = 48.sp, fontWeight = FontWeight.Bold, color = Ui.Accent)
                }
                IconButton(
                    onClick = { if (channel < 99) onChannelChange(channel + 1) },
                    modifier = Modifier.size(52.dp).clip(RoundedCornerShape(16.dp)).background(Ui.Surface2),
                ) {
                    Icon(Icons.Default.Add, contentDescription = "Channel Up", tint = Ui.Text, modifier = Modifier.size(28.dp))
                }
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(Ui.Bg)
                    .padding(4.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                listOf("Main", "A", "B").forEachIndexed { idx, label ->
                    val on = idx == subchannel
                    TextButton(
                        onClick = { onSubchannelChange(idx) },
                        modifier = Modifier.weight(1f).height(40.dp),
                        shape = RoundedCornerShape(9.dp),
                        colors = ButtonDefaults.textButtonColors(
                            containerColor = if (on) Ui.Accent else Color.Transparent,
                            contentColor = if (on) Ui.AccentInk else Ui.TextDim,
                        ),
                    ) {
                        Text(label, fontSize = 14.sp, fontWeight = if (on) FontWeight.SemiBold else FontWeight.Normal)
                    }
                }
            }
        }
    }
}

/** One-line link status under the channel title; tap to reconnect. */
@Composable
private fun LinkStatusLine(
    connectState: ConnectState,
    statusText: String,
    planeLabel: String,
    encrypted: Boolean,
    sealed: Boolean,
    peers: Int,
    onReconnect: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .clip(RoundedCornerShape(6.dp))
            .clickable(onClickLabel = "Reconnect", onClick = onReconnect),
    ) {
        when (connectState) {
            ConnectState.CONNECTED -> {
                Box(Modifier.size(8.dp).clip(CircleShape).background(Ui.Ok))
                Spacer(Modifier.width(8.dp))
                val parts = buildList {
                    add(planeLabel)
                    add(if (encrypted) "Encrypted" else "Not encrypted")
                    if (sealed) add("Sealed")
                    add(if (peers == 0) "No one else yet" else "$peers on channel")
                }
                Text(
                    parts.joinToString(" \u00b7 "),
                    fontSize = 13.sp,
                    color = if (encrypted) Ui.TextDim else Ui.SosText,
                    maxLines = 1,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
            }
            ConnectState.FAILED -> {
                Box(Modifier.size(8.dp).clip(CircleShape).background(Ui.Sos))
                Spacer(Modifier.width(8.dp))
                Text("$statusText \u00b7 Tap to retry", fontSize = 13.sp, color = Ui.SosText, maxLines = 1)
            }
            else -> {
                CircularProgressIndicator(modifier = Modifier.size(12.dp), strokeWidth = 1.5.dp, color = Ui.Accent)
                Spacer(Modifier.width(8.dp))
                Text(statusText, fontSize = 13.sp, color = Ui.TextDim, maxLines = 1)
            }
        }
    }
}

private fun clock(ms: Long): String {
    val s = (ms / 1000).coerceAtLeast(0)
    return "%d:%02d".format(s / 60, s % 60)
}

/** The received clip currently playing, with progress. */
@Composable
private fun NowPlayingCard(name: String, positionMs: Long, totalMs: Long, onSkip: () -> Unit) {
    UiCard(container = Ui.AccentSoft, border = Ui.AccentLine) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    Modifier.size(44.dp).clip(CircleShape).background(Ui.Accent),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(name.take(2).uppercase(), color = Ui.AccentInk, fontWeight = FontWeight.Bold)
                }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(name, color = Ui.Text, fontWeight = FontWeight.SemiBold, fontSize = 16.sp, maxLines = 1)
                    Text(
                        "Playing clip \u00b7 ${clock(positionMs.coerceAtMost(totalMs))} of ${clock(totalMs)}",
                        color = Ui.AccentText,
                        fontSize = 13.sp,
                    )
                }
                IconButton(onClick = onSkip) {
                    Icon(Icons.Default.SkipNext, contentDescription = "Skip this clip", tint = Ui.AccentText)
                }
            }
            LinearProgressIndicator(
                progress = if (totalMs > 0) (positionMs.toFloat() / totalMs).coerceIn(0f, 1f) else 0f,
                modifier = Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)),
                color = Ui.Accent,
                trackColor = Ui.AccentLine,
            )
        }
    }
}

/** A transmission still arriving; it plays once it ends. */
@Composable
private fun ArrivingCard(name: String, ms: Long, playsNext: Boolean) {
    UiCard {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Default.GraphicEq, contentDescription = null, tint = Ui.OnAir, modifier = Modifier.size(24.dp))
            Spacer(Modifier.width(12.dp))
            Column {
                Text("$name is talking", color = Ui.Text, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
                Text(
                    "Receiving \u00b7 ${clock(ms)} so far \u00b7 " + if (playsNext) "plays next" else "plays when they let go",
                    color = Ui.TextDim,
                    fontSize = 13.sp,
                )
            }
        }
    }
}

/** Shown while we transmit. */
@Composable
private fun OnAirCard(elapsedMs: Long, peers: Int, limitMs: Long?) {
    UiCard(container = Ui.OnAirSoft, border = Ui.OnAirLine) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Text("ON AIR", color = Ui.OnAir, fontWeight = FontWeight.Bold, fontSize = 15.sp, letterSpacing = 1.2.sp)
                Text(clock(elapsedMs), color = Ui.OnAirText, fontWeight = FontWeight.SemiBold, fontSize = 28.sp)
            }
            val who = if (peers == 1) "1 person" else "$peers people"
            val limit = limitMs?.let { " \u00b7 limit ${clock(com.sassyconsulting.sassytalkie.TxSafetyPolicy.normalizeMaxTxMs(it))}" } ?: ""
            Text("Sending to $who \u00b7 they hear it when you let go$limit", color = Ui.OnAirText, fontSize = 13.sp)
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
                scale.animateTo(1.04f, tween(600, easing = EaseInOut))
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
    busy: Boolean = false,
    holdMode: Boolean = false,
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
    // 3.2.8 design: flat fills. Teal = ready, orange = on air, grey = a peer
    // holds the channel (still pressable: barge-in stays possible).
    val fill = when {
        isTransmitting -> Ui.OnAir
        busy || dimmed -> Ui.Surface2
        else -> Ui.Accent
    }
    val buttonBrush = Brush.linearGradient(listOf(fill, fill))
    val ringColor = if (isTransmitting) Ui.OnAirLine else Ui.Surface2
    val ink = when {
        isTransmitting -> Ui.OnAirInk
        busy || dimmed -> Ui.TextMid
        else -> Ui.AccentInk
    }
    val label = when {
        isTransmitting && holdMode -> "Tap to send"
        isTransmitting -> "Release to send"
        busy -> "Channel busy"
        holdMode -> "Tap to talk"
        else -> "Hold to talk"
    }

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
                                Ui.OnAir.copy(alpha = 0.25f),
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
                .border(10.dp, ringColor, btnShape)
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
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(
                    Icons.Default.Mic,
                    contentDescription = if (isTransmitting) "Transmitting" else "Push to talk",
                    tint = ink,
                    modifier = Modifier.size(52.dp),
                )
                Spacer(modifier = Modifier.height(6.dp))
                Text(text = label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, color = ink)
            }
        }
    }
    }
}

/**
 * Trial countdown shown on the radio screen for the last two free sessions.
 *
 * Deliberately understated — a muted card, not an interstitial. The user is
 * mid-radio-session; the point is that the paywall does not arrive as a
 * surprise, not to interrupt the thing they came here to do. "Upgrade" is
 * offered so they can act on the warning immediately instead of having to
 * remember it later, but ignoring it costs them nothing until the count runs
 * out.
 */
@Composable
private fun TrialCountdownBanner(sessionsLeft: Int, onUpgrade: () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = SurfaceBg),
        shape = RoundedCornerShape(10.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                Icons.Default.Info,
                contentDescription = null,
                tint = Orange,
                modifier = Modifier.size(16.dp),
            )
            Spacer(modifier = Modifier.width(8.dp))
            Text(
                text = if (sessionsLeft <= 1) {
                    "Last free session"
                } else {
                    "$sessionsLeft free sessions left"
                },
                color = TextGray,
                fontSize = 12.sp,
                fontWeight = FontWeight.Medium,
                modifier = Modifier.weight(1f),
            )
            TextButton(
                onClick = onUpgrade,
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp),
            ) {
                Text("Upgrade", color = Cyan, fontSize = 12.sp, fontWeight = FontWeight.Bold)
            }
        }
    }
}
