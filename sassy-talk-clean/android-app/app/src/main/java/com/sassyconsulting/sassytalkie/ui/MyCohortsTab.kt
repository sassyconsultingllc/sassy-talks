// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-5MAX7OGMV4I2
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*
import org.json.JSONArray

data class CohortListItem(
    val cohortId: String,
    val channel: Int,
    val groupName: String,
    val role: String,
    val hostDevice: String?,
    val participants: List<String>,
    val lastJoinedAt: Long,
)

private fun parseHistory(json: String): List<CohortListItem> {
    return try {
        val arr = JSONArray(json)
        (0 until arr.length()).map { i ->
            val o = arr.getJSONObject(i)
            val parts = o.optJSONArray("last_participants") ?: JSONArray()
            val names = (0 until parts.length()).map { j ->
                parts.getJSONObject(j).optString("name", "")
            }.filter { it.isNotEmpty() }
            CohortListItem(
                cohortId = o.getString("cohort_id"),
                channel = o.getInt("channel"),
                groupName = o.optString("group_name", "Channel ${o.getInt("channel")}"),
                role = o.optString("role", "Hosted"),
                hostDevice = o.optString("host_device", "").ifEmpty { null },
                participants = names,
                lastJoinedAt = o.optLong("last_joined_at", 0),
            )
        }
    } catch (_: Exception) { emptyList() }
}

private fun relativeAgo(unixSecs: Long): String {
    if (unixSecs == 0L) return ""
    val nowSecs = System.currentTimeMillis() / 1000
    val delta = (nowSecs - unixSecs).coerceAtLeast(0)
    return when {
        delta < 60 -> "just now"
        delta < 3600 -> "${delta / 60}m ago"
        delta < 86400 -> "${delta / 3600}h ago"
        else -> "${delta / 86400}d ago"
    }
}

@Composable
fun MyCohortsTab(
    onRejoinHost: (channel: Int, groupName: String, cohortId: String) -> Unit,
    onRejoinJoiner: (channel: Int, hostDevice: String?) -> Unit,
) {
    var items by remember { mutableStateOf(parseHistory(SassyTalkNative.getCohortHistory())) }

    fun refresh() {
        items = parseHistory(SassyTalkNative.getCohortHistory())
    }

    Column(modifier = Modifier.fillMaxSize()) {
        if (items.isEmpty()) {
            Box(
                modifier = Modifier.fillMaxSize().padding(32.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    "Cohorts you host or join will show up here so you can resume them without saving keys.",
                    color = TextGray,
                    fontSize = 14.sp,
                    textAlign = TextAlign.Center,
                )
            }
        } else {
            LazyColumn(
                modifier = Modifier.fillMaxWidth().weight(1f),
                verticalArrangement = Arrangement.spacedBy(8.dp),
                contentPadding = PaddingValues(8.dp),
            ) {
                items(items, key = { it.cohortId }) { item ->
                    CohortRow(
                        item = item,
                        onRejoin = {
                            when (item.role) {
                                "Hosted", "Both" -> onRejoinHost(item.channel, item.groupName, item.cohortId)
                                else -> onRejoinJoiner(item.channel, item.hostDevice)
                            }
                        },
                        onRemove = {
                            SassyTalkNative.removeCohort(item.cohortId)
                            refresh()
                        },
                    )
                }
            }
            OutlinedButton(
                onClick = {
                    SassyTalkNative.clearCohortHistory()
                    refresh()
                },
                shape = RoundedCornerShape(25.dp),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = TextMuted),
                modifier = Modifier.fillMaxWidth().padding(8.dp).height(44.dp),
            ) {
                Text("Clear All", fontSize = 13.sp)
            }
        }
    }
}

@Composable
private fun CohortRow(
    item: CohortListItem,
    onRejoin: () -> Unit,
    onRemove: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    Card(
        colors = CardDefaults.cardColors(containerColor = CardBg),
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(item.groupName, fontSize = 16.sp, fontWeight = FontWeight.SemiBold,
                     color = TextWhite, modifier = Modifier.weight(1f))
                AssistChip(
                    onClick = {},
                    label = { Text("Ch ${item.channel}", fontSize = 11.sp) },
                    colors = AssistChipDefaults.assistChipColors(containerColor = SurfaceBg, labelColor = Orange),
                )
            }
            val subtitle = buildString {
                if (item.role == "Joined" || item.role == "Both") {
                    item.hostDevice?.let { append("Hosted by $it · ") }
                }
                append("${item.participants.size} participants")
                val ago = relativeAgo(item.lastJoinedAt)
                if (ago.isNotEmpty()) append(" · $ago")
            }
            Text(subtitle, fontSize = 12.sp, color = TextGray, modifier = Modifier.padding(top = 2.dp))

            if (item.participants.isNotEmpty()) {
                Spacer(modifier = Modifier.height(6.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    item.participants.take(3).forEach { name ->
                        AssistChip(
                            onClick = {},
                            label = { Text(name, fontSize = 11.sp) },
                            colors = AssistChipDefaults.assistChipColors(containerColor = SurfaceBg, labelColor = TextWhite),
                        )
                    }
                    if (item.participants.size > 3) {
                        AssistChip(
                            onClick = {},
                            label = { Text("+${item.participants.size - 3}", fontSize = 11.sp) },
                            colors = AssistChipDefaults.assistChipColors(containerColor = SurfaceBg, labelColor = TextMuted),
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(8.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Button(
                    onClick = onRejoin,
                    colors = ButtonDefaults.buttonColors(containerColor = Orange),
                    shape = RoundedCornerShape(25.dp),
                    modifier = Modifier.weight(1f).height(40.dp),
                ) {
                    Icon(
                        when (item.role) {
                            "Hosted" -> Icons.Default.QrCode2
                            "Joined" -> Icons.Default.QrCodeScanner
                            else -> Icons.Default.Login
                        },
                        contentDescription = null,
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        when (item.role) {
                            "Hosted" -> "Rejoin"
                            "Joined" -> "Rejoin · Scan"
                            else -> "Rejoin"
                        },
                        fontSize = 13.sp,
                    )
                }
                Box {
                    IconButton(onClick = { menuOpen = true }) {
                        Icon(Icons.Default.MoreVert, contentDescription = "More", tint = TextGray)
                    }
                    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                        DropdownMenuItem(
                            text = { Text("Remove") },
                            onClick = { menuOpen = false; onRemove() },
                        )
                    }
                }
            }
        }
    }
}
