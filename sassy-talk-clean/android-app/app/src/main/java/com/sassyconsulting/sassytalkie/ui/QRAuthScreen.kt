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
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*
import org.json.JSONObject

@Composable
fun QRAuthScreen(onAuthenticated: () -> Unit) {
    val context = LocalContext.current
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
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Spacer(modifier = Modifier.height(24.dp))

        // Title
        Text(
            text = "Authenticate",
            fontSize = 28.sp,
            fontWeight = FontWeight.Bold,
            color = Orange
        )

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = "Scan a QR code or share a session code",
            fontSize = 14.sp,
            color = TextGray,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(16.dp))

        // ── Existing session card ──
        if (hasExistingSession.value) {
            ActiveSessionCard(
                onContinue = onAuthenticated,
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
            contentColor = Orange,
            edgePadding = 0.dp,
            modifier = Modifier.clip(RoundedCornerShape(12.dp))
        ) {
            Tab(
                selected = selectedTab == 0,
                onClick = { selectedTab = 0 },
                text = { Text("Show QR", color = if (selectedTab == 0) Orange else TextGray, fontSize = 13.sp) }
            )
            Tab(
                selected = selectedTab == 1,
                onClick = { selectedTab = 1 },
                text = { Text("Scan QR", color = if (selectedTab == 1) Orange else TextGray, fontSize = 13.sp) }
            )
            Tab(
                selected = selectedTab == 2,
                onClick = { selectedTab = 2 },
                text = { Text("Enter Code", color = if (selectedTab == 2) Orange else TextGray, fontSize = 13.sp) }
            )
            Tab(
                selected = selectedTab == 3,
                onClick = { selectedTab = 3 },
                text = { Text("My Cohorts", color = if (selectedTab == 3) Orange else TextGray, fontSize = 13.sp) }
            )
        }

        Spacer(modifier = Modifier.height(24.dp))

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
                        qrBitmap = generateQRBitmap(qrJson, 600)
                        hasExistingSession.value = true
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
                    scanResult = if (success) "Session established!" else "Invalid QR code"
                    if (success) {
                        hasExistingSession.value = true
                        onAuthenticated()
                    }
                }
            )
            2 -> EnterCodeTab(
                context = context,
                onAuthenticated = {
                    hasExistingSession.value = true
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
                        qrBitmap = generateQRBitmap(qr, 600)
                        hasExistingSession.value = true
                        selectedTab = 0
                    }
                },
                onRejoinJoiner = { hostDevice ->
                    scanResult = if (hostDevice != null) "Ask $hostDevice to show their QR" else null
                    showScanner = false
                    selectedTab = 1
                },
            )
        }
    }
}

@Composable
private fun ActiveSessionCard(
    onContinue: () -> Unit,
    onNewSession: () -> Unit
) {
    val sessionJson = SassyTalkNative.getSessionStatus()
    val session = try { JSONObject(sessionJson) } catch (_: Exception) { null }
    // Find first active channel from per-channel status
    val channels = session?.optJSONArray("channels")
    var peerDevice = ""
    var remainingSecs = 0L
    if (channels != null) {
        for (i in 0 until channels.length()) {
            val ch = channels.getJSONObject(i)
            if (ch.optBoolean("active", false)) {
                peerDevice = ch.optString("peer_device", "")
                remainingSecs = ch.optLong("remaining_seconds", 0)
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
                    onClick = onContinue,
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
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier
            .fillMaxWidth()
            .verticalScroll(rememberScrollState())
    ) {
        // Channel picker
        Card(
            colors = CardDefaults.cardColors(containerColor = CardBg),
            shape = RoundedCornerShape(12.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Text("Channel", color = TextGray, fontSize = 14.sp)
                Spacer(modifier = Modifier.height(8.dp))
                Row(
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                ) {
                    for (ch in 1..8) {
                        FilterChip(
                            selected = ch == selectedChannel,
                            onClick = { onChannelChange(ch) },
                            label = { Text("$ch", fontSize = 13.sp, color = if (ch == selectedChannel) DarkBg else TextGray) },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = Orange,
                                containerColor = SurfaceBg
                            )
                        )
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(12.dp))

        // Group name input
        OutlinedTextField(
            value = groupName,
            onValueChange = { if (it.length <= 30) onGroupNameChange(it) },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            placeholder = { Text("Group name (optional)", color = TextMuted) },
            colors = OutlinedTextFieldDefaults.colors(
                focusedBorderColor = Orange,
                unfocusedBorderColor = CardBg,
                focusedTextColor = TextWhite,
                unfocusedTextColor = TextWhite,
                cursorColor = Orange,
                focusedContainerColor = SurfaceBg,
                unfocusedContainerColor = SurfaceBg
            ),
            shape = RoundedCornerShape(12.dp)
        )

        Spacer(modifier = Modifier.height(12.dp))

        // Duration picker
        Card(
            colors = CardDefaults.cardColors(containerColor = CardBg),
            shape = RoundedCornerShape(12.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Text("Session Duration", color = TextGray, fontSize = 14.sp)
                Spacer(modifier = Modifier.height(12.dp))

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

        Spacer(modifier = Modifier.height(24.dp))

        // Generate button
        Button(
            onClick = onGenerate,
            colors = ButtonDefaults.buttonColors(containerColor = Orange),
            shape = RoundedCornerShape(25.dp),
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            Icon(Icons.Default.QrCode2, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text(
                if (qrBitmap != null) "Regenerate QR" else "Generate Session QR",
                fontSize = 16.sp
            )
        }

        Spacer(modifier = Modifier.height(24.dp))

        // QR display
        if (qrBitmap != null) {
            Card(
                colors = CardDefaults.cardColors(containerColor = androidx.compose.ui.graphics.Color.White),
                shape = RoundedCornerShape(16.dp)
            ) {
                Image(
                    bitmap = qrBitmap.asImageBitmap(),
                    contentDescription = "Session QR Code",
                    modifier = Modifier
                        .size(200.dp)
                        .padding(12.dp)
                )
            }

            Spacer(modifier = Modifier.height(12.dp))

            Text(
                text = "Scan this QR, or share the code for long-distance relay",
                color = TextGray,
                fontSize = 13.sp,
                textAlign = TextAlign.Center
            )

            Spacer(modifier = Modifier.height(12.dp))

            // Share / Copy row for long-distance relay
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                // Copy to clipboard
                OutlinedButton(
                    onClick = {
                        val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        cm.setPrimaryClip(ClipData.newPlainText("SassyTalk Session", sessionJson))
                        Toast.makeText(context, "Session code copied", Toast.LENGTH_SHORT).show()
                    },
                    shape = RoundedCornerShape(25.dp),
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
                    modifier = Modifier.weight(1f).height(44.dp)
                ) {
                    Icon(Icons.Default.ContentCopy, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(modifier = Modifier.width(6.dp))
                    Text("Copy Code", fontSize = 13.sp)
                }

                // Share via intent (text, Signal, WhatsApp, etc.)
                OutlinedButton(
                    onClick = {
                        val intent = Intent(Intent.ACTION_SEND).apply {
                            type = "text/plain"
                            putExtra(Intent.EXTRA_TEXT, sessionJson)
                            putExtra(Intent.EXTRA_SUBJECT, "SassyTalk Session Code")
                        }
                        context.startActivity(Intent.createChooser(intent, "Share session code"))
                    },
                    shape = RoundedCornerShape(25.dp),
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
                    modifier = Modifier.weight(1f).height(44.dp)
                ) {
                    Icon(Icons.Default.Share, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(modifier = Modifier.width(6.dp))
                    Text("Share", fontSize = 13.sp)
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Continue button
            Button(
                onClick = onContinue,
                colors = ButtonDefaults.buttonColors(containerColor = Green),
                shape = RoundedCornerShape(25.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Icon(Icons.Default.CheckCircle, contentDescription = null)
                Spacer(modifier = Modifier.width(8.dp))
                Text("Continue", fontSize = 15.sp)
            }

            Spacer(modifier = Modifier.height(8.dp))

            Text(
                text = "Your session key is active — the other device just needs to scan or enter the code.",
                color = TextMuted,
                fontSize = 12.sp,
                textAlign = TextAlign.Center
            )
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
            text = "Paste a session code received from another device",
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
            placeholder = { Text("Paste session code here...", color = TextMuted) },
            colors = OutlinedTextFieldDefaults.colors(
                focusedBorderColor = Orange,
                unfocusedBorderColor = CardBg,
                focusedTextColor = TextWhite,
                unfocusedTextColor = TextWhite,
                cursorColor = Orange,
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

        // Join button
        Button(
            onClick = {
                val trimmed = codeText.trim()
                if (trimmed.isEmpty()) {
                    result = "Please paste a session code first"
                    return@Button
                }
                val success = SassyTalkNative.importSessionFromQR(trimmed)
                result = if (success) "Session established!" else "Invalid session code"
                if (success) {
                    onAuthenticated()
                }
            },
            colors = ButtonDefaults.buttonColors(containerColor = Orange),
            shape = RoundedCornerShape(25.dp),
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp),
            enabled = codeText.isNotBlank()
        ) {
            Icon(Icons.Default.VpnKey, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Join Session", fontSize = 16.sp)
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

/** Generate a QR code bitmap from a string */
private fun generateQRBitmap(content: String, size: Int): Bitmap? {
    return try {
        val writer = QRCodeWriter()
        val bitMatrix = writer.encode(content, BarcodeFormat.QR_CODE, size, size)
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.RGB_565)
        for (x in 0 until size) {
            for (y in 0 until size) {
                bitmap.setPixel(x, y, if (bitMatrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE)
            }
        }
        bitmap
    } catch (e: Exception) {
        null
    }
}
