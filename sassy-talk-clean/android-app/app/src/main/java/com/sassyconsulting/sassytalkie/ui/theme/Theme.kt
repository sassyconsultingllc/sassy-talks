package com.sassyconsulting.sassytalkie.ui.theme

import android.app.Activity
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

private val DarkColorScheme = darkColorScheme(
    primary = Orange,
    onPrimary = TextWhite,
    primaryContainer = OrangeDark,
    onPrimaryContainer = TextWhite,
    
    secondary = Cyan,
    onSecondary = DarkBg,
    secondaryContainer = CyanDark,
    onSecondaryContainer = TextWhite,
    
    tertiary = Purple,
    onTertiary = TextWhite,
    
    background = DarkBg,
    onBackground = TextWhite,
    
    surface = SurfaceBg,
    onSurface = TextWhite,
    surfaceVariant = CardBg,
    onSurfaceVariant = TextGray,
    
    error = StatusDisconnected,
    onError = TextWhite,
)

@Composable
fun SassyTalkTheme(
    content: @Composable () -> Unit
) {
    val colorScheme = DarkColorScheme
    val view = LocalView.current
    
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            window.statusBarColor = DarkerBg.toArgb()
            window.navigationBarColor = DarkerBg.toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = false
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        content = content
    )
}
