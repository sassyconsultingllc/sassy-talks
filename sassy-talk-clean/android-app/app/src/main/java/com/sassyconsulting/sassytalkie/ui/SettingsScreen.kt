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
import androidx.compose.ui.graphics.Color
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val PREFS_SETTINGS = "sassy_settings"
private const val KEY_LOCK_SCREEN_PTT = "lock_screen_ptt"
private const val KEY_MIC_GAIN = "mic_gain"
private const val KEY_SQUELCH_DBFS = "squelch_dbfs"
private const val KEY_ENABLE_WIFI = "enable_wifi_multicast"
private const val KEY_ENABLE_RELAY = "enable_cloudflare_relay"
// v2.7.5 — RX playback + speakerphone + jitter buffer
private const val KEY_RX_GAIN = "rx_gain"
private const val KEY_SPEAKERPHONE = "speakerphone_on"
private const val KEY_JITTER_PREBUFFER = "jitter_prebuffer_frames"
private const val DEFAULT_RX_GAIN = 1.0f
private const val DEFAULT_SPEAKERPHONE = true   // walkie-talkie default = loud speaker
private const val DEFAULT_JITTER_PREBUFFER = 5  // matches core's DEFAULT_LIVE_JITTER_PREBUFFER_FRAMES
// When true, 2..=6 overlapping speakers are PCM-mixed in real time on the
// receiver instead of being serialized into the legacy Queue. Lets a small
// group sound like a real conversation (with step-on) instead of a walkie-
// talkie. Off by default — preserves the classic single-speaker-at-a-time
// behavior unless the user opts in.
private const val KEY_ENABLE_MIX_MODE = "enable_mix_mode"
private const val DEFAULT_MIC_GAIN = 1.0f
private const val DEFAULT_SQUELCH_DBFS = 0 // 0 = disabled
// AI noise suppression on the mic TX path (spectral-subtraction Wiener filter).
// Default off — zero cost when disabled.
private const val KEY_NOISE_SUPPRESSION = "noise_suppression"
// Sealed sender — blind the relay's room/peer/device metadata behind per-epoch
// HKDF handles. Default off; coordinated (all peers must enable + share the key).
private const val KEY_SEALED_SENDER = "sealed_sender"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onTransportPrefsChanged: () -> Unit = {},
    walkieService: com.sassyconsulting.sassytalkie.WalkieService? = null,
) {
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(PREFS_SETTINGS, Context.MODE_PRIVATE) }
    val scrollState = rememberScrollState()
    // v2.7.0 — used by the Diagnostics card's Test Relay button
    val scope = rememberCoroutineScope()

    var lockScreenPtt by remember { mutableStateOf(prefs.getBoolean(KEY_LOCK_SCREEN_PTT, false)) }
    var micGain by remember { mutableFloatStateOf(prefs.getFloat(KEY_MIC_GAIN, DEFAULT_MIC_GAIN)) }
    var squelchDbfs by remember { mutableIntStateOf(prefs.getInt(KEY_SQUELCH_DBFS, DEFAULT_SQUELCH_DBFS)) }
    val squelchOn = squelchDbfs != 0
    var wifiEnabled by remember { mutableStateOf(prefs.getBoolean(KEY_ENABLE_WIFI, true)) }
    var relayEnabled by remember { mutableStateOf(prefs.getBoolean(KEY_ENABLE_RELAY, true)) }
    // v2.7.5 playback state
    var rxGain by remember { mutableFloatStateOf(prefs.getFloat(KEY_RX_GAIN, DEFAULT_RX_GAIN)) }
    var speakerphoneOn by remember { mutableStateOf(prefs.getBoolean(KEY_SPEAKERPHONE, DEFAULT_SPEAKERPHONE)) }
    var jitterFrames by remember { mutableIntStateOf(prefs.getInt(KEY_JITTER_PREBUFFER, DEFAULT_JITTER_PREBUFFER)) }
    // Sync mix-mode state from native on first composition so the toggle
    // reflects the actual cache state even after a config change / process
    // restart picked up a stale prefs value.
    var mixModeEnabled by remember {
        mutableStateOf(prefs.getBoolean(KEY_ENABLE_MIX_MODE, false))
    }
    var noiseSuppression by remember { mutableStateOf(prefs.getBoolean(KEY_NOISE_SUPPRESSION, false)) }
    var sealedSender by remember { mutableStateOf(prefs.getBoolean(KEY_SEALED_SENDER, false)) }
    // Shared on-device translator (ML Kit + offline SpeechRecognizer). Released
    // when the screen leaves composition so model handles don't leak.
    val translationManager = remember { com.sassyconsulting.sassytalkie.translate.TranslationManager() }
    DisposableEffect(Unit) { onDispose { translationManager.release() } }
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
                color = BrandPurpleLight
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
                        thumbColor = Teal,
                        activeTrackColor = Teal,
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
                            thumbColor = Teal,
                            activeTrackColor = Teal,
                            inactiveTrackColor = SurfaceBg
                        )
                    )
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            // v2.7.5: Playback — RX-side controls (incoming audio).
            // Three knobs that together address "audio is choppy and quiet":
            //   1. Volume — pre-AudioTrack gain multiplier (independent of
            //      system media volume); fixes "quiet".
            //   2. Speakerphone — loudspeaker vs earpiece routing; fixes
            //      "I can only hear it pressed to my ear".
            //   3. Buffer — Live-mode jitter pre-buffer size preset; trades
            //      latency for smoothness; fixes "choppy".
            SettingsCard(title = "Playback") {
                Text(
                    text = "Tune the incoming audio: boost quiet peers, pick the speaker, and absorb cellular jitter at the cost of a small delay.",
                    fontSize = 11.sp,
                    color = TextMuted,
                )
                Spacer(modifier = Modifier.height(10.dp))

                // RX Volume / gain slider
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Default.VolumeUp, contentDescription = null, tint = TextMuted, modifier = Modifier.size(20.dp))
                    Spacer(modifier = Modifier.width(10.dp))
                    Column(modifier = Modifier.weight(1f)) {
                        Text("RX Volume", fontSize = 14.sp, color = TextGray, fontWeight = FontWeight.Medium)
                        Text(
                            String.format("%.2fx (%+.1f dB)", rxGain, 20.0 * kotlin.math.log10(rxGain.toDouble())),
                            fontSize = 11.sp,
                            color = TextMuted,
                        )
                    }
                }
                Slider(
                    value = rxGain,
                    onValueChange = { rxGain = it },
                    onValueChangeFinished = {
                        prefs.edit().putFloat(KEY_RX_GAIN, rxGain).apply()
                        SassyTalkNative.setRxGain(rxGain)
                    },
                    valueRange = 0.25f..4.0f,
                    steps = 14,  // 0.25, 0.5, …, 4.0
                    colors = SliderDefaults.colors(
                        thumbColor = Teal,
                        activeTrackColor = Teal,
                        inactiveTrackColor = SurfaceBg,
                    ),
                )
                TextButton(
                    onClick = {
                        rxGain = DEFAULT_RX_GAIN
                        prefs.edit().putFloat(KEY_RX_GAIN, rxGain).apply()
                        SassyTalkNative.setRxGain(rxGain)
                    }
                ) {
                    Text("Reset to 1.0×", fontSize = 11.sp, color = Cyan)
                }

                Spacer(modifier = Modifier.height(8.dp))

                // Speakerphone vs earpiece
                SettingsToggle(
                    icon = if (speakerphoneOn) Icons.Default.VolumeUp else Icons.Default.PhoneInTalk,
                    title = if (speakerphoneOn) "Loud Speaker" else "Earpiece",
                    description = if (speakerphoneOn) {
                        "Route received voice through the main speaker (walkie-talkie default)."
                    } else {
                        "Route received voice through the earpiece (private listening — phone-call style)."
                    },
                    checked = speakerphoneOn,
                    onCheckedChange = { on ->
                        speakerphoneOn = on
                        prefs.edit().putBoolean(KEY_SPEAKERPHONE, on).apply()
                        SassyTalkNative.setSpeakerphone(on)
                    },
                )

                Spacer(modifier = Modifier.height(8.dp))

                // Jitter buffer preset
                Text("Buffer", fontSize = 14.sp, color = TextGray, fontWeight = FontWeight.Medium)
                Text(
                    "Larger buffer = smoother audio on jittery networks, but more delay between PTT-press and audio.",
                    fontSize = 11.sp,
                    color = TextMuted,
                )
                Spacer(modifier = Modifier.height(6.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), modifier = Modifier.fillMaxWidth()) {
                    listOf(
                        Triple("Low-Latency", 3, "60 ms"),
                        Triple("Balanced",    5, "100 ms"),
                        Triple("Smooth",      8, "160 ms"),
                    ).forEach { (label, frames, latency) ->
                        val selected = jitterFrames == frames
                        FilterChip(
                            selected = selected,
                            onClick = {
                                jitterFrames = frames
                                prefs.edit().putInt(KEY_JITTER_PREBUFFER, frames).apply()
                                SassyTalkNative.setJitterPrebufferFrames(frames)
                            },
                            label = {
                                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                                    Text(label, fontSize = 11.sp)
                                    Text(latency, fontSize = 9.sp, color = TextMuted)
                                }
                            },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = Cyan.copy(alpha = 0.2f),
                                selectedLabelColor = Cyan,
                            ),
                            modifier = Modifier.weight(1f),
                        )
                    }
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

            // AI Noise Suppression — spectral-subtraction / Wiener denoise on
            // the mic TX path. Default off (zero cost). For trucks/wind/sites.
            SettingsCard(title = "AI Noise Suppression") {
                Text(
                    text = "On-device denoise of your microphone (wind, engine, HVAC, crowd) before it's transmitted. Runs entirely on the phone.",
                    fontSize = 11.sp,
                    color = TextMuted
                )
                Spacer(modifier = Modifier.height(8.dp))
                SettingsToggle(
                    icon = Icons.Default.GraphicEq,
                    title = "Noise Suppression",
                    description = "Clean up background noise on your outgoing voice",
                    checked = noiseSuppression,
                    onCheckedChange = { enabled ->
                        noiseSuppression = enabled
                        prefs.edit().putBoolean(KEY_NOISE_SUPPRESSION, enabled).apply()
                        SassyTalkNative.setNoiseSuppressionEnabled(enabled)
                    }
                )
            }

            Spacer(modifier = Modifier.height(12.dp))

            // On-device translation — ML Kit + offline SpeechRecognizer. Captions
            // and translates your speech locally; nothing leaves the device. The
            // panel carries its own enable switch + target-language picker.
            com.sassyconsulting.sassytalkie.ui.TranslationPanel(
                translationManager = translationManager,
                requireWifiForModels = wifiEnabled,
            )

            Spacer(modifier = Modifier.height(12.dp))

            // Privacy — sealed sender (metadata resistance). Blinds the relay's
            // room/peer/device fields behind rotating, key-derived HKDF handles
            // so the relay can't correlate who talks to whom or track a peer
            // over time. Coordinated: every peer must enable it (shared key).
            SettingsCard(title = "Privacy") {
                Text(
                    text = "Sealed Sender hides your room and device identity from the relay using rotating, key-derived handles — the relay only ever sees opaque tokens, never a stable room or device name. Everyone in the channel must turn this on (you all share the same key); toggling briefly reconnects.",
                    fontSize = 11.sp,
                    color = TextMuted
                )
                Spacer(modifier = Modifier.height(8.dp))
                SettingsToggle(
                    icon = Icons.Default.Lock,
                    title = "Sealed Sender",
                    description = "Blind room/peer/device metadata on the relay (metadata resistance)",
                    checked = sealedSender,
                    onCheckedChange = { enabled ->
                        sealedSender = enabled
                        prefs.edit().putBoolean(KEY_SEALED_SENDER, enabled).apply()
                        SassyTalkNative.setSealedSenderEnabled(enabled)
                        // Reconnect so the relay WS picks up the new (blinded or
                        // plaintext) room URL immediately.
                        onTransportPrefsChanged()
                    }
                )
            }

            Spacer(modifier = Modifier.height(12.dp))

            // Network — manual transport overrides. Defaults to both on
            // (current auto behavior). Turning Relay off keeps you LAN-only
            // for same-WiFi peers; turning WiFi off forces relay-only.
            SettingsCard(title = "Network") {
                Text(
                    text = "Choose which transports the app uses. The app auto-fails over WiFi → relay → Bluetooth when signal drops, and advises when a better path is available. Turning Cloudflare Relay off keeps audio LAN-local for peers on the same WiFi.",
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

            Spacer(modifier = Modifier.height(12.dp))

            // Diagnostics — live overlay + one-shot dump for support threads.
            SettingsCard(title = "Diagnostics") {
                Text(
                    text = "Use the live panel while testing audio on the main radio screen. " +
                        "Tap the panel header to expand. Copy the full dump to paste into support.",
                    fontSize = 11.sp,
                    color = TextMuted,
                )
                Spacer(modifier = Modifier.height(8.dp))
                var testing by remember { mutableStateOf(false) }
                var testResult by remember { mutableStateOf<String?>(null) }
                Button(
                    onClick = {
                        testing = true
                        testResult = null
                        scope.launch {
                            val r = withContext(Dispatchers.IO) { probeRelay() }
                            testResult = r
                            testing = false
                        }
                    },
                    enabled = !testing,
                    colors = ButtonDefaults.buttonColors(containerColor = Cyan, contentColor = DarkBg),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(Icons.Default.NetworkPing, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(if (testing) "Testing…" else "Test Relay", fontSize = 14.sp)
                }
                if (testResult != null) {
                    Spacer(modifier = Modifier.height(6.dp))
                    Text(
                        text = testResult!!,
                        fontSize = 11.sp,
                        color = if (testResult!!.startsWith("OK")) Color(0xFF4CD964) else Color(0xFFFF6B6B),
                    )
                }

                // v2.7.2: "Show diagnostic info" — one-screen dump of every
                // useful debug field, with a copy-all action. The thing you
                // paste into a support thread when something's broken.
                Spacer(modifier = Modifier.height(12.dp))
                var showDiag by remember { mutableStateOf(false) }
                OutlinedButton(
                    onClick = { showDiag = true },
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = Cyan),
                    modifier = Modifier.fillMaxWidth().height(44.dp),
                ) {
                    Icon(Icons.Default.Info, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(modifier = Modifier.width(8.dp))
                    Text("Show Diagnostic Info", fontSize = 14.sp)
                }
                if (showDiag) {
                    DiagnosticInfoDialog(
                        walkieService = walkieService,
                        onDismiss = { showDiag = false },
                    )
                }

                Spacer(modifier = Modifier.height(12.dp))
                val overlayOn by com.sassyconsulting.sassytalkie.debug.DiagnosticsPrefs
                    .overlayEnabled.collectAsState()
                SettingsToggle(
                    icon = Icons.Default.Visibility,
                    title = "Show diagnostics panel",
                    description = "Live HUD on the radio screen: transport, relay room, queues, peers",
                    checked = overlayOn,
                    onCheckedChange = {
                        com.sassyconsulting.sassytalkie.debug.DiagnosticsPrefs.setOverlayEnabled(it)
                    }
                )
            }
        }
    }
}

/**
 * v2.7.2 diagnostic-info sheet. Aggregates everything a troubleshooter would
 * need into one scrollable + copyable text block:
 *   - app build (version + versionCode + commit-ish)
 *   - device / OS (Build.MODEL, Build.MANUFACTURER, SDK)
 *   - native init status
 *   - session status (channels, active cohorts) from `getSessionStatus`
 *   - cache status from `getCacheStatus`
 *   - current network transport (ConnectivityManager snapshot)
 *     SettingsScreen doesn't bind to WalkieService directly; we read what
 *     ConnectivityManager reports right now instead, which is the same value
 *     the badge shows).
 *   - relay probe result (one-shot)
 *
 * NOT included by design: any session keys, encrypted blobs, or peer-identifying
 * tokens. The dump is safe to paste into a support thread.
 */
@Composable
private fun DiagnosticInfoDialog(
    walkieService: com.sassyconsulting.sassytalkie.WalkieService?,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var dump by remember { mutableStateOf("Collecting…") }

    LaunchedEffect(Unit) {
        dump = withContext(Dispatchers.IO) {
            com.sassyconsulting.sassytalkie.debug.DiagnosticsCollector
                .buildTextDump(context, walkieService) + "\n--- Relay probe ---\n" + probeRelay()
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            TextButton(
                onClick = {
                    val cm = context.getSystemService(Context.CLIPBOARD_SERVICE)
                            as android.content.ClipboardManager
                    cm.setPrimaryClip(android.content.ClipData.newPlainText("SassyTalkie diag", dump))
                    scope.launch { onDismiss() }
                }
            ) { Text("Copy") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
        title = { Text("Diagnostic Info") },
        text = {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 480.dp)
                    .verticalScroll(rememberScrollState())
            ) {
                Text(
                    text = dump,
                    fontSize = 11.sp,
                    color = TextGray,
                    fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                )
            }
        }
    )
}

/**
 * Probe the Cloudflare relay's /auth endpoint with a throwaway room ID.
 * ID and report HTTP status + round-trip ms. Blocking — caller must run on
 * Dispatchers.IO. Returns a single-line summary suitable for inline display.
 *
 * Uses /auth (not /health) because /auth requires AUTH_SECRET to be configured
 * on the server — a 200 or 500 here proves DNS + TLS + worker routing all
 * resolved; only a network error would prevent any response at all.
 */
private suspend fun probeRelay(): String {
    val start = System.currentTimeMillis()
    return try {
        val client = okhttp3.OkHttpClient.Builder()
            .connectTimeout(5, java.util.concurrent.TimeUnit.SECONDS)
            .readTimeout(5, java.util.concurrent.TimeUnit.SECONDS)
            .build()
        val req = okhttp3.Request.Builder()
            .url("https://relay.sassyconsultingllc.com/auth?room=test-probe-0001")
            .get()
            .build()
        client.newCall(req).execute().use { resp ->
            val ms = System.currentTimeMillis() - start
            val tag = if (resp.code in 200..299 || resp.code == 500) "OK" else "WARN"
            "$tag · HTTP ${resp.code} · ${ms} ms"
        }
    } catch (t: Throwable) {
        val ms = System.currentTimeMillis() - start
        "FAIL · ${t.javaClass.simpleName}: ${t.message ?: "unknown"} · ${ms} ms"
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
                checkedThumbColor = Teal,
                checkedTrackColor = Teal.copy(alpha = 0.3f),
                uncheckedThumbColor = TextMuted,
                uncheckedTrackColor = SurfaceBg
            )
        )
    }
}
