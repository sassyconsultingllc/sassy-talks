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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*

@Composable
fun UsersScreen(onBack: () -> Unit) {
    var users by remember { mutableStateOf(SassyTalkNative.getUsers()) }

    val favorites = users.filter { it.isFavorite }
    val others = users.filter { !it.isFavorite }

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
                text = "People",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = Orange
            )

            IconButton(onClick = { users = SassyTalkNative.getUsers() }) {
                Icon(Icons.Default.Refresh, contentDescription = "Refresh", tint = Cyan)
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        if (users.isEmpty()) {
            // Empty state
            Spacer(modifier = Modifier.height(60.dp))
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.fillMaxWidth()
            ) {
                Icon(
                    Icons.Default.PeopleOutline,
                    contentDescription = null,
                    tint = TextMuted,
                    modifier = Modifier.size(64.dp)
                )
                Spacer(modifier = Modifier.height(16.dp))
                Text("No users yet", color = TextGray, fontSize = 16.sp)
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    "Users will appear here when they connect to your channel",
                    color = TextMuted,
                    fontSize = 13.sp
                )
            }
        } else {
            LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                // Favorites section
                if (favorites.isNotEmpty()) {
                    item {
                        SectionHeader(
                            title = "Favorites",
                            icon = Icons.Default.Star,
                            iconColor = OrangeLight,
                            count = favorites.size
                        )
                    }

                    items(favorites) { user ->
                        UserCard(
                            user = user,
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

                    item { Spacer(modifier = Modifier.height(8.dp)) }
                }

                // Others section
                if (others.isNotEmpty()) {
                    item {
                        SectionHeader(
                            title = "Others",
                            icon = Icons.Default.People,
                            iconColor = TextGray,
                            count = others.size
                        )
                    }

                    items(others) { user ->
                        UserCard(
                            user = user,
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
private fun SectionHeader(
    title: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    iconColor: androidx.compose.ui.graphics.Color,
    count: Int
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.padding(vertical = 8.dp)
    ) {
        Icon(icon, contentDescription = null, tint = iconColor, modifier = Modifier.size(20.dp))
        Spacer(modifier = Modifier.width(8.dp))
        Text(
            text = "$title ($count)",
            fontSize = 14.sp,
            fontWeight = FontWeight.SemiBold,
            color = TextGray,
            letterSpacing = 1.sp
        )
    }
}

@Composable
private fun UserCard(
    user: SassyTalkNative.UserInfo,
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
            // Avatar
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

            // Name
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = user.name,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Medium,
                    color = if (isMuted) TextMuted else TextWhite,
                    textDecoration = if (isMuted) TextDecoration.LineThrough else TextDecoration.None
                )
                if (isMuted) {
                    Text(
                        text = "Muted",
                        fontSize = 12.sp,
                        color = StatusDisconnected
                    )
                }
            }

            // Favorite toggle
            IconButton(onClick = onToggleFavorite) {
                Icon(
                    if (user.isFavorite) Icons.Default.Star else Icons.Default.StarOutline,
                    contentDescription = if (user.isFavorite) "Remove favorite" else "Add favorite",
                    tint = if (user.isFavorite) OrangeLight else TextMuted,
                    modifier = Modifier.size(24.dp)
                )
            }

            // Mute toggle
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
