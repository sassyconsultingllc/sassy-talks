package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.TranscriptionBridge
import com.sassyconsulting.sassytalkie.ui.theme.*
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.*

/** A single transcription entry */
data class TranscriptionEntry(
    val senderId: String,
    val senderName: String,
    val text: String,
    val timestamp: Long,
    val isFavorite: Boolean = false,
    val isMuted: Boolean = false,
    /** Unique utterance ID from Rust audio cache history. -1 if not cached. */
    val utteranceId: Long = -1L,
)

/** Color palette for user avatars — deterministic from user ID */
private val userColors = listOf(
    Color(0xFF4CD964), // Green
    Color(0xFF00E6C8), // Cyan
    Color(0xFF9664E6), // Purple
    Color(0xFFFF8C00), // Orange
    Color(0xFF5AC8FA), // Blue
    Color(0xFFFF6B6B), // Red-pink
    Color(0xFFFFCC00), // Yellow
)

private fun userColor(userId: String): Color {
    val hash = userId.hashCode().let { if (it < 0) -it else it }
    return userColors[hash % userColors.size]
}

/**
 * Snapshot of cache state for the header chip. Reflects the native
 * `AudioCache::status_json()` shape — see audio_cache.rs::status_json.
 */
private data class CacheStatusSnapshot(
    val mode: String = "Live",
    val queuedUtterances: Int = 0,
    val queuedDurationMs: Long = 0L,
    val currentSpeakerName: String? = null,
    val historyCount: Int = 0,
)

private fun TranscriptionBridge.CacheSnapshot.toUi(): CacheStatusSnapshot =
    CacheStatusSnapshot(
        mode = mode,
        queuedUtterances = queuedUtterances,
        queuedDurationMs = queuedDurationMs,
        currentSpeakerName = currentSpeakerName,
        historyCount = historyCount,
    )

@Composable
fun TranscriptionFeedScreen(
    entries: List<TranscriptionEntry>,
    onBack: () -> Unit,
    modifier: Modifier = Modifier
) {
    val listState = rememberLazyListState()
    val favorites = entries.filter { it.isFavorite && !it.isMuted }
    val others = entries.filter { !it.isFavorite && !it.isMuted }
    val timeFormatter = remember { SimpleDateFormat("HH:mm:ss", Locale.getDefault()) }

    val cacheStatus = TranscriptionBridge.cacheSnapshot.collectAsState().value.toUi()

    val snackbarHost = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()

    Scaffold(
        snackbarHost = { SnackbarHost(snackbarHost) },
        containerColor = DarkBg,
        modifier = modifier.fillMaxSize(),
    ) { padding ->
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
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
                text = "Timeline",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )

            Spacer(modifier = Modifier.width(48.dp)) // balance the back button
        }

        Spacer(modifier = Modifier.height(8.dp))

        // Cache status chip — surfaces mode + queue depth + current speaker,
        // plus a Skip button when something is mid-playback. Without this the
        // user has no way to tell that "this entry is queued waiting on the
        // current one to finish" or that the replay they just tapped is
        // actually running.
        CacheStatusBar(
            status = cacheStatus,
            onSkip = {
                SassyTalkNative.skipCurrentUtterance()
                scope.launch { snackbarHost.showSnackbar("Skipped") }
            },
            onClearCache = {
                SassyTalkNative.clearAudioCache()
                scope.launch { snackbarHost.showSnackbar("Cache cleared") }
            },
        )

        Spacer(modifier = Modifier.height(12.dp))

        if (entries.isEmpty()) {
            // Empty state
            Spacer(modifier = Modifier.height(60.dp))
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.fillMaxWidth()
            ) {
                Icon(
                    Icons.Default.SubtitlesOff,
                    contentDescription = null,
                    tint = TextMuted,
                    modifier = Modifier.size(64.dp)
                )
                Spacer(modifier = Modifier.height(16.dp))
                Text("No activity yet", color = TextGray, fontSize = 16.sp)
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    "Timeline entries will appear here when others speak",
                    color = TextMuted,
                    fontSize = 13.sp
                )
            }
        } else {
            LazyColumn(
                state = listState,
                verticalArrangement = Arrangement.spacedBy(6.dp),
                modifier = Modifier.weight(1f)
            ) {
                // Favorites section
                if (favorites.isNotEmpty()) {
                    item {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(
                                Icons.Default.Star,
                                contentDescription = null,
                                tint = OrangeLight,
                                modifier = Modifier.size(16.dp)
                            )
                            Spacer(modifier = Modifier.width(6.dp))
                            Text(
                                "Favorites",
                                fontSize = 12.sp,
                                color = TextMuted,
                                letterSpacing = 1.sp
                            )
                        }
                    }

                    items(favorites) { entry ->
                        TranscriptionBubble(entry, timeFormatter, snackbarHost, scope)
                    }

                    item { Spacer(modifier = Modifier.height(4.dp)) }
                }

                // Others section
                if (others.isNotEmpty()) {
                    if (favorites.isNotEmpty()) {
                        item {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Icon(
                                    Icons.Default.People,
                                    contentDescription = null,
                                    tint = TextMuted,
                                    modifier = Modifier.size(16.dp)
                                )
                                Spacer(modifier = Modifier.width(6.dp))
                                Text(
                                    "Others",
                                    fontSize = 12.sp,
                                    color = TextMuted,
                                    letterSpacing = 1.sp
                                )
                            }
                        }
                    }

                    items(others) { entry ->
                        TranscriptionBubble(entry, timeFormatter, snackbarHost, scope)
                    }
                }
            }
        }
    }
    } // close Scaffold content
}

/**
 * Cache status bar — fixed-height chip just under the screen header.
 * Shows current speaker / queue depth / mode, with Skip and Clear actions.
 * Hides itself entirely when the cache is idle and history is empty
 * (avoids visual noise on first launch).
 */
@Composable
private fun CacheStatusBar(
    status: CacheStatusSnapshot,
    onSkip: () -> Unit,
    onClearCache: () -> Unit,
) {
    val isIdle = status.currentSpeakerName == null &&
        status.queuedUtterances == 0 &&
        status.historyCount == 0
    if (isIdle) return

    val activeColor = when (status.mode) {
        "Queue" -> Orange
        "Mix"   -> Cyan
        "Replay"-> OrangeLight
        else    -> TextMuted   // Live mode
    }

    Surface(
        color = SurfaceBg,
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // Mode pip
            Box(
                modifier = Modifier
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(activeColor)
            )
            Spacer(modifier = Modifier.width(8.dp))

            // Status text — what's happening right now
            val label = when {
                !status.currentSpeakerName.isNullOrBlank() && status.currentSpeakerName != "null" ->
                    "Playing ${status.currentSpeakerName}" +
                        if (status.queuedUtterances > 0) " · ${status.queuedUtterances} queued" else ""
                status.queuedUtterances > 0 ->
                    "${status.queuedUtterances} queued (${status.queuedDurationMs / 1000}s)"
                else ->
                    "${status.mode} · ${status.historyCount} in history"
            }
            Text(
                text = label,
                fontSize = 12.sp,
                color = TextGray,
                modifier = Modifier.weight(1f),
            )

            // Skip — only useful when something is currently playing or
            // queued. Tapping skips the in-flight utterance and advances
            // to the next queued one.
            if (status.currentSpeakerName != null || status.queuedUtterances > 0) {
                IconButton(onClick = onSkip, modifier = Modifier.size(28.dp)) {
                    Icon(
                        Icons.Default.SkipNext,
                        contentDescription = "Skip current",
                        tint = Cyan,
                        modifier = Modifier.size(18.dp),
                    )
                }
            }

            // Clear — empties active buffers and queue, keeps history so
            // replay buttons still work after a "reset playback" gesture.
            IconButton(onClick = onClearCache, modifier = Modifier.size(28.dp)) {
                Icon(
                    Icons.Default.Clear,
                    contentDescription = "Clear queue",
                    tint = TextMuted,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
    }
}

@Composable
private fun TranscriptionBubble(
    entry: TranscriptionEntry,
    timeFormatter: SimpleDateFormat,
    snackbarHost: SnackbarHostState,
    scope: kotlinx.coroutines.CoroutineScope,
) {
    val color = userColor(entry.senderId)

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.Top
    ) {
        // Avatar
        Box(
            modifier = Modifier
                .size(32.dp)
                .clip(CircleShape)
                .background(color.copy(alpha = 0.2f)),
            contentAlignment = Alignment.Center
        ) {
            Text(
                text = entry.senderName.take(1).uppercase(),
                color = color,
                fontSize = 14.sp,
                fontWeight = FontWeight.Bold
            )
        }

        Spacer(modifier = Modifier.width(8.dp))

        Column(modifier = Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = entry.senderName,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = color
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = timeFormatter.format(Date(entry.timestamp)),
                    fontSize = 11.sp,
                    color = TextMuted
                )
            }

            Spacer(modifier = Modifier.height(2.dp))

            Text(
                text = entry.text,
                fontSize = 14.sp,
                color = TextWhite
            )
        }

        // Replay button — always rendered so the row layout is consistent;
        // disabled (grey) and shows an explanatory snackbar on tap when this
        // entry's audio isn't in the cache (utteranceId < 0). Hiding the
        // button entirely was the previous behavior, and it made the feature
        // feel broken — users tapped where they expected a button and got
        // nothing.
        val cached = entry.utteranceId >= 0
        IconButton(
            onClick = {
                if (cached) {
                    val started = SassyTalkNative.replayById(entry.utteranceId)
                    scope.launch {
                        snackbarHost.showSnackbar(
                            if (started) "Replaying ${entry.senderName}"
                            else         "Audio expired from cache"
                        )
                    }
                } else {
                    scope.launch {
                        snackbarHost.showSnackbar(
                            "No cached audio — older entries aren't replayable"
                        )
                    }
                }
            },
            modifier = Modifier.size(32.dp),
        ) {
            Icon(
                Icons.Default.PlayArrow,
                contentDescription = if (cached) "Replay" else "Replay unavailable",
                tint = if (cached) Cyan else TextMuted.copy(alpha = 0.4f),
                modifier = Modifier.size(20.dp),
            )
        }
    }
}
