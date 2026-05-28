package com.sassyconsulting.sassytalkie.ui

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*

private const val PREFS_SETTINGS = "sassy_settings"
private const val KEY_LOCK_SCREEN_PTT = "lock_screen_ptt"
private const val KEY_MIC_GAIN = "mic_gain"
private const val KEY_SQUELCH_DBFS = "squelch_dbfs"
private const val KEY_ENABLE_WIFI = "enable_wifi_multicast"
private const val KEY_ENABLE_RELAY = "enable_cloudflare_relay"
// When true, 2..=6 overlapping speakers are PCM-mixed in real time on the
// receiver instead of being serialized into the legacy Queue. Lets a small
// group sound like a real conversation (with step-on) instead of a walkie-
// talkie. Off by default — preserves the classic single-speaker-at-a-time
// behavior unless the user opts in.
private const val KEY_ENABLE_MIX_MODE = "enable_mix_mode"
private const val DEFAULT_MIC_GAIN = 1.0f
private const val DEFAULT_SQUELCH_DBFS = 0 // 0 = disabled

@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onTransportPrefsChanged: () -> Unit = {},
) {
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(PREFS_SETTINGS, Context.MODE_PRIVATE) }
    val scrollState = rememberScrollState()

    var lockScreenPtt by remember { mutableStateOf(prefs.getBoolean(KEY_LOCK_SCREEN_PTT, false)) }
    var micGain by remember { mutableFloatStateOf(prefs.getFloat(KEY_MIC_GAIN, DEFAULT_MIC_GAIN)) }
    var squelchDbfs by remember { mutableIntStateOf(prefs.getInt(KEY_SQUELCH_DBFS, DEFAULT_SQUELCH_DBFS)) }
    val squelchOn = squelchDbfs != 0
    var wifiEnabled by remember { mutableStateOf(prefs.getBoolean(KEY_ENABLE_WIFI, true)) }
    var relayEnabled by remember { mutableStateOf(prefs.getBoolean(KEY_ENABLE_RELAY, true)) }
    // Sync mix-mode state from native on first composition so the toggle
    // reflects the actual cache state even after a config change / process
    // restart picked up a stale prefs value.
    var mixModeEnabled by remember {
        mutableStateOf(prefs.getBoolean(KEY_ENABLE_MIX_MODE, false))
    }
    LaunchedEffect(Unit) {
        // Apply the persisted preference to native on screen entry — the
        // native side resets to default (off) whenever the audio cache is
        // recreated, so we re-assert here.
        SassyTalkNative.setMixModeEnabled(mixModeEnabled)
    }

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
                text = "Settings",
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
                .verticalScroll(scrollState)
        ) {
            // PTT Settings
            SettingsCard(title = "Push-to-Talk") {
                SettingsToggle(
                    icon = Icons.Default.LockOpen,
                    title = "Lock Screen PTT",
                    description = "Allow push-to-talk from the lock screen notification",
                    checked = lockScreenPtt,
                    onCheckedChange = { enabled ->
                        lockScreenPtt = enabled
                        prefs.edit().putBoolean(KEY_LOCK_SCREEN_PTT, enabled).apply()
                    }
                )
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Mic Settings — gain + squelch. Defaults to gain 1.0× and squelch
            // off so the user experience is unchanged unless they adjust.
            SettingsCard(title = "Microphone") {
                Text(
                    text = "Raise the gain if your voice isn't picked up; enable squelch if background noise gets transmitted.",
                    fontSize = 11.sp,
                    color = TextMuted
                )
                Spacer(modifier = Modifier.height(10.dp))

                // Gain slider
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Default.Mic, contentDescription = null, tint = TextMuted, modifier = Modifier.size(20.dp))
                    Spacer(modifier = Modifier.width(10.dp))
                    Column(modifier = Modifier.weight(1f)) {
                        Text("Mic Gain", fontSize = 14.sp, color = TextGray, fontWeight = FontWeight.Medium)
                        Text(
                            String.format("%.2fx (%+.1f dB)", micGain, 20.0 * kotlin.math.log10(micGain.toDouble())),
                            fontSize = 11.sp,
                            color = TextMuted
                        )
                    }
                }
                Slider(
                    value = micGain,
                    onValueChange = { micGain = it },
                    onValueChangeFinished = {
                        prefs.edit().putFloat(KEY_MIC_GAIN, micGain).apply()
                        SassyTalkNative.setMicGain(micGain)
                    },
                    valueRange = 0.25f..4.0f,
                    steps = 14, // 0.25, 0.5, 0.75, ..., 4.0
                    colors = SliderDefaults.colors(
                        thumbColor = Orange,
                        activeTrackColor = Orange,
                        inactiveTrackColor = SurfaceBg
                    )
                )
                TextButton(
                    onClick = {
                        micGain = DEFAULT_MIC_GAIN
                        prefs.edit().putFloat(KEY_MIC_GAIN, micGain).apply()
                        SassyTalkNative.setMicGain(micGain)
                    }
                ) {
                    Text("Reset to 1.0×", fontSize = 11.sp, color = Cyan)
                }

                Spacer(modifier = Modifier.height(8.dp))

                // Squelch toggle + threshold slider
                SettingsToggle(
                    icon = Icons.Default.GraphicEq,
                    title = "Noise Squelch",
                    description = "Drop quiet frames below the threshold to avoid transmitting background noise.",
                    checked = squelchOn,
                    onCheckedChange = { enabled ->
                        val newDbfs = if (enabled) -40 else 0
                        squelchDbfs = newDbfs
                        prefs.edit().putInt(KEY_SQUELCH_DBFS, newDbfs).apply()
                        SassyTalkNative.setSquelchDbfs(newDbfs)
                    }
                )
                if (squelchOn) {
                    Text(
                        text = "Threshold: $squelchDbfs dBFS",
                        fontSize = 11.sp,
                        color = TextMuted,
                        modifier = Modifier.padding(start = 30.dp)
                    )
                    Slider(
                        value = squelchDbfs.toFloat(),
                        onValueChange = { squelchDbfs = it.toInt() },
                        onValueChangeFinished = {
                            prefs.edit().putInt(KEY_SQUELCH_DBFS, squelchDbfs).apply()
                            SassyTalkNative.setSquelchDbfs(squelchDbfs)
                        },
                        valueRange = -60f..-10f,
                        steps = 9, // -60, -55, ..., -10
                        colors = SliderDefaults.colors(
                            thumbColor = Orange,
                            activeTrackColor = Orange,
                            inactiveTrackColor = SurfaceBg
                        )
                    )
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Group Voice — opt-in client-side mixing for 2-6 simultaneous
            // speakers. With this off, overlapping speakers are serialized
            // (classic walkie-talkie). With it on, they're PCM-mixed in real
            // time so the channel sounds like a real group conversation.
            SettingsCard(title = "Group Voice") {
                Text(
                    text = "Mix up to 6 overlapping speakers in real time instead of queueing them. Useful for active group channels; classic walkie-talkie behavior otherwise. Above 6 talkers the channel auto-falls-back to queue.",
                    fontSize = 11.sp,
                    color = TextMuted
                )
                Spacer(modifier = Modifier.height(8.dp))
                SettingsToggle(
                    icon = Icons.Default.Groups,
                    title = "Live Group Mix",
                    description = "PCM-mix overlapping audio (2-6 speakers) instead of serializing",
                    checked = mixModeEnabled,
                    onCheckedChange = { enabled ->
                        mixModeEnabled = enabled
                        prefs.edit().putBoolean(KEY_ENABLE_MIX_MODE, enabled).apply()
                        SassyTalkNative.setMixModeEnabled(enabled)
                    }
                )
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Network — manual transport overrides. Defaults to both on
            // (current auto behavior). Turning Relay off keeps you LAN-only
            // for same-WiFi peers; turning WiFi off forces relay-only.
            SettingsCard(title = "Network") {
                Text(
                    text = "Choose which transports the app uses. Turning Cloudflare Relay off keeps audio LAN-local for peers on the same WiFi — useful to avoid relay traffic when not needed.",
                    fontSize = 11.sp,
                    color = TextMuted
                )
                Spacer(modifier = Modifier.height(8.dp))
                SettingsToggle(
                    icon = Icons.Default.Wifi,
                    title = "WiFi Multicast",
                    description = "Local peers on the same WiFi network",
                    checked = wifiEnabled,
                    onCheckedChange = { enabled ->
                        wifiEnabled = enabled
                        prefs.edit().putBoolean(KEY_ENABLE_WIFI, enabled).apply()
                        onTransportPrefsChanged()
                    }
                )
                SettingsToggle(
                    icon = Icons.Default.Cloud,
                    title = "Cloudflare Relay",
                    description = "Remote peers via the cellular/internet relay",
                    checked = relayEnabled,
                    onCheckedChange = { enabled ->
                        relayEnabled = enabled
                        prefs.edit().putBoolean(KEY_ENABLE_RELAY, enabled).apply()
                        onTransportPrefsChanged()
                    }
                )
                if (!wifiEnabled && !relayEnabled) {
                    Spacer(modifier = Modifier.height(6.dp))
                    Text(
                        text = "Both disabled — audio only flows if Bluetooth peers are nearby and advertising. Bluetooth is independent of these toggles and runs in the background whenever the adapter is on.",
                        fontSize = 11.sp,
                        color = androidx.compose.ui.graphics.Color(0xFFFF9800)
                    )
                }
            }
        }
    }
}

@Composable
private fun SettingsCard(
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
private fun SettingsToggle(
    icon: ImageVector,
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
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
            modifier = Modifier.size(20.dp)
        )
        Spacer(modifier = Modifier.width(10.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, fontSize = 14.sp, color = TextGray, fontWeight = FontWeight.Medium)
            Text(text = description, fontSize = 11.sp, color = TextMuted)
        }
        Spacer(modifier = Modifier.width(8.dp))
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            colors = SwitchDefaults.colors(
                checkedThumbColor = Orange,
                checkedTrackColor = Orange.copy(alpha = 0.3f),
                uncheckedThumbColor = TextMuted,
                uncheckedTrackColor = SurfaceBg
            )
        )
    }
}
