package com.sassyconsulting.sassytalkie.ui

import android.graphics.Bitmap
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*

@Composable
fun QRAuthScreen(onAuthenticated: () -> Unit) {
    var selectedTab by remember { mutableIntStateOf(0) }
    var durationHours by remember { mutableIntStateOf(24) }
    var qrBitmap by remember { mutableStateOf<Bitmap?>(null) }
    var scanResult by remember { mutableStateOf<String?>(null) }
    var showScanner by remember { mutableStateOf(false) }

    // Check if already authenticated
    LaunchedEffect(Unit) {
        if (SassyTalkNative.isAuthenticated()) {
            onAuthenticated()
        }
    }

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
            text = "Scan a QR code to establish a secure session",
            fontSize = 14.sp,
            color = TextGray,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(24.dp))

        // Tab row
        TabRow(
            selectedTabIndex = selectedTab,
            containerColor = CardBg,
            contentColor = Orange,
            modifier = Modifier.clip(RoundedCornerShape(12.dp))
        ) {
            Tab(
                selected = selectedTab == 0,
                onClick = { selectedTab = 0 },
                text = { Text("Show My QR", color = if (selectedTab == 0) Orange else TextGray) }
            )
            Tab(
                selected = selectedTab == 1,
                onClick = { selectedTab = 1 },
                text = { Text("Scan QR", color = if (selectedTab == 1) Orange else TextGray) }
            )
        }

        Spacer(modifier = Modifier.height(24.dp))

        when (selectedTab) {
            0 -> ShowQRTab(
                durationHours = durationHours,
                onDurationChange = { durationHours = it },
                qrBitmap = qrBitmap,
                onGenerate = {
                    val qrJson = SassyTalkNative.generateSessionQR(durationHours)
                    if (qrJson.isNotEmpty()) {
                        qrBitmap = generateQRBitmap(qrJson, 600)
                    }
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
                        onAuthenticated()
                    }
                }
            )
        }
    }
}

@Composable
private fun ShowQRTab(
    durationHours: Int,
    onDurationChange: (Int) -> Unit,
    qrBitmap: Bitmap?,
    onGenerate: () -> Unit
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.fillMaxWidth()
    ) {
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
            Text("Generate Session QR", fontSize = 16.sp)
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
                        .size(280.dp)
                        .padding(16.dp)
                )
            }

            Spacer(modifier = Modifier.height(12.dp))

            Text(
                text = "Have the other device scan this QR code",
                color = TextGray,
                fontSize = 13.sp,
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
