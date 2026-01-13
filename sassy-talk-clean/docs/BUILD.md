# Sassy-Talk Build Guide

Cross-platform PTT Walkie-Talkie using Tauri 2.0

## Prerequisites

### All Platforms
- Rust 1.77+ (`rustup update stable`)
- Node.js 18+ (`node -v`)
- npm or pnpm

### Platform-Specific

#### Windows
```powershell
# Install Visual Studio Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools

# Install WebView2 (usually pre-installed on Windows 10+)
winget install Microsoft.EdgeWebView2Runtime
```

#### macOS
```bash
# Install Xcode Command Line Tools
xcode-select --install

# For iOS builds, install full Xcode from App Store
```

#### Linux (Ubuntu/Debian)
```bash
sudo apt update
sudo apt install -y \
  build-essential \
  libwebkit2gtk-4.1-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libasound2-dev
```

#### Android
```bash
# Install Android Studio
# Set ANDROID_HOME and add to PATH
export ANDROID_HOME=$HOME/Android/Sdk
export PATH=$PATH:$ANDROID_HOME/platform-tools
export PATH=$PATH:$ANDROID_HOME/cmdline-tools/latest/bin

# Install NDK
sdkmanager "ndk;26.1.10909125"

# Install Rust Android targets
rustup target add aarch64-linux-android
rustup target add armv7-linux-androideabi
rustup target add i686-linux-android
rustup target add x86_64-linux-android

# Install cargo-ndk
cargo install cargo-ndk
```

#### iOS
```bash
# Install Xcode from App Store (required for iOS)

# Install Rust iOS targets
rustup target add aarch64-apple-ios
rustup target add aarch64-apple-ios-sim

# Install ios-deploy for device installation
brew install ios-deploy
```

---

## Quick Start

```bash
# Clone and enter directory
cd sassy-talk

# Install dependencies
npm install

# Install Tauri CLI
cargo install tauri-cli

# Development mode (desktop)
cargo tauri dev
```

---

## Building for Each Platform

### Desktop (Windows/Mac/Linux)

```bash
# Debug build
cargo tauri build --debug

# Release build
cargo tauri build

# Build outputs:
# Windows: src-tauri/target/release/bundle/msi/Sassy-Talk_1.0.0_x64.msi
# macOS:   src-tauri/target/release/bundle/dmg/Sassy-Talk_1.0.0_x64.dmg
# Linux:   src-tauri/target/release/bundle/deb/sassy-talk_1.0.0_amd64.deb
```

### Android (APK)

```bash
# Initialize Android project (first time only)
cargo tauri android init

# Development build
cargo tauri android dev

# Release APK
cargo tauri android build --apk

# Release AAB (for Play Store)
cargo tauri android build --aab

# Output: src-tauri/gen/android/app/build/outputs/apk/release/app-release.apk
```

### iOS (IPA)

```bash
# Initialize iOS project (first time only)
cargo tauri ios init

# Development (simulator)
cargo tauri ios dev

# Build for device
cargo tauri ios build

# Open in Xcode for archive/distribution
open src-tauri/gen/apple/sassy-talk.xcodeproj
```

---

## Cross-Compilation Targets

| Platform | Target | Command |
|----------|--------|---------|
| Windows x64 | x86_64-pc-windows-msvc | cargo tauri build |
| macOS Intel | x86_64-apple-darwin | cargo tauri build --target x86_64-apple-darwin |
| macOS ARM | aarch64-apple-darwin | cargo tauri build --target aarch64-apple-darwin |
| Linux x64 | x86_64-unknown-linux-gnu | cargo tauri build |
| Android | aarch64-linux-android | cargo tauri android build |
| iOS | aarch64-apple-ios | cargo tauri ios build |

---

## Security Features

### Binary Protection
- LTO (Link-Time Optimization)
- Symbol stripping
- Code obfuscation via optimization

### Mobile Security (Android/iOS)
- Anti-debugging
- Root/Jailbreak detection
- Emulator detection
- Hook detection (Frida, Xposed)
- Signature verification

### Encryption
- AES-256-GCM audio encryption
- X25519 key exchange
- 60-second key rotation

---

## Troubleshooting

### "No audio devices found"
- Grant microphone permissions
- Linux: install libasound2-dev
- macOS: allow microphone in System Preferences

### Network discovery not working
- Devices must be on same WiFi
- Allow UDP ports 5354 and 41337 in firewall
- iOS: grant "Local Network" permission

---

## License

Proprietary - © 2025 Sassy Consulting LLC
