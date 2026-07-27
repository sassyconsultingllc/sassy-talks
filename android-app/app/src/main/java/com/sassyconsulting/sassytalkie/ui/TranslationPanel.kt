// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-R75DJW7J6RNN
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Translate
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.translate.LiveCaptionTranslator
import com.sassyconsulting.sassytalkie.translate.LiveTranslationBridge
import com.sassyconsulting.sassytalkie.translate.LiveTranslationText
import com.sassyconsulting.sassytalkie.translate.TranslationManager
import com.sassyconsulting.sassytalkie.ui.theme.*

/**
 * Settings panel for offline live-caption + translation of the LOCAL speaker.
 *
 * Configures the app-scoped [LiveTranslationBridge] (enable, source/target
 * languages, Wi-Fi-only model downloads). Recognition keeps running on the
 * main radio screen after leaving Settings so captions stay available in use.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TranslationPanel(
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    // Ensure bridge is ready even if Activity init raced composition.
    LaunchedEffect(Unit) { LiveTranslationBridge.init(context) }
    DisposableEffect(Unit) {
        LiveTranslationBridge.acquireUi()
        onDispose { LiveTranslationBridge.releaseUi() }
    }

    val enabled by LiveTranslationBridge.enabled.collectAsState()
    val sourceLang by LiveTranslationBridge.sourceLang.collectAsState()
    val targetLang by LiveTranslationBridge.targetLang.collectAsState()
    val wifiOnly by LiveTranslationBridge.wifiOnlyModels.collectAsState()
    val ttsEnabled by LiveTranslationBridge.ttsEnabled.collectAsState()
    val timelineEnabled by LiveTranslationBridge.timelineEnabled.collectAsState()
    val pausedForPtt by LiveTranslationBridge.pausedForPtt.collectAsState()
    val caption by LiveTranslationBridge.caption.collectAsState()
    val translation by LiveTranslationBridge.translation.collectAsState()
    val status by LiveTranslationBridge.status.collectAsState()
    val errorMessage by LiveTranslationBridge.errorMessage.collectAsState()
    val modelState by LiveTranslationBridge.modelState.collectAsState()
    val downloadedModels by LiveTranslationBridge.downloadedModels.collectAsState()

    var sourceMenuExpanded by remember { mutableStateOf(false) }
    var targetMenuExpanded by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        LiveTranslationBridge.refreshDownloadedModels()
    }
    LaunchedEffect(enabled) {
        if (enabled) LiveTranslationBridge.refreshDownloadedModels()
    }

    val sourceLabel = remember(sourceLang) {
        TranslationManager.COMMON_LANGUAGES.firstOrNull { it.code == sourceLang }?.label ?: sourceLang
    }
    val targetLabel = remember(targetLang) {
        TranslationManager.COMMON_LANGUAGES.firstOrNull { it.code == targetLang }?.label ?: targetLang
    }

    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = CardBg),
        shape = RoundedCornerShape(12.dp),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Default.Translate,
                    contentDescription = null,
                    tint = Cyan,
                    modifier = Modifier.size(20.dp),
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = "Live Translation",
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = Cyan,
                    letterSpacing = 1.sp,
                    modifier = Modifier.weight(1f),
                )
                Switch(
                    checked = enabled,
                    onCheckedChange = { LiveTranslationBridge.setEnabled(it) },
                    colors = SwitchDefaults.colors(
                        checkedThumbColor = Orange,
                        checkedTrackColor = Orange.copy(alpha = 0.3f),
                        uncheckedThumbColor = TextMuted,
                        uncheckedTrackColor = SurfaceBg,
                    ),
                )
            }

            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Captions your speech on-device and translates it on the radio screen. Runs offline; pauses while you transmit and releases the mic when the app is backgrounded.",
                fontSize = 11.sp,
                color = TextMuted,
            )

            Spacer(modifier = Modifier.height(12.dp))

            LanguagePicker(
                label = "I speak",
                selectedLabel = sourceLabel,
                expanded = sourceMenuExpanded,
                onExpandedChange = { sourceMenuExpanded = it },
                downloadedModels = downloadedModels,
                onSelect = { code ->
                    LiveTranslationBridge.setSourceLang(code)
                    // Keep source ≠ target so captions actually translate.
                    if (code == targetLang) {
                        val fallback = TranslationManager.COMMON_LANGUAGES
                            .firstOrNull { it.code != code }?.code
                        if (fallback != null) LiveTranslationBridge.setTargetLang(fallback)
                    }
                    sourceMenuExpanded = false
                },
            )

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 4.dp),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(
                    onClick = { LiveTranslationBridge.swapLanguages() },
                    enabled = sourceLang != targetLang,
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 0.dp),
                ) {
                    Text(
                        text = "⇅ Swap languages",
                        fontSize = 12.sp,
                        color = if (sourceLang != targetLang) TealLight else TextMuted,
                    )
                }
            }

            LanguagePicker(
                label = "Translate to",
                selectedLabel = targetLabel,
                expanded = targetMenuExpanded,
                onExpandedChange = { targetMenuExpanded = it },
                excludeCode = sourceLang,
                downloadedModels = downloadedModels,
                onSelect = { code ->
                    LiveTranslationBridge.setTargetLang(code)
                    targetMenuExpanded = false
                },
            )

            if (sourceLang == targetLang) {
                Spacer(modifier = Modifier.height(6.dp))
                Text(
                    text = "Pick a different target language to translate — source and target match.",
                    fontSize = 11.sp,
                    color = StatusWarning,
                )
            }

            if (modelState == TranslationManager.ModelState.DOWNLOADING) {
                Spacer(modifier = Modifier.height(10.dp))
                ModelDownloadBanner(
                    sourceLabel = sourceLabel,
                    targetLabel = targetLabel,
                )
            }

            Spacer(modifier = Modifier.height(10.dp))

            PrefToggle(
                title = "Wi-Fi only downloads",
                description = "Language models (~30 MB each) download only on unmetered networks",
                checked = wifiOnly,
                onCheckedChange = { LiveTranslationBridge.setWifiOnlyModels(it) },
                accent = Cyan,
            )

            Spacer(modifier = Modifier.height(8.dp))

            PrefToggle(
                title = "Save to Timeline",
                description = "Append final captions and translations to the Activity timeline",
                checked = timelineEnabled,
                onCheckedChange = { LiveTranslationBridge.setTimelineEnabled(it) },
                accent = Cyan,
            )

            Spacer(modifier = Modifier.height(8.dp))

            PrefToggle(
                title = "Speak translation",
                description = "Read back translated finals with on-device TTS (muted during PTT and while a peer is speaking)",
                checked = ttsEnabled,
                onCheckedChange = { LiveTranslationBridge.setTtsEnabled(it) },
                accent = Orange,
            )

            Spacer(modifier = Modifier.height(12.dp))

            StatusLine(
                enabled = enabled,
                pausedForPtt = pausedForPtt,
                status = status,
                modelState = modelState,
                errorMessage = errorMessage,
                wifiOnly = wifiOnly,
                sourceLabel = sourceLabel,
                targetLabel = targetLabel,
            )

            val needsOfflineSpeechPack = enabled && (
                errorMessage?.contains("Offline language", ignoreCase = true) == true ||
                    errorMessage?.contains("not installed", ignoreCase = true) == true
                )

            if (needsOfflineSpeechPack) {
                Spacer(modifier = Modifier.height(8.dp))
                TextButton(onClick = {
                    LiveTranslationBridge.openOfflineSpeechSettings(context)
                }) {
                    Text("Open speech settings", color = Cyan, fontSize = 12.sp)
                }
            }

            if (enabled && modelState == TranslationManager.ModelState.FAILED) {
                Spacer(modifier = Modifier.height(8.dp))
                TextButton(onClick = { LiveTranslationBridge.retryModelDownload() }) {
                    Text("Retry model download", color = Orange, fontSize = 12.sp)
                }
            }

            if (enabled && downloadedModels.isNotEmpty()) {
                Spacer(modifier = Modifier.height(12.dp))
                Text("Downloaded models", fontSize = 12.sp, color = TextMuted)
                Spacer(modifier = Modifier.height(4.dp))
                downloadedModels.forEach { code ->
                    val label = TranslationManager.COMMON_LANGUAGES
                        .firstOrNull { it.code == code }?.label ?: code
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = label,
                            fontSize = 13.sp,
                            color = TextWhite,
                            modifier = Modifier.weight(1f),
                        )
                        IconButton(
                            onClick = { LiveTranslationBridge.deleteLanguageModel(code) },
                            modifier = Modifier.size(32.dp),
                        ) {
                            Icon(
                                Icons.Default.Delete,
                                contentDescription = "Delete $label model",
                                tint = TextMuted,
                                modifier = Modifier.size(18.dp),
                            )
                        }
                    }
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            CaptionBlock(
                label = "Heard ($sourceLabel)",
                text = when {
                    !enabled -> "—"
                    pausedForPtt -> "Paused while transmitting"
                    caption.isNotBlank() -> caption
                    else -> "Listening…"
                },
                accent = TextGray,
            )

            Spacer(modifier = Modifier.height(8.dp))

            CaptionBlock(
                label = targetLabel,
                text = translation.ifBlank { "—" },
                accent = Orange,
            )
        }
    }
}

@Composable
private fun PrefToggle(
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    accent: androidx.compose.ui.graphics.Color,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(title, fontSize = 13.sp, color = TextWhite)
            Text(text = description, fontSize = 11.sp, color = TextMuted)
        }
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            colors = SwitchDefaults.colors(
                checkedThumbColor = accent,
                checkedTrackColor = accent.copy(alpha = 0.3f),
                uncheckedThumbColor = TextMuted,
                uncheckedTrackColor = SurfaceBg,
            ),
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LanguagePicker(
    label: String,
    selectedLabel: String,
    expanded: Boolean,
    onExpandedChange: (Boolean) -> Unit,
    onSelect: (String) -> Unit,
    excludeCode: String? = null,
    downloadedModels: List<String> = emptyList(),
) {
    val options = remember(excludeCode) {
        TranslationManager.COMMON_LANGUAGES.filter { it.code != excludeCode }
    }
    Text(label, fontSize = 12.sp, color = TextMuted)
    Spacer(modifier = Modifier.height(4.dp))
    ExposedDropdownMenuBox(
        expanded = expanded,
        onExpandedChange = onExpandedChange,
    ) {
        OutlinedTextField(
            value = selectedLabel,
            onValueChange = {},
            readOnly = true,
            trailingIcon = {
                Icon(Icons.Default.ArrowDropDown, contentDescription = null, tint = TextGray)
            },
            colors = OutlinedTextFieldDefaults.colors(
                focusedBorderColor = Cyan,
                unfocusedBorderColor = SurfaceBg,
                focusedTextColor = TextWhite,
                unfocusedTextColor = TextWhite,
                focusedContainerColor = SurfaceBg,
                unfocusedContainerColor = SurfaceBg,
            ),
            shape = RoundedCornerShape(12.dp),
            modifier = Modifier
                .fillMaxWidth()
                .menuAnchor(),
        )
        ExposedDropdownMenu(
            expanded = expanded,
            onDismissRequest = { onExpandedChange(false) },
        ) {
            options.forEach { lang ->
                val onDevice = downloadedModels.contains(lang.code)
                DropdownMenuItem(
                    text = {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(
                                text = lang.label,
                                modifier = Modifier.weight(1f),
                            )
                            if (!onDevice) {
                                Text(
                                    text = "download",
                                    fontSize = 10.sp,
                                    color = StatusWarning,
                                )
                            }
                        }
                    },
                    onClick = { onSelect(lang.code) },
                )
            }
        }
    }
}

@Composable
private fun ModelDownloadBanner(
    sourceLabel: String,
    targetLabel: String,
) {
    Surface(
        color = StatusWarning.copy(alpha = 0.12f),
        shape = RoundedCornerShape(10.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(
                color = StatusWarning,
                strokeWidth = 2.dp,
                modifier = Modifier.size(18.dp),
            )
            Spacer(modifier = Modifier.width(10.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "Downloading language model",
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = StatusWarning,
                )
                Spacer(modifier = Modifier.height(2.dp))
                Text(
                    text = "$sourceLabel → $targetLabel · ~30 MB on this device",
                    fontSize = 11.sp,
                    color = TextGray,
                )
            }
        }
    }
}

@Composable
private fun StatusLine(
    enabled: Boolean,
    pausedForPtt: Boolean,
    status: LiveCaptionTranslator.Status,
    modelState: TranslationManager.ModelState,
    errorMessage: String?,
    wifiOnly: Boolean = true,
    sourceLabel: String = "",
    targetLabel: String = "",
) {
    val downloadLine = if (sourceLabel.isNotEmpty() && targetLabel.isNotEmpty()) {
        "Downloading $sourceLabel → $targetLabel model (~30 MB)…"
    } else {
        "Downloading language model…"
    }
    val (text, color) = when {
        // Download notation wins even while the feature toggle is off —
        // picking a language still kicks off the model fetch.
        modelState == TranslationManager.ModelState.DOWNLOADING ->
            downloadLine to StatusWarning
        !enabled -> "Off" to TextMuted
        pausedForPtt -> "Paused for PTT" to StatusWarning
        status == LiveCaptionTranslator.Status.UNAVAILABLE ->
            when {
                errorMessage?.contains("Offline language", ignoreCase = true) == true ||
                    errorMessage?.contains("not installed", ignoreCase = true) == true ->
                    "Install offline speech pack in system Settings → Languages" to StatusDisconnected
                else -> (errorMessage ?: "Unavailable") to StatusDisconnected
            }
        status == LiveCaptionTranslator.Status.ERROR ->
            when {
                errorMessage?.contains("Offline language", ignoreCase = true) == true ||
                    errorMessage?.contains("not installed", ignoreCase = true) == true ->
                    "Install offline speech pack in system Settings → Languages" to StatusDisconnected
                else -> (errorMessage ?: "Error") to StatusDisconnected
            }
        modelState == TranslationManager.ModelState.FAILED ->
            if (wifiOnly) {
                "Model download failed — connect to Wi-Fi or disable Wi-Fi-only" to StatusDisconnected
            } else {
                "Model download failed — check network and retry" to StatusDisconnected
            }
        status == LiveCaptionTranslator.Status.LISTENING ->
            "Listening (offline) — captions also show on the radio screen" to StatusConnected
        status == LiveCaptionTranslator.Status.IDLE && enabled && !pausedForPtt ->
            if (sourceLabel.isNotEmpty() && targetLabel.isNotEmpty()) {
                "Quiet · $sourceLabel → $targetLabel" to TextGray
            } else {
                "Quiet — will resume listening shortly" to TextGray
            }
        modelState == TranslationManager.ModelState.READY &&
            sourceLabel.isNotEmpty() && targetLabel.isNotEmpty() ->
            "Ready · $sourceLabel → $targetLabel" to StatusConnected
        else -> "Ready" to TextGray
    }

    Row(verticalAlignment = Alignment.CenterVertically) {
        if (modelState == TranslationManager.ModelState.DOWNLOADING) {
            CircularProgressIndicator(
                color = StatusWarning,
                strokeWidth = 2.dp,
                modifier = Modifier.size(14.dp),
            )
            Spacer(modifier = Modifier.width(8.dp))
        }
        Text(text = text, fontSize = 11.sp, color = color, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun CaptionBlock(
    label: String,
    text: String,
    accent: androidx.compose.ui.graphics.Color,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(SurfaceBg)
            .padding(12.dp),
    ) {
        Text(text = label, fontSize = 10.sp, color = TextMuted, letterSpacing = 1.sp)
        Spacer(modifier = Modifier.height(4.dp))
        Text(text = text, fontSize = 15.sp, color = accent)
    }
}

/**
 * Compact caption strip for the main radio screen when live translation is on.
 */
@Composable
fun LiveTranslationOverlay(
    modifier: Modifier = Modifier,
) {
    val enabled by LiveTranslationBridge.enabled.collectAsState()
    if (!enabled) return

    val caption by LiveTranslationBridge.caption.collectAsState()
    val translation by LiveTranslationBridge.translation.collectAsState()
    val pausedForPtt by LiveTranslationBridge.pausedForPtt.collectAsState()
    val sourceLang by LiveTranslationBridge.sourceLang.collectAsState()
    val targetLang by LiveTranslationBridge.targetLang.collectAsState()
    val status by LiveTranslationBridge.status.collectAsState()
    val modelState by LiveTranslationBridge.modelState.collectAsState()

    val sourceLabel = remember(sourceLang) {
        TranslationManager.COMMON_LANGUAGES.firstOrNull { it.code == sourceLang }?.label ?: sourceLang
    }
    val targetLabel = remember(targetLang) {
        TranslationManager.COMMON_LANGUAGES.firstOrNull { it.code == targetLang }?.label ?: targetLang
    }

    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = CardBg.copy(alpha = 0.92f)),
        shape = RoundedCornerShape(12.dp),
    ) {
        Column(modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Default.Translate,
                    contentDescription = null,
                    tint = TealLight,
                    modifier = Modifier.size(16.dp),
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = "$sourceLabel → $targetLabel",
                    fontSize = 11.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = TealLight,
                    modifier = Modifier.weight(1f),
                )
                Text(
                    text = when {
                        pausedForPtt -> "PTT"
                        modelState == TranslationManager.ModelState.DOWNLOADING -> "DL"
                        status == LiveCaptionTranslator.Status.LISTENING -> "●"
                        status == LiveCaptionTranslator.Status.ERROR ||
                            status == LiveCaptionTranslator.Status.UNAVAILABLE -> "!"
                        else -> "○"
                    },
                    fontSize = 11.sp,
                    color = when {
                        pausedForPtt -> StatusWarning
                        modelState == TranslationManager.ModelState.DOWNLOADING -> StatusWarning
                        status == LiveCaptionTranslator.Status.LISTENING -> StatusConnected
                        status == LiveCaptionTranslator.Status.ERROR ||
                            status == LiveCaptionTranslator.Status.UNAVAILABLE -> StatusDisconnected
                        else -> TextMuted
                    },
                )
            }
            Spacer(modifier = Modifier.height(6.dp))
            if (modelState == TranslationManager.ModelState.DOWNLOADING) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(
                        color = StatusWarning,
                        strokeWidth = 2.dp,
                        modifier = Modifier.size(12.dp),
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(
                        text = LiveTranslationText.downloadStatusLine(sourceLang, targetLang),
                        fontSize = 13.sp,
                        fontWeight = FontWeight.Medium,
                        color = StatusWarning,
                        maxLines = 2,
                    )
                }
            } else {
                Text(
                    text = when {
                        pausedForPtt -> "Paused while transmitting"
                        status == LiveCaptionTranslator.Status.UNAVAILABLE ||
                            status == LiveCaptionTranslator.Status.ERROR ->
                            "Translation unavailable — check Settings"
                        translation.isNotBlank() -> translation
                        caption.isNotBlank() -> caption
                        status == LiveCaptionTranslator.Status.LISTENING -> "Listening…"
                        else -> "Quiet — will resume shortly"
                    },
                    fontSize = 15.sp,
                    fontWeight = FontWeight.Medium,
                    color = when {
                        pausedForPtt -> StatusWarning
                        translation.isNotBlank() -> Coral
                        status == LiveCaptionTranslator.Status.ERROR ||
                            status == LiveCaptionTranslator.Status.UNAVAILABLE -> StatusDisconnected
                        else -> TextPrimary
                    },
                    maxLines = 3,
                )
                if (translation.isNotBlank() && caption.isNotBlank() && !pausedForPtt) {
                    Spacer(modifier = Modifier.height(2.dp))
                    Text(
                        text = caption,
                        fontSize = 11.sp,
                        color = TextMuted,
                        maxLines = 2,
                    )
                }
            }
        }
    }
}
