// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-LKBX5BETEAK6
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import com.sassyconsulting.sassytalkie.PresenceState
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.WalkieService
import com.sassyconsulting.sassytalkie.ui.theme.*

@Composable
fun UsersScreen(
    onBack: () -> Unit,
    onEditProfile: () -> Unit = {},
    walkieService: WalkieService? = null
) {
    val context = LocalContext.current
    var users by remember { mutableStateOf(SassyTalkNative.getUsers()) }
    var isRefreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // Local "You" profile from SharedPreferences
    val myName = remember { getSavedProfileName(context) }
    val myEmoji = remember { getSavedEmoji(context) }
    val myColorIdx = remember { getSavedColorIdx(context) }

    // Auto-refresh every 3 seconds so new users appear without manual tap
    LaunchedEffect(Unit) {
        while (true) {
            delay(3000)
            users = SassyTalkNative.getUsers()
        }
    }

    val favorites = users.filter { it.isFavorite }
    val others = users.filter { !it.isFavorite }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DarkBg)
            .safeDrawingPadding()
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
                text = "People",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )

            // Edit profile + refresh row
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onEditProfile) {
                    Icon(Icons.Default.Edit, contentDescription = "Edit Profile", tint = Cyan)
                }
                Box(
                    modifier = Modifier.size(48.dp),
                    contentAlignment = Alignment.Center
                ) {
                    RainbowRefreshIndicator(
                        isRefreshing = isRefreshing,
                        onRefresh = {
                            scope.launch {
                                isRefreshing = true
                                delay(600)
                                users = SassyTalkNative.getUsers()
                                isRefreshing = false
                            }
                        },
                        size = 28.dp,
                        strokeWidth = 3.dp
                    )
                }
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            // ── You (local user) ──
            item {
                SectionHeader(
                    title = "You",
                    icon = Icons.Default.Person,
                    iconColor = Cyan,
                    count = null
                )
            }
            item {
                YouCard(
                    name = myName,
                    emoji = myEmoji,
                    colorIdx = myColorIdx,
                    onEditProfile = onEditProfile
                )
            }

            if (users.isEmpty()) {
                item {
                    Spacer(modifier = Modifier.height(16.dp))
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Icon(
                            Icons.Default.PeopleOutline,
                            contentDescription = null,
                            tint = TextMuted,
                            modifier = Modifier.size(48.dp)
                        )
                        Spacer(modifier = Modifier.height(12.dp))
                        Text("No one else yet", color = TextGray, fontSize = 15.sp)
                        Spacer(modifier = Modifier.height(6.dp))
                        Text(
                            "Others will appear here when they transmit on your channel",
                            color = TextMuted,
                            fontSize = 13.sp
                        )
                    }
                }
            } else {
                // Favorites section
                if (favorites.isNotEmpty()) {
                    item {
                        Spacer(modifier = Modifier.height(8.dp))
                        SectionHeader(
                            title = "Favorites",
                            icon = Icons.Default.Star,
                            iconColor = OrangeLight,
                            count = favorites.size
                        )
                    }
                    items(favorites) { user ->
                        val liveness = walkieService?.pttCoordinator?.liveness
                        UserCard(
                            user = user,
                            presenceState = liveness?.presence(user.id) ?: PresenceState.IDLE,
                            rttMs = liveness?.rttMs(user.id) ?: -1,
                            lastHeardMs = liveness?.lastHeardMs(user.id) ?: 0L,
                            onToggleMute = {
                                SassyTalkNative.setUserMuted(user.id, !user.isMuted)
                                users = SassyTalkNative.getUsers()
                            },
                            onToggleFavorite = {
                                SassyTalkNative.setUserFavorite(user.id, !user.isFavorite)
                                users = SassyTalkNative.getUsers()
                            }
                        )
                    }
                }

                // Others section
                if (others.isNotEmpty()) {
                    item {
                        Spacer(modifier = Modifier.height(8.dp))
                        SectionHeader(
                            title = "Others",
                            icon = Icons.Default.People,
                            iconColor = TextGray,
                            count = others.size
                        )
                    }
                    items(others) { user ->
                        val liveness = walkieService?.pttCoordinator?.liveness
                        UserCard(
                            user = user,
                            presenceState = liveness?.presence(user.id) ?: PresenceState.IDLE,
                            rttMs = liveness?.rttMs(user.id) ?: -1,
                            lastHeardMs = liveness?.lastHeardMs(user.id) ?: 0L,
                            onToggleMute = {
                                SassyTalkNative.setUserMuted(user.id, !user.isMuted)
                                users = SassyTalkNative.getUsers()
                            },
                            onToggleFavorite = {
                                SassyTalkNative.setUserFavorite(user.id, !user.isFavorite)
                                users = SassyTalkNative.getUsers()
                            }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun YouCard(
    name: String,
    emoji: String,
    colorIdx: Int,
    onEditProfile: () -> Unit
) {
    Card(
        colors = CardDefaults.cardColors(containerColor = CardBg),
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth()
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            // Emoji avatar
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .background(avatarColor(colorIdx)),
                contentAlignment = Alignment.Center
            ) {
                Text(text = emoji, fontSize = 22.sp)
            }

            Spacer(modifier = Modifier.width(12.dp))

            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = name,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Medium,
                    color = Cyan
                )
                Text(
                    text = "This device",
                    fontSize = 12.sp,
                    color = TextMuted
                )
            }

            IconButton(onClick = onEditProfile) {
                Icon(
                    Icons.Default.Edit,
                    contentDescription = "Edit profile",
                    tint = TextMuted,
                    modifier = Modifier.size(20.dp)
                )
            }
        }
    }
}

@Composable
private fun SectionHeader(
    title: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    iconColor: Color,
    count: Int?
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.padding(vertical = 4.dp)
    ) {
        Icon(icon, contentDescription = null, tint = iconColor, modifier = Modifier.size(18.dp))
        Spacer(modifier = Modifier.width(6.dp))
        Text(
            text = if (count != null) "$title ($count)" else title,
            fontSize = 13.sp,
            fontWeight = FontWeight.SemiBold,
            color = TextGray,
            letterSpacing = 1.sp
        )
    }
}

/** Format a last-heard timestamp as a human-readable "active N ago" string. */
private fun formatLastActive(lastHeardMs: Long, nowMs: Long): String {
    if (lastHeardMs <= 0L) return "offline"
    val ageMs = nowMs - lastHeardMs
    return when {
        ageMs < 5_000L    -> "active now"
        ageMs < 60_000L   -> "active ${ageMs / 1000}s ago"
        ageMs < 3_600_000L -> "active ${ageMs / 60_000}m ago"
        else               -> "offline"
    }
}

@Composable
private fun UserCard(
    user: SassyTalkNative.UserInfo,
    presenceState: PresenceState = PresenceState.IDLE,
    rttMs: Int = -1,
    lastHeardMs: Long = 0L,
    onToggleMute: () -> Unit,
    onToggleFavorite: () -> Unit
) {
    // Tick every second so the "active Ns ago" label stays fresh
    var nowMs by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1_000L)
            nowMs = System.currentTimeMillis()
        }
    }
    val isMuted = user.isMuted

    Card(
        colors = CardDefaults.cardColors(containerColor = CardBg),
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier
            .fillMaxWidth()
            .alpha(if (isMuted) 0.5f else 1f)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            // Initials avatar
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .background(if (isMuted) TextMuted else Cyan.copy(alpha = 0.2f)),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = user.name.take(2).uppercase(),
                    color = if (isMuted) DarkBg else Cyan,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Bold
                )
            }

            Spacer(modifier = Modifier.width(12.dp))

            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = user.name,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Medium,
                    color = if (isMuted) TextMuted else TextWhite,
                    textDecoration = if (isMuted) TextDecoration.LineThrough else TextDecoration.None
                )
                if (isMuted) {
                    Text(text = "Muted", fontSize = 12.sp, color = StatusDisconnected)
                } else {
                    Spacer(modifier = Modifier.height(2.dp))
                    PresenceChip(state = presenceState, rttMs = rttMs)
                    val lastActiveText = formatLastActive(lastHeardMs, nowMs)
                    Text(
                        text = lastActiveText,
                        fontSize = 11.sp,
                        color = if (lastActiveText == "active now") Color(0xFF4CAF50) else TextMuted
                    )
                }
            }

            IconButton(onClick = onToggleFavorite) {
                Icon(
                    if (user.isFavorite) Icons.Default.Star else Icons.Default.StarOutline,
                    contentDescription = if (user.isFavorite) "Remove favorite" else "Add favorite",
                    tint = if (user.isFavorite) OrangeLight else TextMuted,
                    modifier = Modifier.size(24.dp)
                )
            }

            IconButton(onClick = onToggleMute) {
                Icon(
                    if (isMuted) Icons.Default.VolumeOff else Icons.Default.VolumeUp,
                    contentDescription = if (isMuted) "Unmute" else "Mute",
                    tint = if (isMuted) StatusDisconnected else Green,
                    modifier = Modifier.size(24.dp)
                )
            }
        }
    }
}

/**
 * Small presence indicator chip showing the peer's current PresenceState and RTT.
 *
 * Visual spec:
 *   LISTENING   -> green dot  + "Listening"
 *   SPEAKING    -> blue dot   + "Speaking"
 *   MUTED       -> orange dot + "Muted"
 *   AWAY        -> gray dot   + "Away"
 *   BACKGROUNDED -> gray dot  + "Background"
 *   DND         -> purple dot + "DND"
 *   IDLE        -> gray dot   + "Idle"
 *
 * If rttMs is in 1..999 an RTT label is appended (e.g. "42 ms").
 */
@Composable
fun PresenceChip(state: PresenceState, rttMs: Int) {
    val (dotColor, label) = when (state) {
        PresenceState.LISTENING    -> Green  to "Listening"
        PresenceState.SPEAKING     -> Color(0xFF2979FF) to "Speaking"
        PresenceState.MUTED        -> Orange to "Muted"
        PresenceState.AWAY         -> TextMuted to "Away"
        PresenceState.BACKGROUNDED -> TextMuted to "Background"
        PresenceState.DND          -> Purple to "DND"
        PresenceState.IDLE         -> TextMuted to "Idle"
    }

    val rttLabel = if (rttMs in 1..999) "  ${rttMs} ms" else ""

    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            modifier = Modifier
                .size(7.dp)
                .clip(CircleShape)
                .background(dotColor)
        )
        Spacer(modifier = Modifier.width(4.dp))
        Text(
            text = "$label$rttLabel",
            fontSize = 11.sp,
            color = dotColor,
            fontWeight = FontWeight.Normal
        )
    }
}
