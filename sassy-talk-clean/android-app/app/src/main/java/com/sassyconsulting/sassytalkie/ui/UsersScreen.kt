// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-2KGXNHA3RE3F
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import com.sassyconsulting.sassytalkie.PeerHealth
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
    var users by remember { mutableStateOf(emptyList<SassyTalkNative.UserInfo>()) }
    var isRefreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
    }

    val myName = remember { getSavedProfileName(context) }
    val myEmoji = remember { getSavedEmoji(context) }
    val myColorIdx = remember { getSavedColorIdx(context) }

    val coord = walkieService?.pttCoordinator
    val liveness = coord?.liveness
    val activePeerIds by (coord?.peerIds ?: remember {
        kotlinx.coroutines.flow.MutableStateFlow(emptySet())
    }).collectAsState()

    // One clock + roster refresh loop; peer events trigger immediate reload.
    var nowMs by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(coord) {
        val peerEventsJob = launch {
            coord?.peerEvents?.collect {
                users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
            }
        }
        var tick = 0
        try {
            while (true) {
                delay(1_000L)
                nowMs = System.currentTimeMillis()
                tick++
                if (tick % 8 == 0) {
                    users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
                }
            }
        } finally {
            peerEventsJob.cancel()
        }
    }

    // One physical peer can register under multiple ids (relay:<clientId>, its
    // audio sender_id, a BLE MAC). Collapse rows that resolve to the same
    // display name — but ONLY when the group contains a transport-alias id
    // (relay:*/MAC); two distinct devices that merely share a display name
    // (both have real sender_ids) must stay separate rows. Keep the entry
    // liveness actually tracks so presence/health stay accurate.
    fun isTransportAlias(id: String): Boolean =
        id.startsWith("relay:") || Regex("(?i)^([0-9a-f]{2}:){5}[0-9a-f]{2}$").matches(id)
    val deduped = users
        .groupBy { it.displayName() }
        .flatMap { (_, dupes) ->
            if (dupes.size > 1 && dupes.any { isTransportAlias(it.id) }) {
                listOf(
                    dupes.firstOrNull { liveness?.isTracked(it.id) == true || it.id in activePeerIds }
                        ?: dupes.first()
                )
            } else {
                dupes
            }
        }
    val favorites = deduped.filter { it.isFavorite }
    val others = deduped.filter { !it.isFavorite }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DarkBg)
            .padding(16.dp)
    ) {
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

            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onEditProfile) {
                    Icon(Icons.Default.Edit, contentDescription = "Edit Profile", tint = Cyan)
                }
                IconButton(
                    onClick = {
                        scope.launch {
                            isRefreshing = true
                            users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
                            isRefreshing = false
                        }
                    },
                    enabled = !isRefreshing,
                ) {
                    if (isRefreshing) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(22.dp),
                            strokeWidth = 2.dp,
                            color = Cyan,
                        )
                    } else {
                        Icon(Icons.Default.Refresh, contentDescription = "Refresh", tint = Cyan)
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
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
                            "Others appear when they join your channel",
                            color = TextMuted,
                            fontSize = 13.sp
                        )
                    }
                }
            } else {
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
                    items(favorites, key = { it.id }) { user ->
                        UserCard(
                            user = user,
                            status = peerStatus(
                                presence = liveness?.presence(user.id) ?: PresenceState.IDLE,
                                health = liveness?.health(user.id, nowMs),
                                lastHeardMs = liveness?.lastHeardMs(user.id) ?: 0L,
                                rttMs = liveness?.rttMs(user.id) ?: -1,
                                inActiveSet = user.id in activePeerIds,
                                isTracked = liveness?.isTracked(user.id) == true,
                                nowMs = nowMs,
                            ),
                            onToggleMute = {
                                SassyTalkNative.setUserMuted(user.id, !user.isMuted)
                                scope.launch {
                                    users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
                                }
                            },
                            onToggleFavorite = {
                                SassyTalkNative.setUserFavorite(user.id, !user.isFavorite)
                                scope.launch {
                                    users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
                                }
                            }
                        )
                    }
                }

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
                    items(others, key = { it.id }) { user ->
                        UserCard(
                            user = user,
                            status = peerStatus(
                                presence = liveness?.presence(user.id) ?: PresenceState.IDLE,
                                health = liveness?.health(user.id, nowMs),
                                lastHeardMs = liveness?.lastHeardMs(user.id) ?: 0L,
                                rttMs = liveness?.rttMs(user.id) ?: -1,
                                inActiveSet = user.id in activePeerIds,
                                isTracked = liveness?.isTracked(user.id) == true,
                                nowMs = nowMs,
                            ),
                            onToggleMute = {
                                SassyTalkNative.setUserMuted(user.id, !user.isMuted)
                                scope.launch {
                                    users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
                                }
                            },
                            onToggleFavorite = {
                                SassyTalkNative.setUserFavorite(user.id, !user.isFavorite)
                                scope.launch {
                                    users = withContext(Dispatchers.IO) { SassyTalkNative.getUsers() }
                                }
                            }
                        )
                    }
                }
            }
        }
    }
}

/** Single-line peer status derived from liveness heartbeats + registry membership. */
internal data class PeerStatus(val label: String, val color: Color)

/**
 * Never label a registry peer "offline" just because heartbeats haven't
 * arrived yet. "Out of contact" only when tracked AND liveness says STALE.
 */
internal fun peerStatus(
    presence: PresenceState,
    health: PeerHealth?,
    lastHeardMs: Long,
    rttMs: Int,
    inActiveSet: Boolean,
    isTracked: Boolean,
    nowMs: Long,
): PeerStatus {
    when (presence) {
        PresenceState.SPEAKING -> return PeerStatus("Speaking", Color(0xFF2979FF))
        PresenceState.LISTENING -> return PeerStatus("Listening", Green)
        PresenceState.MUTED -> return PeerStatus("Muted", Orange)
        PresenceState.AWAY -> return PeerStatus("Away", TextMuted)
        PresenceState.BACKGROUNDED -> return PeerStatus("Background", TextMuted)
        PresenceState.DND -> return PeerStatus("Do not disturb", Purple)
        PresenceState.IDLE -> { /* fall through */ }
    }

    if (health == PeerHealth.STALE && isTracked) {
        return PeerStatus("Out of contact", StatusDisconnected)
    }
    if (health == PeerHealth.DEGRADED) {
        val rtt = if (rttMs in 1..999) " · ${rttMs} ms" else ""
        return PeerStatus("Weak signal$rtt", Orange)
    }

    if (lastHeardMs > 0L) {
        val ageMs = nowMs - lastHeardMs
        if (ageMs < 8_000L) {
            val rtt = if (rttMs in 1..999) " · ${rttMs} ms" else ""
            return PeerStatus(
                if (ageMs < 5_000L) "In channel$rtt" else "Active ${ageMs / 1000}s ago$rtt",
                Green,
            )
        }
    }

    if (inActiveSet || health == PeerHealth.HEALTHY) {
        val rtt = if (rttMs in 1..999) " · ${rttMs} ms" else ""
        return PeerStatus("In channel$rtt", Green)
    }

    // Heard on the wire (registry) but heartbeats not in yet — still on channel.
    return PeerStatus("On channel", TextGray)
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

/**
 * A human label for a roster entry. Relay/BLE peers can be registered under a
 * raw transport id (relay:<hex>, a MAC, or a 16-hex sender id) or a blank/"null"
 * name; never surface those — show a friendly placeholder instead.
 */
private fun SassyTalkNative.UserInfo.displayName(): String =
    name.takeUnless {
        it.isBlank() ||
            it == "null" ||
            it.startsWith("relay:") ||
            Regex("(?i)^([0-9a-f]{2}:){5}[0-9a-f]{2}$").matches(it) ||
            Regex("(?i)^[0-9a-f]{16}$").matches(it)
    } ?: "Nearby device"

@Composable
private fun UserCard(
    user: SassyTalkNative.UserInfo,
    status: PeerStatus,
    onToggleMute: () -> Unit,
    onToggleFavorite: () -> Unit
) {
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
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .background(if (isMuted) TextMuted else Cyan.copy(alpha = 0.2f)),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = user.displayName().take(2).uppercase(),
                    color = if (isMuted) DarkBg else Cyan,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Bold
                )
            }

            Spacer(modifier = Modifier.width(12.dp))

            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = user.displayName(),
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Medium,
                    color = if (isMuted) TextMuted else TextWhite,
                    textDecoration = if (isMuted) TextDecoration.LineThrough else TextDecoration.None
                )
                if (isMuted) {
                    Text(text = "Muted", fontSize = 12.sp, color = StatusDisconnected)
                } else {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Box(
                            modifier = Modifier
                                .size(7.dp)
                                .clip(CircleShape)
                                .background(status.color)
                        )
                        Spacer(modifier = Modifier.width(4.dp))
                        Text(
                            text = status.label,
                            fontSize = 12.sp,
                            color = status.color,
                        )
                    }
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
