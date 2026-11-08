// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.ui.theme.Ui

/** The four top-level destinations reachable from the bottom tab bar. */
enum class Tab(val label: String, val icon: ImageVector, val screen: Screen) {
    Radio("Radio", Icons.Default.SettingsInputAntenna, Screen.Main),
    Activity("Activity", Icons.Default.Subject, Screen.Activity),
    People("People", Icons.Default.People, Screen.Users),
    Settings("Settings", Icons.Default.Settings, Screen.Settings),
}

/**
 * Frame for a tab screen: the screen's content above a persistent bottom tab
 * bar. Replaces the radio screen's overflow menu, which hid Activity, People
 * and Settings one tap deeper than they need to be.
 */
@Composable
fun TabFrame(selected: Tab, onSelect: (Tab) -> Unit, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize().background(Ui.Bg)) {
        Box(Modifier.weight(1f).fillMaxWidth()) { content() }
        Divider(color = Ui.Surface2, thickness = 1.dp)
        NavigationBar(containerColor = Ui.Bar, tonalElevation = 0.dp) {
            Tab.entries.forEach { tab ->
                NavigationBarItem(
                    selected = tab == selected,
                    onClick = { if (tab != selected) onSelect(tab) },
                    icon = { Icon(tab.icon, contentDescription = null) },
                    label = { Text(tab.label, fontSize = 12.sp) },
                    colors = NavigationBarItemDefaults.colors(
                        selectedIconColor = Ui.Accent,
                        selectedTextColor = Ui.Accent,
                        indicatorColor = Ui.AccentSoft,
                        unselectedIconColor = Ui.TextDim,
                        unselectedTextColor = Ui.TextDim,
                    ),
                )
            }
        }
    }
}

/** Screen title row used by every tab screen: one size, one colour. */
@Composable
fun ScreenHeader(
    title: String,
    modifier: Modifier = Modifier,
    onBack: (() -> Unit)? = null,
    trailing: @Composable RowScope.() -> Unit = {},
) {
    Row(
        modifier = modifier.fillMaxWidth().padding(start = if (onBack != null) 4.dp else 20.dp, end = 20.dp, top = 16.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (onBack != null) {
            IconButton(onClick = onBack) {
                Icon(Icons.Default.ArrowBack, contentDescription = "Back", tint = Ui.TextMid)
            }
        }
        Text(
            title,
            modifier = Modifier.weight(1f),
            fontSize = 24.sp,
            fontWeight = FontWeight.SemiBold,
            color = Ui.Text,
            maxLines = 1,
        )
        trailing()
    }
}

/** Small uppercase label above a group of rows. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text.uppercase(),
        modifier = modifier.padding(top = 12.dp, bottom = 6.dp),
        fontSize = 12.sp,
        letterSpacing = 1.2.sp,
        color = Ui.TextDim,
    )
}

/** Rounded surface card with the standard border. */
@Composable
fun UiCard(
    modifier: Modifier = Modifier,
    container: androidx.compose.ui.graphics.Color = Ui.Surface,
    border: androidx.compose.ui.graphics.Color = Ui.Line,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(18.dp))
            .background(container)
            .border(1.dp, border, RoundedCornerShape(18.dp)),
        content = content,
    )
}
