// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-PBLN6O7JKWAZ
package com.sassyconsulting.sassytalkie.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.widget.Toast
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.SessionShareLink
import com.sassyconsulting.sassytalkie.ui.theme.*
import com.sassyconsulting.sassytalkie.ui.util.QrBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

@Composable
/**
 * @param onAuthenticated invoked once the local session is established AND the
 *   user is ready to leave this screen (after import, or after the host taps
 *   Continue on their generated QR). Caller is expected to trigger a relay
 *   reconnect (e.g. via AutoConnectManager.disconnect → MainScreen re-mounts).
 * @param onSessionMutated invoked the instant the local session_id changes
 *   (after a successful generate OR import) — caller should force a cellular
 *   WS teardown+reconnect so the relay attaches to the new room immediately,
 *   without waiting for the user to navigate.
 *   Critical for the HOST path: the host generates a QR and STAYS on this
 *   screen waiting for joiners; without an immediate reconnect, the host's
 *   WS stays bound to the old room and joiners scan into an empty room.
 */
fun QRAuthScreen(
    onAuthenticated: () -> Unit,
    onSessionMutated: () -> Unit = {},
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val qrDisplayPx = with(density) { 160.dp.roundToPx() }
    var selectedTab by remember { mutableIntStateOf(0) }
    var selectedChannel by remember { mutableIntStateOf(1) }
    var groupName by remember { mutableStateOf("") }
    var durationHours by remember { mutableIntStateOf(24) }
    var qrBitmap by remember { mutableStateOf<Bitmap?>(null) }
    var lastGeneratedJson by remember { mutableStateOf("") }
    var scanResult by remember { mutableStateOf<String?>(null) }
    var showScanner by remember { mutableStateOf(false) }
    val hasExistingSession = remember { mutableStateOf(SassyTalkNative.isAuthenticated()) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DarkBg)
            .imePadding()
            .padding(horizontal = 8.dp, vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Spacer(modifier = Modifier.height(8.dp))

        // Title
        Text(
            text = "Authenticate",
            fontSize = 24.sp,
            fontWeight = FontWeight.Bold,
            color = Teal
        )

        Spacer(modifier = Modifier.height(2.dp))

        Text(
            text = "Scan a QR code or share a session code",
            fontSize = 12.sp,
            color = TextGray,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(8.dp))

        // ── Existing session card ──
        // Suppressed while the Show QR tab is displaying a freshly-generated
        // QR: that tab renders its own Continue directly above the code, and
        // stacking the card on top duplicated the button and pushed the QR
        // below the fold (the old "scroll to find Continue" complaint).
        if (hasExistingSession.value && !(selectedTab == 0 && qrBitmap != null)) {
            ActiveSessionCard(
                onContinue = { activeChannel ->
                    scope.launch {
                        // Re-arm live crypto from the persisted channel session
                        // when a prior native disconnect wiped it. Without this
                        // re-import, Continue landed on Main un-encrypted and
                        // every PTT press bounced off "Authenticate via QR
                        // first" while presence still showed peers.
                        if (activeChannel > 0 && !SassyTalkNative.isEncrypted()) {
                            val rearmed = withContext(Dispatchers.IO) {
                                val stored = try {
                                    SassyTalkNative.getChannelSessionJson(activeChannel)
                                } catch (_: Throwable) { "" }
                                stored.isNotEmpty() && try {
                                    SassyTalkNative.importSessionFromQR(stored)
                                } catch (_: Throwable) { false }
                            }
                            // Same post-import contract as scan/paste joins:
                            // the WS must attach to the session's room.
                            if (rearmed) onSessionMutated()
                        }
                        onAuthenticated()
                    }
                },
                onNewSession = {
                    SassyTalkNative.clearSession()
                    hasExistingSession.value = false
                    qrBitmap = null
                    lastGeneratedJson = ""
                    scanResult = null
                }
            )
            Spacer(modifier = Modifier.height(16.dp))
        }

        // Tab row: Show QR | Scan QR | Enter Code
        ScrollableTabRow(
            selectedTabIndex = selectedTab,
            containerColor = CardBg,
            contentColor = Teal,
            edgePadding = 0.dp,
            modifier = Modifier.clip(RoundedCornerShape(12.dp))
        ) {
            Tab(
                selected = selectedTab == 0,
                onClick = { selectedTab = 0 },
                text = { Text("Show QR", color = if (selectedTab == 0) Teal else TextGray, fontSize = 12.sp) }
            )
            Tab(
                selected = selectedTab == 1,
                onClick = { selectedTab = 1 },
                text = { Text("Scan QR", color = if (selectedTab == 1) Teal else TextGray, fontSize = 12.sp) }
            )
            Tab(
                selected = selectedTab == 2,
                onClick = { selectedTab = 2 },
                text = { Text("Enter Code", color = if (selectedTab == 2) Teal else TextGray, fontSize = 12.sp) }
            )
            Tab(
                selected = selectedTab == 3,
                onClick = { selectedTab = 3 },
                text = { Text("My Cohorts", color = if (selectedTab == 3) Teal else TextGray, fontSize = 12.sp) }
            )
        }

        Spacer(modifier = Modifier.height(8.dp))

        when (selectedTab) {
            0 -> ShowQRTab(
                context = context,
                selectedChannel = selectedChannel,
                onChannelChange = { selectedChannel = it },
                groupName = groupName,
                onGroupNameChange = { groupName = it },
                durationHours = durationHours,
                onDurationChange = { durationHours = it },
                qrBitmap = qrBitmap,
                sessionJson = lastGeneratedJson,
                onGenerate = {
                    val qrJson = SassyTalkNative.generateChannelQR(
                        selectedChannel, durationHours, groupName
                    )
                    if (qrJson.isNotEmpty()) {
                        lastGeneratedJson = qrJson
                        scope.launch {
                            qrBitmap = withContext(Dispatchers.Default) {
                                QrBitmap.generate(qrJson, qrDisplayPx)
                            }
                        }
                        hasExistingSession.value = true
                        // Host-side: a new session_id was just minted. Force
                        // the cellular WS to reconnect so the host attaches
                        // to the new room BEFORE any joiner scans the QR.
                        onSessionMutated()
                    }
                },
                onContinue = {
                    onAuthenticated()
                }
            )
            1 -> ScanQRTab(
                scanResult = scanResult,
                showScanner = showScanner,
                onStartScan = { showScanner = true },
                onQRScanned = { json ->
                    showScanner = false
                    val success = SassyTalkNative.importSessionFromQR(json)
                    val roomHint = runCatching {
                        org.json.JSONObject(json).optString("session_id", "").take(8)
                    }.getOrDefault("")
                    scanResult = when {
                        success && roomHint.isNotEmpty() ->
                            "Joined room $roomHint — match the host Room id"
                        success -> "Session established!"
                        else -> "Invalid QR code"
                    }
                    if (success) {
                        hasExistingSession.value = true
                        // Joiner-side: force-reconnect to the host's room
                        // immediately. autoConnect.disconnect() in
                        // onAuthenticated also tears down, but that's a
                        // soft path that depends on MainScreen re-mounting;
                        // the explicit force is the load-bearing call.
                        onSessionMutated()
                        onAuthenticated()
                    }
                }
            )
            2 -> EnterCodeTab(
                context = context,
                onAuthenticated = {
                    hasExistingSession.value = true
                    // Same path as scan-import: session_id just changed, need
                    // the WS to land on the host's room before the joiner
                    // exits this screen.
                    onSessionMutated()
                    onAuthenticated()
                }
            )
            3 -> MyCohortsTab(
                onRejoinHost = { channel, gName, cohortId ->
                    selectedChannel = channel
                    groupName = gName
                    val qr = SassyTalkNative.generateChannelQR(channel, durationHours, gName, cohortId)
                    if (qr.isNotEmpty()) {
                        lastGeneratedJson = qr
                        scope.launch {
                            qrBitmap = withContext(Dispatchers.Default) {
                                QrBitmap.generate(qr, qrDisplayPx)
                            }
                        }
                        hasExistingSession.value = true
                        selectedTab = 0
                        // Same host-side reconnect as the fresh-generate path.
                        onSessionMutated()
                    }
                },
                onRejoinJoiner = { channel, hostDevice ->
                    // v2.7.3: prefer rejoining with the LOCALLY-PERSISTED
                    // session credentials. The full JSON (including the
                    // AES key) was stashed during the original import via
                    // SessionManager → EncryptedSharedPreferences. If it
                    // is still on disk, re-import + force reconnect — no
                    // QR rescan, no message to the host needed.
                    val stored = try {
                        SassyTalkNative.getChannelSessionJson(channel)
                    } catch (_: Throwable) { "" }

                    if (stored.isNotEmpty()) {
                        val ok = try {
                            SassyTalkNative.importSessionFromQR(stored)
                        } catch (_: Throwable) { false }
                        if (ok) {
                            hasExistingSession.value = true
                            scanResult = "Rejoined " + (hostDevice ?: "session")
                            onSessionMutated()   // force WS reconnect to host's room
                            onAuthenticated()    // navigate to Main
                            return@MyCohortsTab
                        }
                    }
                    // Fallback: credentials gone (wiped / fresh install).
                    // Send the user to the scan tab with a context hint.
                    scanResult = if (hostDevice != null) {
                        "Credentials expired — ask $hostDevice to show their QR"
                    } else {
                        "Credentials expired — scan a fresh QR"
                    }
                    showScanner = false
                    selectedTab = 1
                },
            )
        }
    }
}

@Composable
private fun ActiveSessionCard(
    onContinue: (activeChannel: Int) -> Unit,
    onNewSession: () -> Unit
) {
    val sessionJson = SassyTalkNative.getSessionStatus()
    val session = try { JSONObject(sessionJson) } catch (_: Exception) { null }
    // Find first active channel from per-channel status
    val channels = session?.optJSONArray("channels")
    var peerDevice = ""
    var remainingSecs = 0L
    var activeChannel = -1
    if (channels != null) {
        for (i in 0 until channels.length()) {
            val ch = channels.getJSONObject(i)
            if (ch.optBoolean("active", false)) {
                peerDevice = ch.optString("peer_device", "")
                remainingSecs = ch.optLong("remaining_seconds", 0)
                activeChannel = ch.optInt("channel", -1)
                break
            }
        }
    } else {
        // Legacy fallback
        peerDevice = session?.optString("peer_device", "") ?: ""
        remainingSecs = session?.optLong("remaining_seconds", 0) ?: 0
    }
    val hours = remainingSecs / 3600
    val mins = (remainingSecs % 3600) / 60

    Card(
        colors = CardDefaults.cardColors(containerColor = Green.copy(alpha = 0.15f)),
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.fillMaxWidth()
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Default.CheckCircle, contentDescription = null, tint = Green, modifier = Modifier.size(24.dp))
                Spacer(modifier = Modifier.width(8.dp))
                Text("Active Session", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = Green)
            }
            Spacer(modifier = Modifier.height(4.dp))
            if (peerDevice.isNotEmpty()) {
                Text("from $peerDevice", fontSize = 13.sp, color = TextGray)
            }
            Text(
                text = "${hours}h ${mins}m remaining",
                fontSize = 13.sp, color = TextGray
            )
            Spacer(modifier = Modifier.height(12.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                // Continue with existing session
                Button(
                    onClick = { onContinue(activeChannel) },
                    colors = ButtonDefaults.buttonColors(containerColor = Green, contentColor = DarkBg),
                    shape = RoundedCornerShape(25.dp),
                    modifier = Modifier.weight(1f).height(44.dp)
                ) {
                    Text("Continue", fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                }
                // Start new session
                OutlinedButton(
                    onClick = onNewSession,
                    shape = RoundedCornerShape(25.dp),
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = TextGray),
                    modifier = Modifier.weight(1f).height(44.dp)
                ) {
                    Text("New Session", fontSize = 14.sp)
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ShowQRTab(
    context: Context,
    selectedChannel: Int,
    onChannelChange: (Int) -> Unit,
    groupName: String,
    onGroupNameChange: (String) -> Unit,
    durationHours: Int,
    onDurationChange: (Int) -> Unit,
    qrBitmap: Bitmap?,
    sessionJson: String,
    onGenerate: () -> Unit,
    onContinue: () -> Unit
) {
    // No verticalScroll here: everything, Continue included, must be reachable
    // without scrolling. Continue renders ABOVE the QR so the primary action
    // is never below the fold even on short screens.
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.fillMaxWidth()
    ) {
        // Channel picker
        Card(
            colors = CardDefaults.cardColors(containerColor = CardBg),
            shape = RoundedCornerShape(12.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Text("Channel", color = TextGray, fontSize = 12.sp)
                Spacer(modifier = Modifier.height(4.dp))
                Row(
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    for (ch in 1..8) {
                        FilterChip(
                            selected = ch == selectedChannel,
                            onClick = { onChannelChange(ch) },
                            label = {
                                Text(
                                    "$ch",
                                    fontSize = 11.sp,
                                    color = if (ch == selectedChannel) DarkBg else TextGray,
                                    textAlign = TextAlign.Center,
                                    modifier = Modifier.fillMaxWidth()
                                )
                            },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = Teal,
                                containerColor = SurfaceBg
                            ),
                            modifier = Modifier
                                .weight(1f)
                                .height(28.dp)
                        )
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(6.dp))

        // Group name input
        OutlinedTextField(
            value = groupName,
            onValueChange = { if (it.length <= 30) onGroupNameChange(it) },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            placeholder = { Text("Group name (optional)", color = TextMuted) },
            colors = OutlinedTextFieldDefaults.colors(
                focusedBorderColor = Teal,
                unfocusedBorderColor = CardBg,
                focusedTextColor = TextWhite,
                unfocusedTextColor = TextWhite,
                cursorColor = Teal,
                focusedContainerColor = SurfaceBg,
                unfocusedContainerColor = SurfaceBg
            ),
            shape = RoundedCornerShape(12.dp)
        )

        Spacer(modifier = Modifier.height(6.dp))

        // Duration picker
        Card(
            colors = CardDefaults.cardColors(containerColor = CardBg),
            shape = RoundedCornerShape(12.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Text("Session Duration", color = TextGray, fontSize = 12.sp)
                Spacer(modifier = Modifier.height(4.dp))

                Row(
                    horizontalArrangement = Arrangement.SpaceEvenly,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    DurationChip("1 Day", 24, durationHours) { onDurationChange(24) }
                    DurationChip("2 Days", 48, durationHours) { onDurationChange(48) }
                    DurationChip("3 Days", 72, durationHours) { onDurationChange(72) }
                }
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        // Generate / rotate session. "Regenerate" is easy to confuse with
        // "mint another Copy Link for the same room" — rotating here mints a
        // NEW session_id and leaves any previously shared /v/… invites
        // pointing at a dead room (they still decrypt, but the host has left).
        val regenerating = qrBitmap != null
        Button(
            onClick = {
                if (regenerating) {
                    Toast.makeText(
                        context,
                        "New room created — old invite links no longer reach this device",
                        Toast.LENGTH_LONG,
                    ).show()
                }
                onGenerate()
            },
            colors = ButtonDefaults.buttonColors(containerColor = Color.Transparent),
            shape = RoundedCornerShape(25.dp),
            modifier = Modifier
                .fillMaxWidth()
                .height(44.dp)
                // Teal→blue gradient CTA (GradientAccent), matching the Tauri
                // desktop "Find Devices"/primary action; transparent container
                // lets the gradient show through the Material button surface.
                .background(Brush.linearGradient(GradientAccent), RoundedCornerShape(25.dp))
        ) {
            Icon(Icons.Default.QrCode2, contentDescription = null, modifier = Modifier.size(18.dp))
            Spacer(modifier = Modifier.width(6.dp))
            Text(
                if (regenerating) "New session (invalidates invites)" else "Generate Session QR",
                fontSize = 13.sp
            )
        }

        Spacer(modifier = Modifier.height(8.dp))

        if (qrBitmap != null) {
            // Room fingerprint: same session_id on both devices = same relay
            // room. Different /v/{shareId}#key URLs are normal (one-shot
            // dead-drops) and do NOT mean different rooms — only this id does.
            val roomId = remember(sessionJson) {
                runCatching {
                    org.json.JSONObject(sessionJson).optString("session_id", "")
                }.getOrDefault("").take(8)
            }
            if (roomId.isNotEmpty()) {
                Text(
                    text = "Room $roomId · both devices must show the same id",
                    color = Cyan,
                    fontSize = 12.sp,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(modifier = Modifier.height(6.dp))
            }

            // Continue FIRST — the primary action sits above the QR so it's
            // always visible the instant the code is generated (previously it
            // was last in a scrolling column and hid below the fold).
            Button(
                onClick = onContinue,
                colors = ButtonDefaults.buttonColors(containerColor = Green),
                shape = RoundedCornerShape(25.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp)
            ) {
                Icon(Icons.Default.CheckCircle, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(modifier = Modifier.width(6.dp))
                Text("Continue", fontSize = 14.sp)
            }

            Spacer(modifier = Modifier.height(8.dp))

            // QR display
            Card(
                colors = CardDefaults.cardColors(containerColor = androidx.compose.ui.graphics.Color.White),
                shape = RoundedCornerShape(12.dp)
            ) {
                Image(
                    bitmap = qrBitmap.asImageBitmap(),
                    contentDescription = "Session QR Code",
                    modifier = Modifier
                        .size(160.dp)
                        .padding(6.dp)
                )
            }

            Spacer(modifier = Modifier.height(6.dp))

            Text(
                text = "Scan this QR, or share a one-time link (Copy/Share mint new URLs for this same room)",
                color = TextGray,
                fontSize = 11.sp,
                textAlign = TextAlign.Center
            )

            Spacer(modifier = Modifier.height(6.dp))

            // Share / Copy row for long-distance relay
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                // Copy an encrypted one-shot share LINK to the clipboard.
                // Earlier revisions copied the raw session JSON (including
                // the AES-256 room key); that's the same plaintext leak the
                // Share button refactor was meant to close. Both buttons now
                // route through SessionShareLink so the clipboard only ever
                // holds a URL whose fragment is the only decryption material.
                var copying by remember { mutableStateOf(false) }
                val copyScope = rememberCoroutineScope()
                OutlinedButton(
                    enabled = !copying,
                    onClick = {
                        copying = true
                        copyScope.launch {
                            val result = withContext(Dispatchers.IO) {
                                SessionShareLink.createShare(sessionJson)
                            }
                            copying = false
                            when (result) {
                                is SessionShareLink.Result.Ok -> {
                                    val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                    cm.setPrimaryClip(
                                        ClipData.newPlainText(
                                            "SassyTalk Invite",
                                            // The https link is the resilient form: it's tappable in
                                            // every chat client, opens the app DIRECTLY via verified
                                            // App Links (the relay serves assetlinks.json for both
                                            // release and debug certs — confirmed verified on-device),
                                            // and on a phone without the app it lands on the /v/ viewer
                                            // page whose "Open in SassyTalk" button fires sassytalk://
                                            // with the #key intact. A bare sassytalk:// string is NOT
                                            // linkified by most messengers and silently fails to open.
                                            // Single clean link so paste never corrupts the #fragment.
                                            result.httpsUrl,
                                        ),
                                    )
                                    Toast.makeText(
                                        context,
                                        "Invite copied — one-time link for this room (not a new session)",
                                        Toast.LENGTH_LONG,
                                    ).show()
                                }
                                is SessionShareLink.Result.Err -> {
                                    Toast.makeText(context, "Copy failed: ${result.message}", Toast.LENGTH_LONG).show()
                                }
                            }
                        }
                    },
                    shape = RoundedCornerShape(25.dp),
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                    modifier = Modifier.weight(1f).height(36.dp)
                ) {
                    if (copying) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(14.dp),
                            color = Cyan,
                            strokeWidth = 2.dp,
                        )
                    } else {
                        Icon(Icons.Default.ContentCopy, contentDescription = null, modifier = Modifier.size(16.dp))
                    }
                    Spacer(modifier = Modifier.width(4.dp))
                    Text(if (copying) "Linking…" else "Copy Link", fontSize = 12.sp)
                }

                // Share as an end-to-end encrypted one-shot link. The session
                // JSON is AES-GCM encrypted with a fresh key that lives only
                // in the URL fragment; the relay sees ciphertext.
                var sharing by remember { mutableStateOf(false) }
                val scope = rememberCoroutineScope()
                OutlinedButton(
                    enabled = !sharing,
                    onClick = {
                        sharing = true
                        scope.launch {
                            val result = withContext(Dispatchers.IO) {
                                SessionShareLink.createShare(sessionJson)
                            }
                            sharing = false
                            when (result) {
                                is SessionShareLink.Result.Ok -> {
                                    val intent = Intent(Intent.ACTION_SEND).apply {
                                        type = "text/plain"
                                        putExtra(
                                            Intent.EXTRA_TEXT,
                                            // The https link is tappable in every messenger and opens
                                            // the app directly via verified App Links; without the app
                                            // it lands on the /v/ viewer page (Open-in-app + install +
                                            // copy/paste fallbacks). sassytalk:// is omitted: clients
                                            // don't linkify custom schemes, so it reads as dead text.
                                            "Join my SassyTalk session:\n${result.httpsUrl}\n\n" +
                                                "One-time encrypted invite, expires shortly. If it opens a " +
                                                "web page, tap \"Open in SassyTalk\" there — or paste the " +
                                                "link in SassyTalk → Authenticate → Enter Code.",
                                        )
                                        putExtra(Intent.EXTRA_SUBJECT, "SassyTalk invite")
                                    }
                                    context.startActivity(
                                        Intent.createChooser(intent, "Share invite link"),
                                    )
                                }
                                is SessionShareLink.Result.Err -> {
                                    Toast.makeText(
                                        context,
                                        "Share failed: ${result.message}",
                                        Toast.LENGTH_LONG,
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
                    if (sharing) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(14.dp),
                            color = Cyan,
                            strokeWidth = 2.dp,
                        )
                    } else {
                        Icon(Icons.Default.Share, contentDescription = null, modifier = Modifier.size(16.dp))
                    }
                    Spacer(modifier = Modifier.width(4.dp))
                    Text(if (sharing) "Sharing…" else "Share Link", fontSize = 12.sp)
                }
            }

        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DurationChip(
    label: String,
    hours: Int,
    selectedHours: Int,
    onClick: () -> Unit
) {
    val isSelected = hours == selectedHours
    FilterChip(
        selected = isSelected,
        onClick = onClick,
        label = { Text(label, color = if (isSelected) DarkBg else TextGray) },
        colors = FilterChipDefaults.filterChipColors(
            selectedContainerColor = Cyan,
            containerColor = SurfaceBg
        )
    )
}

@Composable
private fun ScanQRTab(
    scanResult: String?,
    showScanner: Boolean,
    onStartScan: () -> Unit,
    onQRScanned: (String) -> Unit
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.fillMaxWidth()
    ) {
        if (showScanner) {
            // Camera preview for QR scanning
            QRScannerView(
                onQRScanned = onQRScanned,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(400.dp)
                    .clip(RoundedCornerShape(16.dp))
            )
        } else {
            // Scan button
            Spacer(modifier = Modifier.height(40.dp))

            Icon(
                Icons.Default.QrCodeScanner,
                contentDescription = null,
                tint = Cyan,
                modifier = Modifier.size(80.dp)
            )

            Spacer(modifier = Modifier.height(24.dp))

            Button(
                onClick = onStartScan,
                colors = ButtonDefaults.buttonColors(containerColor = Cyan),
                shape = RoundedCornerShape(25.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Icon(Icons.Default.CameraAlt, contentDescription = null, tint = DarkBg)
                Spacer(modifier = Modifier.width(8.dp))
                Text("Open Scanner", fontSize = 16.sp, color = DarkBg)
            }
        }

        if (scanResult != null) {
            Spacer(modifier = Modifier.height(16.dp))
            Text(
                text = scanResult,
                color = if (scanResult.contains("established")) Green else StatusDisconnected,
                fontSize = 16.sp,
                fontWeight = FontWeight.Medium
            )
        }
    }
}

@Composable
private fun EnterCodeTab(
    context: Context,
    onAuthenticated: () -> Unit
) {
    var codeText by remember { mutableStateOf("") }
    var result by remember { mutableStateOf<String?>(null) }
    var joining by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.fillMaxWidth()
    ) {
        Spacer(modifier = Modifier.height(16.dp))

        Icon(
            Icons.Default.Key,
            contentDescription = null,
            tint = Cyan,
            modifier = Modifier.size(48.dp)
        )

        Spacer(modifier = Modifier.height(12.dp))

        Text(
            text = "Paste a session code or invite link from another device",
            color = TextGray,
            fontSize = 14.sp,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(20.dp))

        // Code input field
        OutlinedTextField(
            value = codeText,
            onValueChange = { codeText = it },
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 120.dp),
            placeholder = {
                Text(
                    "Paste invite link or session JSON…",
                    color = TextMuted,
                    fontSize = 13.sp,
                )
            },
            colors = OutlinedTextFieldDefaults.colors(
                focusedBorderColor = Teal,
                unfocusedBorderColor = CardBg,
                focusedTextColor = TextWhite,
                unfocusedTextColor = TextWhite,
                cursorColor = Teal,
                focusedContainerColor = SurfaceBg,
                unfocusedContainerColor = SurfaceBg
            ),
            shape = RoundedCornerShape(12.dp),
            maxLines = 6
        )

        Spacer(modifier = Modifier.height(12.dp))

        // Paste from clipboard button
        OutlinedButton(
            onClick = {
                val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                val clip = cm.primaryClip
                if (clip != null && clip.itemCount > 0) {
                    codeText = clip.getItemAt(0).text?.toString() ?: ""
                }
            },
            shape = RoundedCornerShape(25.dp),
            colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
            modifier = Modifier.fillMaxWidth().height(44.dp)
        ) {
            Icon(Icons.Default.ContentPaste, contentDescription = null, modifier = Modifier.size(18.dp))
            Spacer(modifier = Modifier.width(8.dp))
            Text("Paste from Clipboard", fontSize = 14.sp)
        }

        Spacer(modifier = Modifier.height(16.dp))

        // Join button — accepts raw QR JSON or an encrypted https invite link.
        Button(
            onClick = {
                val trimmed = codeText.trim()
                if (trimmed.isEmpty()) {
                    result = "Please paste a session code or invite link first"
                    return@Button
                }
                joining = true
                scope.launch {
                    val success = if (SessionShareLink.looksLikeShareLink(trimmed)) {
                        when (val share = withContext(Dispatchers.IO) {
                            SessionShareLink.importFromShareText(trimmed)
                        }) {
                            is SessionShareLink.Result.Ok -> withContext(Dispatchers.IO) {
                                SassyTalkNative.importSessionFromQR(share.json)
                            }
                            is SessionShareLink.Result.Err -> {
                                result = share.message
                                joining = false
                                return@launch
                            }
                        }
                    } else {
                        SassyTalkNative.importSessionFromQR(trimmed)
                    }
                    joining = false
                    val roomHint = SassyTalkNative.getSessionId()?.take(8).orEmpty()
                    result = when {
                        success && roomHint.isNotEmpty() ->
                            "Joined room $roomHint — must match the host"
                        success -> "Session established!"
                        else -> "Invalid session code"
                    }
                    if (success) {
                        onAuthenticated()
                    }
                }
            },
            colors = ButtonDefaults.buttonColors(containerColor = Color.Transparent),
            shape = RoundedCornerShape(25.dp),
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
                .background(Brush.linearGradient(GradientAccent), RoundedCornerShape(25.dp)),
            enabled = codeText.isNotBlank() && !joining
        ) {
            if (joining) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    color = Color.White,
                    strokeWidth = 2.dp,
                )
            } else {
                Icon(Icons.Default.VpnKey, contentDescription = null)
            }
            Spacer(modifier = Modifier.width(8.dp))
            Text(if (joining) "Joining…" else "Join Session", fontSize = 16.sp)
        }

        if (result != null) {
            Spacer(modifier = Modifier.height(12.dp))
            Text(
                text = result!!,
                color = if (result!!.contains("established")) Green else StatusDisconnected,
                fontSize = 15.sp,
                fontWeight = FontWeight.Medium
            )
        }
    }
}

