package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Translate
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.translate.LiveCaptionTranslator
import com.sassyconsulting.sassytalkie.translate.TranslationLangDefaults
import com.sassyconsulting.sassytalkie.translate.TranslationManager
import com.sassyconsulting.sassytalkie.ui.theme.*

/**
 * Compose panel for offline live-caption + translation of the LOCAL speaker.
 *
 * Shows: an on/off toggle, a target-language dropdown, the live recognized
 * caption (source language), its translation (target language), and the
 * model-download / recognizer state. Matches the Material3 card style used by
 * SettingsScreen.
 *
 * Ownership: this composable owns nothing long-lived itself — it takes a
 * [TranslationManager] (so models/cache survive recomposition and can be
 * shared) and lazily builds a [LiveCaptionTranslator] remembered against it.
 * Recognition is started/stopped from the toggle; the effect cleans up on
 * disposal so leaving the screen always releases the mic.
 *
 * The CALLER is responsible for not enabling this while actively
 * PTT-transmitting (mic contention — see LiveCaptionTranslator docs).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TranslationPanel(
    translationManager: TranslationManager,
    modifier: Modifier = Modifier,
    sourceLang: String = TranslationLangDefaults.DEFAULT_SOURCE,
    initialTargetLang: String = TranslationLangDefaults.DEFAULT_TARGET,
    requireWifiForModels: Boolean = true,
) {
    val context = LocalContext.current

    // One translator per panel instance, bound to the shared manager. Gate its
    // restarts on PTT so on-device captioning never fights the walkie pipeline
    // for the mic (both open their own AudioRecord session).
    val translator = remember(translationManager) {
        LiveCaptionTranslator(context.applicationContext, translationManager).apply {
            onMicBusy = { com.sassyconsulting.sassytalkie.SassyTalkNative.isPttActive() }
        }
    }

    var enabled by rememberSaveable { mutableStateOf(false) }
    var targetLang by rememberSaveable { mutableStateOf(initialTargetLang) }
    var menuExpanded by remember { mutableStateOf(false) }

    val caption by translator.caption.collectAsState()
    val translation by translator.translation.collectAsState()
    val status by translator.status.collectAsState()
    val errorMessage by translator.errorMessage.collectAsState()
    val modelState by translationManager.modelState.collectAsState()

    // Push current config into the translator whenever it changes.
    LaunchedEffect(sourceLang, targetLang, requireWifiForModels) {
        translator.setRequireWifi(requireWifiForModels)
        translator.setLanguages(sourceLang, targetLang)
    }

    // Drive start/stop from the toggle; always release on disposal.
    DisposableEffect(enabled) {
        if (enabled) translator.start() else translator.stop()
        onDispose { translator.stop() }
    }
    DisposableEffect(Unit) {
        onDispose { translator.release() }
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
            // Header row: title + on/off
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
                    onCheckedChange = { enabled = it },
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
                text = "Captions your speech on-device and translates it. Runs offline; pause PTT while captioning to avoid mic conflicts.",
                fontSize = 11.sp,
                color = TextMuted,
            )

            Spacer(modifier = Modifier.height(12.dp))

            // Target-language picker
            Text("Translate to", fontSize = 12.sp, color = TextMuted)
            Spacer(modifier = Modifier.height(4.dp))
            ExposedDropdownMenuBox(
                expanded = menuExpanded,
                onExpandedChange = { menuExpanded = it },
            ) {
                OutlinedTextField(
                    value = targetLabel,
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
                    expanded = menuExpanded,
                    onDismissRequest = { menuExpanded = false },
                ) {
                    TranslationManager.COMMON_LANGUAGES.forEach { lang ->
                        DropdownMenuItem(
                            text = { Text(lang.label) },
                            onClick = {
                                targetLang = lang.code
                                menuExpanded = false
                            },
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Status / model line
            StatusLine(
                enabled = enabled,
                status = status,
                modelState = modelState,
                errorMessage = errorMessage,
            )

            Spacer(modifier = Modifier.height(12.dp))

            // Caption (source)
            CaptionBlock(
                label = "Heard",
                text = caption.ifBlank { if (enabled) "Listening…" else "—" },
                accent = TextGray,
            )

            Spacer(modifier = Modifier.height(8.dp))

            // Translation (target)
            CaptionBlock(
                label = targetLabel,
                text = translation.ifBlank { "—" },
                accent = Orange,
            )
        }
    }
}

@Composable
private fun StatusLine(
    enabled: Boolean,
    status: LiveCaptionTranslator.Status,
    modelState: TranslationManager.ModelState,
    errorMessage: String?,
) {
    val (text, color) = when {
        !enabled -> "Off" to TextMuted
        status == LiveCaptionTranslator.Status.UNAVAILABLE ->
            (errorMessage ?: "Unavailable") to StatusDisconnected
        status == LiveCaptionTranslator.Status.ERROR ->
            (errorMessage ?: "Error") to StatusDisconnected
        modelState == TranslationManager.ModelState.DOWNLOADING ->
            "Downloading language model…" to StatusWarning
        modelState == TranslationManager.ModelState.FAILED ->
            "Model download failed" to StatusDisconnected
        status == LiveCaptionTranslator.Status.LISTENING ->
            "Listening (offline)" to StatusConnected
        else -> "Ready" to TextGray
    }

    Row(verticalAlignment = Alignment.CenterVertically) {
        if (enabled && modelState == TranslationManager.ModelState.DOWNLOADING) {
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
