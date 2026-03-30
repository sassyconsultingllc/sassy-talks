package com.sassyconsulting.sassytalkie.ui

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.BuildConfig
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import com.sassyconsulting.sassytalkie.WhisperModelManager
import com.sassyconsulting.sassytalkie.ui.theme.*

@Composable
fun AboutScreen(
    onBack: () -> Unit
) {
    val context = LocalContext.current
    val scrollState = rememberScrollState()

    // Gather live status
    val transportName = remember { SassyTalkNative.getTransportName() }
    val isEncrypted = remember { SassyTalkNative.isEncrypted() }
    val whisperReady = TranscriptionBridge.whisperReady
    val transcriptionEnabled = remember { mutableStateOf(TranscriptionBridge.isEnabled()) }
    val modelState by WhisperModelManager.state.collectAsState()
    var selectedModel by remember { mutableStateOf(WhisperModelManager.currentModel) }

    // Check device RAM to filter available models
    val totalRamMb = remember {
        val am = context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager
        val memInfo = ActivityManager.MemoryInfo()
        am.getMemoryInfo(memInfo)
        (memInfo.totalMem / (1024 * 1024)).toInt()
    }
    // Base needs ~400MB free, Small ~700MB, Medium ~2GB, Large ~4GB
    val availableModels = remember(totalRamMb) {
        WhisperModelManager.ModelSize.entries.filter { model ->
            when (model) {
                WhisperModelManager.ModelSize.BASE_EN -> totalRamMb >= 3000
                WhisperModelManager.ModelSize.SMALL_EN -> totalRamMb >= 4000
                WhisperModelManager.ModelSize.MEDIUM_EN -> totalRamMb >= 6000
                WhisperModelManager.ModelSize.LARGE_V3 -> totalRamMb >= 8000
            }
        }
    }
    val lowRamDevice = availableModels.isEmpty()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DarkBg)
            .padding(16.dp)
    ) {
        // Header
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.Default.ArrowBack, contentDescription = "Back", tint = TextGray)
            }

            Text(
                text = "About",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )

            Spacer(modifier = Modifier.width(48.dp))
        }

        Spacer(modifier = Modifier.height(16.dp))

        Column(
            modifier = Modifier
                .weight(1f)
                .verticalScroll(scrollState),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            // App identity
            Text(
                text = "\uD83D\uDCFB",
                fontSize = 56.sp
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = "SassyTalk",
                fontSize = 28.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )
            Text(
                text = "Encrypted Push-to-Talk",
                fontSize = 14.sp,
                color = Cyan
            )
            Text(
                text = "v${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})",
                fontSize = 12.sp,
                color = TextMuted
            )

            Spacer(modifier = Modifier.height(24.dp))

            // Connection status card
            StatusCard(title = "Connection Status") {
                StatusRow(
                    icon = Icons.Default.Wifi,
                    label = "Transport",
                    value = transportName,
                    valueColor = if (transportName != "---") StatusConnected else StatusDisconnected
                )
                StatusRow(
                    icon = Icons.Default.Lock,
                    label = "Encryption",
                    value = if (isEncrypted) "AES-256-GCM" else "Not active",
                    valueColor = if (isEncrypted) StatusConnected else StatusDisconnected
                )
                StatusRow(
                    icon = Icons.Default.Subtitles,
                    label = "Whisper Model",
                    value = when (modelState) {
                        is WhisperModelManager.ModelState.Ready -> "${WhisperModelManager.currentModel.label} - Ready"
                        is WhisperModelManager.ModelState.Downloading -> "Downloading..."
                        is WhisperModelManager.ModelState.Error -> "Error"
                        else -> "Not loaded"
                    },
                    valueColor = when (modelState) {
                        is WhisperModelManager.ModelState.Ready -> StatusConnected
                        is WhisperModelManager.ModelState.Downloading -> Orange
                        is WhisperModelManager.ModelState.Error -> StatusDisconnected
                        else -> TextMuted
                    }
                )

                // Transcription toggle
                Spacer(modifier = Modifier.height(8.dp))
                if (lowRamDevice) {
                    Text(
                        text = "Transcription unavailable — device has insufficient RAM (${totalRamMb}MB)",
                        fontSize = 12.sp,
                        color = TextMuted
                    )
                } else {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text("Transcription", fontSize = 13.sp, color = TextGray, modifier = Modifier.weight(1f))
                        Switch(
                            checked = transcriptionEnabled.value,
                            onCheckedChange = { enabled ->
                                if (enabled && !whisperReady) {
                                    WhisperModelManager.ensureReady(context)
                                }
                                TranscriptionBridge.setEnabled(enabled)
                                transcriptionEnabled.value = enabled
                            },
                            colors = SwitchDefaults.colors(
                                checkedThumbColor = Orange,
                                checkedTrackColor = Orange.copy(alpha = 0.3f),
                                uncheckedThumbColor = TextMuted,
                                uncheckedTrackColor = SurfaceBg
                            )
                        )
                    }
                }

                // Download progress bar
                if (modelState is WhisperModelManager.ModelState.Downloading) {
                    val progress = (modelState as WhisperModelManager.ModelState.Downloading).progress
                    Spacer(modifier = Modifier.height(6.dp))
                    LinearProgressIndicator(
                        progress = progress,
                        modifier = Modifier.fillMaxWidth().height(6.dp),
                        color = Orange,
                        trackColor = SurfaceBg,
                    )
                    Text(
                        text = "Downloading ${selectedModel.label}... ${(progress * 100).toInt()}%",
                        fontSize = 11.sp,
                        color = TextMuted,
                        modifier = Modifier.padding(top = 4.dp)
                    )
                }

                if (modelState is WhisperModelManager.ModelState.Error) {
                    val err = (modelState as WhisperModelManager.ModelState.Error).message
                    Text(text = "Error: $err", fontSize = 11.sp, color = StatusDisconnected, modifier = Modifier.padding(top = 4.dp))
                }

                // Model selector — vertical list
                Spacer(modifier = Modifier.height(8.dp))
                Text("Select Model", fontSize = 12.sp, color = TextMuted)
                Spacer(modifier = Modifier.height(4.dp))
                availableModels.forEach { model ->
                    val isSelected = model == WhisperModelManager.currentModel
                    val isDownloading = modelState is WhisperModelManager.ModelState.Downloading
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(8.dp))
                            .background(if (isSelected) SurfaceBg else Color.Transparent)
                            .clickable(enabled = !isDownloading) {
                                selectedModel = model
                                WhisperModelManager.switchModel(context, model)
                            }
                            .padding(horizontal = 12.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Icon(
                            if (isSelected) Icons.Default.RadioButtonChecked else Icons.Default.RadioButtonUnchecked,
                            contentDescription = null,
                            tint = if (isSelected) Orange else TextMuted,
                            modifier = Modifier.size(20.dp)
                        )
                        Spacer(modifier = Modifier.width(10.dp))
                        Text(
                            text = model.label,
                            fontSize = 14.sp,
                            color = if (isSelected) Orange else TextGray,
                            fontWeight = if (isSelected) FontWeight.SemiBold else FontWeight.Normal
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Permissions card
            StatusCard(title = "Permissions") {
                PermissionRow(label = "Microphone", reason = "Capture voice for push-to-talk")
                PermissionRow(label = "Camera", reason = "Scan QR codes for session auth")
                PermissionRow(label = "Bluetooth", reason = "Discover and connect to nearby peers")
                PermissionRow(label = "Internet", reason = "Cellular relay for cross-network comms")
                PermissionRow(label = "Foreground Service", reason = "Keep radio active when screen is off")
                PermissionRow(label = "Notifications", reason = "Transcription alerts when backgrounded")
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Links card
            StatusCard(title = "Legal & Info") {
                LinkRow(
                    icon = Icons.Default.Shield,
                    label = "Privacy Policy",
                    onClick = {
                        val intent = Intent(Intent.ACTION_VIEW, Uri.parse("https://sassyconsultingllc.com/privacy/sassytalk/"))
                        context.startActivity(intent)
                    }
                )
                LinkRow(
                    icon = Icons.Default.Language,
                    label = "Website",
                    onClick = {
                        val intent = Intent(Intent.ACTION_VIEW, Uri.parse("https://sassyconsultingllc.com"))
                        context.startActivity(intent)
                    }
                )
                LinkRow(
                    icon = Icons.Default.Email,
                    label = "Contact Support",
                    onClick = {
                        val intent = Intent(Intent.ACTION_SENDTO, Uri.parse("mailto:support@sassyconsulting.com"))
                        context.startActivity(intent)
                    }
                )
            }

            Spacer(modifier = Modifier.height(24.dp))

            // Credits
            Text(
                text = "Built by Sassy Consulting LLC",
                fontSize = 13.sp,
                color = TextMuted
            )
            Text(
                text = "\u00A9 2025-2026 All rights reserved",
                fontSize = 11.sp,
                color = TextMuted.copy(alpha = 0.6f)
            )

            Spacer(modifier = Modifier.height(16.dp))
        }
    }
}

@Composable
private fun StatusCard(
    title: String,
    content: @Composable ColumnScope.() -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = CardBg),
        shape = RoundedCornerShape(12.dp)
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = title,
                fontSize = 14.sp,
                fontWeight = FontWeight.SemiBold,
                color = Cyan,
                letterSpacing = 1.sp
            )
            Spacer(modifier = Modifier.height(12.dp))
            content()
        }
    }
}

@Composable
private fun StatusRow(
    icon: ImageVector,
    label: String,
    value: String,
    valueColor: Color
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Icon(
            icon,
            contentDescription = null,
            tint = TextMuted,
            modifier = Modifier.size(18.dp)
        )
        Spacer(modifier = Modifier.width(10.dp))
        Text(
            text = label,
            fontSize = 13.sp,
            color = TextGray,
            modifier = Modifier.weight(1f)
        )
        Box(
            modifier = Modifier
                .size(8.dp)
                .clip(CircleShape)
                .background(valueColor)
        )
        Spacer(modifier = Modifier.width(6.dp))
        Text(
            text = value,
            fontSize = 13.sp,
            color = valueColor,
            fontWeight = FontWeight.Medium
        )
    }
}

@Composable
private fun PermissionRow(label: String, reason: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 3.dp),
        verticalAlignment = Alignment.Top
    ) {
        Icon(
            Icons.Default.CheckCircle,
            contentDescription = null,
            tint = StatusConnected.copy(alpha = 0.7f),
            modifier = Modifier.size(16.dp).padding(top = 2.dp)
        )
        Spacer(modifier = Modifier.width(8.dp))
        Column {
            Text(text = label, fontSize = 13.sp, color = TextGray, fontWeight = FontWeight.Medium)
            Text(text = reason, fontSize = 11.sp, color = TextMuted)
        }
    }
}

@Composable
private fun LinkRow(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Icon(icon, contentDescription = null, tint = Cyan, modifier = Modifier.size(20.dp))
        Spacer(modifier = Modifier.width(10.dp))
        Text(
            text = label,
            fontSize = 14.sp,
            color = Cyan,
            modifier = Modifier.weight(1f)
        )
        Icon(
            Icons.Default.ChevronRight,
            contentDescription = null,
            tint = TextMuted,
            modifier = Modifier.size(18.dp)
        )
    }
}
