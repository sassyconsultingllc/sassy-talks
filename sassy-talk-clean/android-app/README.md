# Sassy-Talk Android App

Native Android UI for Sassy-Talk encrypted walkie-talkie.

## Structure

```
android-app/
├── app/
│   ├── src/main/
│   │   ├── java/com/sassyconsulting/sassytalkie/
│   │   │   ├── MainActivity.kt      # Main UI with PTT button
│   │   │   └── RustBridge.kt        # JNI bridge to Rust
│   │   ├── jniLibs/arm64-v8a/
│   │   │   └── libsassytalkie.so    # Rust native library
│   │   ├── res/
│   │   │   ├── layout/              # XML layouts
│   │   │   ├── drawable/            # Button/icon resources
│   │   │   └── values/              # Colors, strings, themes
│   │   └── AndroidManifest.xml
│   └── build.gradle.kts
├── build.gradle.kts
├── settings.gradle.kts
└── BUILD.bat                         # Build script
```

## Building

### Prerequisites
- Android Studio (for Gradle and SDK)
- Rust + cargo-ndk (for native library)
- Android NDK 29+

### Quick Build
```batch
BUILD.bat
```

### Manual Build

1. Build Rust library:
```powershell
cd ..\android-native
$env:ANDROID_NDK_HOME = "$env:LOCALAPPDATA\Android\Sdk\ndk\29.0.14206865"
cargo ndk -t arm64-v8a build --release
```

2. Copy library:
```powershell
copy target\aarch64-linux-android\release\libsassytalkie.so ..\android-app\app\src\main\jniLibs\arm64-v8a\
```

3. Build APK:
```powershell
cd ..\android-app
$env:JAVA_HOME = "$env:ProgramFiles\Android\Android Studio\jbr"
.\gradlew assembleRelease
```

Or open in Android Studio and build from there.

## Features

- Dark tactical theme
- Push-to-talk button with haptic feedback
- Channel selector (1-99)
- Peer discovery status
- Bottom navigation (Talk, Peers, Channels, Settings)

## Permissions

- `RECORD_AUDIO` - Voice transmission
- `INTERNET` - Network communication
- `ACCESS_WIFI_STATE` - WiFi discovery
- `CHANGE_WIFI_MULTICAST_STATE` - UDP multicast
- `VIBRATE` - PTT haptic feedback

## Architecture

The app uses a hybrid architecture:
- **Kotlin UI** - Native Android interface
- **Rust Backend** - Audio, encryption, networking (via JNI)

The `RustBridge.kt` class provides the JNI interface to call native functions from `libsassytalkie.so`.

## Adding to Play Store

Use the assets in `../playstore/` for:
- Feature graphic (1024x500)
- App icons (various sizes)
- Screenshots
- Privacy policy
- Store listing text
