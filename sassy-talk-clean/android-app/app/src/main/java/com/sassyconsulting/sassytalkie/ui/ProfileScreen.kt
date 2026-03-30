package com.sassyconsulting.sassytalkie.ui

import android.content.Context
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sassyconsulting.sassytalkie.SassyTalkNative
import com.sassyconsulting.sassytalkie.ui.theme.*

private val AVATAR_EMOJIS = listOf(
    "📻", "🎙️", "🔊", "📡", "🛡️", "⚡",
    "🌐", "🔒", "🎯", "🚀", "👤", "🦊",
    "🐺", "🦁", "🐉", "🔥", "🌊", "⭐"
)

private val AVATAR_COLORS = listOf(
    Color(0xFF00BCD4), // Cyan
    Color(0xFFFF6B35), // Orange
    Color(0xFF4CAF50), // Green
    Color(0xFF9C27B0), // Purple
    Color(0xFFE91E63), // Pink
    Color(0xFFFFC107), // Amber
    Color(0xFF607D8B), // Blue-gray
    Color(0xFFFF5722), // Deep orange
)

private const val PREFS_NAME = "sassy_profile"
private const val KEY_NAME = "name"
private const val KEY_EMOJI = "emoji"
private const val KEY_COLOR_IDX = "color_idx"
const val KEY_PROFILE_SET = "profile_set"

/** Read the saved profile name, falling back to the Android model. */
fun getSavedProfileName(context: Context): String =
    context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .getString(KEY_NAME, Build.MODEL) ?: Build.MODEL

/** Read the saved avatar emoji. */
fun getSavedEmoji(context: Context): String =
    context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .getString(KEY_EMOJI, "📻") ?: "📻"

/** Read the saved avatar color index (0-based into AVATAR_COLORS). */
fun getSavedColorIdx(context: Context): Int =
    context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .getInt(KEY_COLOR_IDX, 0)

/** Get the Color object for a profile color index. */
fun avatarColor(colorIdx: Int): Color =
    AVATAR_COLORS.getOrElse(colorIdx) { AVATAR_COLORS[0] }

@OptIn(ExperimentalComposeUiApi::class)
@Composable
fun ProfileScreen(
    onDone: () -> Unit,
    showBackButton: Boolean = false,
    onBack: (() -> Unit)? = null
) {
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE) }
    val keyboard = LocalSoftwareKeyboardController.current

    var name by remember { mutableStateOf(getSavedProfileName(context)) }
    var selectedEmoji by remember { mutableStateOf(getSavedEmoji(context)) }
    var selectedColorIdx by remember { mutableIntStateOf(getSavedColorIdx(context)) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(DarkBg)
            .safeDrawingPadding()
            .padding(horizontal = 24.dp, vertical = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // Header
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically
        ) {
            if (showBackButton && onBack != null) {
                TextButton(onClick = onBack) {
                    Text("Cancel", color = TextGray)
                }
            } else {
                Spacer(modifier = Modifier.width(72.dp))
            }
            Text(
                text = "Your Profile",
                fontSize = 22.sp,
                fontWeight = FontWeight.Bold,
                color = Orange,
                modifier = Modifier.weight(1f),
                textAlign = TextAlign.Center
            )
            Spacer(modifier = Modifier.width(72.dp))
        }

        Spacer(modifier = Modifier.height(24.dp))

        // Avatar preview
        Box(
            modifier = Modifier
                .size(100.dp)
                .clip(CircleShape)
                .background(avatarColor(selectedColorIdx)),
            contentAlignment = Alignment.Center
        ) {
            Text(text = selectedEmoji, fontSize = 48.sp)
        }

        Spacer(modifier = Modifier.height(24.dp))

        // Name field
        Text("Display Name", fontSize = 13.sp, color = TextMuted, modifier = Modifier.fillMaxWidth())
        Spacer(modifier = Modifier.height(6.dp))
        OutlinedTextField(
            value = name,
            onValueChange = { if (it.length <= 24) name = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { keyboard?.hide() }),
            colors = OutlinedTextFieldDefaults.colors(
                focusedBorderColor = Orange,
                unfocusedBorderColor = CardBg,
                focusedTextColor = TextWhite,
                unfocusedTextColor = TextWhite,
                cursorColor = Orange,
                focusedContainerColor = SurfaceBg,
                unfocusedContainerColor = SurfaceBg
            ),
            shape = RoundedCornerShape(12.dp)
        )

        Spacer(modifier = Modifier.height(20.dp))

        // Color picker
        Text("Avatar Color", fontSize = 13.sp, color = TextMuted, modifier = Modifier.fillMaxWidth())
        Spacer(modifier = Modifier.height(8.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            AVATAR_COLORS.forEachIndexed { idx, color ->
                Box(
                    modifier = Modifier
                        .size(36.dp)
                        .clip(CircleShape)
                        .background(color)
                        .then(
                            if (idx == selectedColorIdx)
                                Modifier.border(3.dp, TextWhite, CircleShape)
                            else
                                Modifier
                        )
                        .clickable { selectedColorIdx = idx },
                    contentAlignment = Alignment.Center
                ) {
                    if (idx == selectedColorIdx) {
                        Icon(
                            Icons.Default.Check,
                            contentDescription = null,
                            tint = Color.White,
                            modifier = Modifier.size(18.dp)
                        )
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(20.dp))

        // Emoji picker
        Text("Avatar Emoji", fontSize = 13.sp, color = TextMuted, modifier = Modifier.fillMaxWidth())
        Spacer(modifier = Modifier.height(8.dp))
        LazyVerticalGrid(
            columns = GridCells.Fixed(6),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.fillMaxWidth()
        ) {
            items(AVATAR_EMOJIS) { emoji ->
                Box(
                    modifier = Modifier
                        .size(48.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(if (emoji == selectedEmoji) SurfaceBg else CardBg)
                        .then(
                            if (emoji == selectedEmoji)
                                Modifier.border(2.dp, avatarColor(selectedColorIdx), RoundedCornerShape(10.dp))
                            else
                                Modifier
                        )
                        .clickable { selectedEmoji = emoji },
                    contentAlignment = Alignment.Center
                ) {
                    Text(text = emoji, fontSize = 24.sp)
                }
            }
        }

        Spacer(modifier = Modifier.weight(1f))

        // Save button
        Button(
            onClick = {
                val trimmed = name.trim().ifEmpty { Build.MODEL }
                prefs.edit()
                    .putString(KEY_NAME, trimmed)
                    .putString(KEY_EMOJI, selectedEmoji)
                    .putInt(KEY_COLOR_IDX, selectedColorIdx)
                    .putBoolean(KEY_PROFILE_SET, true)
                    .apply()
                SassyTalkNative.setDeviceName(trimmed)
                onDone()
            },
            modifier = Modifier
                .fillMaxWidth()
                .height(52.dp),
            colors = ButtonDefaults.buttonColors(containerColor = Orange),
            shape = RoundedCornerShape(26.dp),
            enabled = name.isNotBlank()
        ) {
            Text("Save & Continue", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
        }

        Spacer(modifier = Modifier.height(16.dp))
    }
}
