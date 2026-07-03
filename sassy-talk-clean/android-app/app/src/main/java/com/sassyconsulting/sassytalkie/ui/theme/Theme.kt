package com.sassyconsulting.sassytalkie.ui.theme

import android.app.Activity
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

// Material3 scheme mapped onto the Tauri tokens: blue primary, teal secondary,
// purple tertiary, slate surfaces. See ui/theme/Color.kt.
private val DarkColorScheme = darkColorScheme(
    primary = PrimaryBlue,
    onPrimary = TextPrimary,
    primaryContainer = PrimaryBlueDark,
    onPrimaryContainer = TextPrimary,

    secondary = Teal,
    onSecondary = BgDark,
    secondaryContainer = TealDark,
    onSecondaryContainer = TextPrimary,

    tertiary = BrandPurple,
    onTertiary = TextPrimary,

    background = BgDark,
    onBackground = TextPrimary,

    surface = BgMedium,
    onSurface = TextPrimary,
    surfaceVariant = BgLight,
    onSurfaceVariant = TextSecondary,

    outline = BorderColor,

    error = StatusErrorToken,
    onError = TextPrimary,
)

@Composable
fun SassyTalkTheme(
    content: @Composable () -> Unit
) {
    val colorScheme = DarkColorScheme
    val view = LocalView.current
    
    if (!view.isInEditMode) {
        SideEffect {
            // Edge-to-edge: bar colors come from the app content drawing behind
            // transparent system bars (enableEdgeToEdge() in MainActivity).
            // window.statusBarColor/navigationBarColor are deprecated in API 35
            // and deliberately not touched here.
            val window = (view.context as Activity).window
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = false
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        content = content
    )
}
