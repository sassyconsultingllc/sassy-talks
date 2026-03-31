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
import com.sassyconsulting.sassytalkie.ui.theme.*
import java.text.SimpleDateFormat
import java.util.*

/** A single activity entry — who spoke, when, how long */
data class ActivityEntry(
    val senderId: String,
    val senderName: String,
    val durationText: String,
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

@Composable
fun ActivityFeedScreen(
    entries: List<ActivityEntry>,
    onBack: () -> Unit,
    modifier: Modifier = Modifier
) {
    val listState = rememberLazyListState()
    val favorites = entries.filter { it.isFavorite && !it.isMuted }
    val others = entries.filter { !it.isFavorite && !it.isMuted }
    val timeFormatter = remember { SimpleDateFormat("HH:mm:ss", Locale.getDefault()) }

    Column(
        modifier = modifier
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
                text = "Activity",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )

            Spacer(modifier = Modifier.width(48.dp))
        }

        Spacer(modifier = Modifier.height(12.dp))

        if (entries.isEmpty()) {
            Spacer(modifier = Modifier.height(60.dp))
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.fillMaxWidth()
            ) {
                Icon(
                    Icons.Default.History,
                    contentDescription = null,
                    tint = TextMuted,
                    modifier = Modifier.size(64.dp)
                )
                Spacer(modifier = Modifier.height(16.dp))
                Text("No activity yet", color = TextGray, fontSize = 16.sp)
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    "Activity will appear here when others speak",
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
                        ActivityBubble(entry, timeFormatter)
                    }

                    item { Spacer(modifier = Modifier.height(4.dp)) }
                }

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
                        ActivityBubble(entry, timeFormatter)
                    }
                }
            }
        }
    }
}

@Composable
private fun ActivityBubble(
    entry: ActivityEntry,
    timeFormatter: SimpleDateFormat
) {
    val color = userColor(entry.senderId)

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically
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
            Text(
                text = entry.senderName,
                fontSize = 13.sp,
                fontWeight = FontWeight.SemiBold,
                color = color
            )

            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = timeFormatter.format(Date(entry.timestamp)),
                    fontSize = 11.sp,
                    color = TextMuted
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = entry.durationText,
                    fontSize = 11.sp,
                    color = TextGray
                )
            }
        }

        // Replay button
        if (entry.utteranceId >= 0) {
            IconButton(
                onClick = { SassyTalkNative.replayById(entry.utteranceId) },
                modifier = Modifier.size(32.dp)
            ) {
                Icon(
                    Icons.Default.PlayArrow,
                    contentDescription = "Replay",
                    tint = Cyan,
                    modifier = Modifier.size(20.dp)
                )
            }
        }
    }
}
