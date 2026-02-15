package com.sassyconsulting.sassytalkie.ui

import androidx.compose.runtime.*
import com.sassyconsulting.sassytalkie.SassyTalkNative

enum class Screen {
    Auth,
    DevicePicker,
    Main,
    Users,
}

@Composable
fun AppNavigation() {
    var currentScreen by remember { mutableStateOf(Screen.Auth) }

    when (currentScreen) {
        Screen.Auth -> QRAuthScreen(
            onAuthenticated = { currentScreen = Screen.DevicePicker }
        )
        Screen.DevicePicker -> DevicePickerScreen(
            onConnected = { currentScreen = Screen.Main },
            onBack = { currentScreen = Screen.Auth }
        )
        Screen.Main -> MainScreen(
            onDisconnect = { currentScreen = Screen.DevicePicker },
            onShowUsers = { currentScreen = Screen.Users }
        )
        Screen.Users -> UsersScreen(
            onBack = { currentScreen = Screen.Main }
        )
    }
}
