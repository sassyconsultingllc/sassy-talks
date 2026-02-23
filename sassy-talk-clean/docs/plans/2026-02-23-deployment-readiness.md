# SassyTalkie Deployment Readiness - Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all deployment blockers so SassyTalkie can be successfully submitted to Google Play Store and Apple App Store.

**Architecture:** Cross-platform PTT walkie-talkie with shared Rust core compiled as native libraries for each platform. Android uses Kotlin/Compose UI + JNI bridge to `libsassytalkie.so`. iOS uses SwiftUI + C FFI bridge to `libsassytalkie_ios.a`. Desktop uses Tauri (React + Rust).

**Tech Stack:** Rust 1.93+, Kotlin 1.9.20, Jetpack Compose, SwiftUI, Gradle 8.4, cargo-ndk, Xcode 15+, GitHub Actions CI/CD

---

## Task 1: Remove Hardcoded Keystore Password (CRITICAL SECURITY)

**Files:**
- Modify: `android-native/Cargo.toml:102-104`

**Step 1: Remove the signing metadata block**

The file currently contains:
```toml
[package.metadata.android.signing.release]
path = "sassy-talk.keystore"
keystore_password = "sassytalk123"
```

Remove ALL three lines (102-104). This metadata is only used by `cargo-apk` which is NOT the build system in use (Gradle handles signing). The password "sassytalk123" is exposed in source control.

**Step 2: Verify no other hardcoded secrets exist**

Run: `grep -rn "password\|secret\|keystore_password\|sassytalk123" android-native/ ios-native/ --include="*.toml" --include="*.rs" --include="*.kt" --include="*.swift" --include="*.gradle*" --include="*.properties"`
Expected: No matches for passwords/secrets (only references to env vars or secret-manager lookups)

**Step 3: Commit**

```bash
git add android-native/Cargo.toml
git commit -m "security: remove hardcoded keystore password from Cargo.toml"
```

---

## Task 2: Fix Android Release Signing Configuration

**Files:**
- Modify: `android-app/app/build.gradle.kts:22-31`

**Step 1: Add release signingConfig block**

The current `android {}` block has no `signingConfigs` section and the release buildType uses debug signing. Replace the `buildTypes` block (lines 22-31) with:

```kotlin
    signingConfigs {
        create("release") {
            // CI reads from environment; local dev falls back to debug signing
            val ksFile = file("keystore/release.keystore")
            if (ksFile.exists()) {
                storeFile = ksFile
                storePassword = System.getenv("RELEASE_STORE_PASSWORD") ?: ""
                keyAlias = System.getenv("RELEASE_KEY_ALIAS") ?: ""
                keyPassword = System.getenv("RELEASE_KEY_PASSWORD") ?: ""
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            // Use release keystore if available, otherwise debug for local dev
            val releaseKs = file("keystore/release.keystore")
            signingConfig = if (releaseKs.exists()) {
                signingConfigs.getByName("release")
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }
```

**Step 2: Add x86_64 to abiFilters**

Change line 18 from:
```kotlin
            abiFilters += listOf("arm64-v8a")
```
to:
```kotlin
            abiFilters += listOf("arm64-v8a", "x86_64")
```

This matches the existing jniLibs (both arm64-v8a and x86_64 .so files are present) and enables emulator testing.

**Step 3: Add keystore directory to .gitignore**

Ensure `android-app/app/keystore/` is gitignored so release keystores are never committed. Check if `.gitignore` exists at `android-app/.gitignore` or `android-app/app/.gitignore`. If not, create `android-app/app/.gitignore`:

```
# Release keystore - NEVER commit
keystore/
```

**Step 4: Verify Gradle sync**

Run (from `android-app/`): `./gradlew tasks --no-daemon`
Expected: Task list output without errors. Look for `bundleRelease` task.

**Step 5: Commit**

```bash
git add android-app/app/build.gradle.kts android-app/app/.gitignore
git commit -m "fix(android): add release signing config, add x86_64 to abiFilters"
```

---

## Task 3: Fix Android CI/CD Workflow

**Files:**
- Modify: `.github/workflows/android-release.yml`

**Step 1: Fix `if` condition syntax**

Lines 23 and 87 use `if: secrets.KEYSTORE_BASE64 != ''` which is incorrect GitHub Actions syntax. Secrets can't be accessed in `if` conditions directly. Change to:

```yaml
      - name: Decode and validate keystore (strict)
        if: ${{ secrets.KEYSTORE_BASE64 != '' }}
```

and:

```yaml
      - name: Secure cleanup of keystore
        if: always()
```

(The cleanup should run `always()` to ensure no keystore remains even if the build fails.)

**Step 2: Update upload-artifact action**

Line 82 uses `actions/upload-artifact@v3` which is deprecated. Update to `@v4`:

```yaml
      - name: Upload AAB artifact
        uses: actions/upload-artifact@v4
```

**Step 3: Commit**

```bash
git add .github/workflows/android-release.yml
git commit -m "fix(ci): correct secrets condition syntax, update upload-artifact to v4"
```

---

## Task 4: Clean Up local.properties for CI Compatibility

**Files:**
- Modify: `android-app/local.properties`
- Verify: `android-app/.gitignore`

**Step 1: Verify local.properties is gitignored**

`local.properties` contains machine-specific SDK paths and should NEVER be committed. Check if it's already in `.gitignore`:

Run: `grep "local.properties" android-app/.gitignore 2>/dev/null || echo "NOT FOUND"`

If not found, add to `android-app/.gitignore`:
```
local.properties
```

**Step 2: Remove from git tracking if tracked**

Run: `git ls-files android-app/local.properties`
If output shows the file is tracked:
```bash
git rm --cached android-app/local.properties
git rm --cached android-app/app/local.properties 2>/dev/null || true
```

**Step 3: Commit**

```bash
git add android-app/.gitignore
git commit -m "chore(android): gitignore local.properties"
```

---

## Task 5: Create iOS CI/CD Workflow

**Files:**
- Create: `.github/workflows/ios-release.yml`

**Step 1: Write the iOS release workflow**

```yaml
name: iOS Release Build

on:
  workflow_dispatch:
  push:
    tags:
      - 'v*'

jobs:
  build:
    runs-on: macos-14  # Apple Silicon runner (M1)
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-ios,x86_64-apple-ios,aarch64-apple-ios-sim

      - name: Cache Rust
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            sassy-talks/sassy-talk-clean/ios-native/target
          key: ${{ runner.os }}-cargo-ios-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-ios-

      - name: Build Rust libraries
        working-directory: sassy-talks/sassy-talk-clean/ios-native
        run: |
          chmod +x build.sh
          ./build.sh

      - name: Generate C headers
        working-directory: sassy-talks/sassy-talk-clean/ios-native
        run: |
          cargo install cbindgen 2>/dev/null || true
          cbindgen --config cbindgen.toml --crate sassytalkie-ios --output SassyTalkie-Generated.h

      - name: Create Xcode project
        working-directory: sassy-talks/sassy-talk-clean/ios-native
        run: |
          # Create xcodeproj using xcodegen if available, or use xcodebuild
          # For now, archive requires manual Xcode project setup
          echo "Rust libraries built successfully"
          echo "Device: target/aarch64-apple-ios/release/libsassytalkie_ios.a"
          echo "Simulator: target/universal-sim/release/libsassytalkie_ios.a"
          ls -la target/aarch64-apple-ios/release/libsassytalkie_ios.a
          ls -la target/universal-sim/release/libsassytalkie_ios.a

      - name: Upload iOS artifacts
        uses: actions/upload-artifact@v4
        with:
          name: ios-rust-libs
          path: |
            sassy-talks/sassy-talk-clean/ios-native/target/aarch64-apple-ios/release/libsassytalkie_ios.a
            sassy-talks/sassy-talk-clean/ios-native/target/universal-sim/release/libsassytalkie_ios.a
            sassy-talks/sassy-talk-clean/ios-native/SassyTalkie-Generated.h
```

**Step 2: Commit**

```bash
git add .github/workflows/ios-release.yml
git commit -m "feat(ci): add iOS release build workflow"
```

---

## Task 6: Create Xcode Project Configuration (xcodegen)

**Files:**
- Create: `ios-native/project.yml` (XcodeGen spec)
- Modify: `ios-native/build.sh` (add XcodeGen step)

**Step 1: Create XcodeGen project spec**

XcodeGen generates `.xcodeproj` from a YAML spec, making it reproducible in CI. Create `ios-native/project.yml`:

```yaml
name: SassyTalkie
options:
  bundleIdPrefix: com.sassyconsulting
  deploymentTarget:
    iOS: "14.0"
  xcodeVersion: "15.0"

settings:
  base:
    SWIFT_VERSION: "5.9"
    TARGETED_DEVICE_FAMILY: "1,2"
    INFOPLIST_FILE: Info.plist
    SWIFT_OBJC_BRIDGING_HEADER: SassyTalkie-Bridging-Header.h
    LIBRARY_SEARCH_PATHS:
      - "$(PROJECT_DIR)/target/aarch64-apple-ios/release"
      - "$(PROJECT_DIR)/target/universal-sim/release"
    OTHER_LDFLAGS:
      - "-lsassytalkie_ios"
      - "-framework Security"
      - "-framework AVFoundation"
      - "-framework CoreBluetooth"
      - "-framework Network"

targets:
  SassyTalkie:
    type: application
    platform: iOS
    sources:
      - path: "."
        includes:
          - "*.swift"
      - path: "SassyTalkie-Bridging-Header.h"
        buildPhase: none
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: com.sassyconsulting.sassytalkie
        MARKETING_VERSION: "1.0.0"
        CURRENT_PROJECT_VERSION: "1"
        CODE_SIGN_STYLE: Automatic
    info:
      path: Info.plist
```

**Step 2: Commit**

```bash
git add ios-native/project.yml
git commit -m "feat(ios): add XcodeGen project spec for reproducible builds"
```

---

## Task 7: Fix iOS Info.plist Device Capability

**Files:**
- Modify: `ios-native/Info.plist:34-37`

**Step 1: Fix device capability**

Line 36 requires `armv7` which excludes modern arm64-only devices and doesn't match the Rust build targets (aarch64 = arm64). Change:

```xml
	<key>UIRequiredDeviceCapabilities</key>
	<array>
		<string>armv7</string>
		<string>microphone</string>
	</array>
```

to:

```xml
	<key>UIRequiredDeviceCapabilities</key>
	<array>
		<string>arm64</string>
		<string>microphone</string>
	</array>
```

**Step 2: Commit**

```bash
git add ios-native/Info.plist
git commit -m "fix(ios): require arm64 instead of armv7 in device capabilities"
```

---

## Task 8: Add Android Feature Flag for eframe (Optional Binary Size Optimization)

**Files:**
- Modify: `android-native/Cargo.toml:19-21`

**Step 1: Gate eframe/egui behind a feature flag**

The eframe/egui dependencies add ~2MB to the .so for a development-only standalone UI that the Kotlin app never uses. Make them optional:

Change lines 19-21 from:
```toml
# UI
eframe = { version = "0.27", default-features = false, features = ["android-native-activity", "glow"] }
egui = "0.27"
```

to:
```toml
# UI (standalone dev mode only - not used by Kotlin app)
eframe = { version = "0.27", default-features = false, features = ["android-native-activity", "glow"], optional = true }
egui = { version = "0.27", optional = true }
android-activity = { version = "0.5", features = ["native-activity"], optional = true }
```

And change line 16 from:
```toml
android-activity = { version = "0.5", features = ["native-activity"] }
```
to removing it (since it's now under the optional group above).

Add a features section after dependencies:
```toml
[features]
default = []
standalone-ui = ["eframe", "egui", "android-activity"]
```

**Step 2: Gate android_main with cfg**

In `android-native/src/lib.rs`, the `android_main` function and `SassyTalkApp` struct should be gated:

Add `#[cfg(feature = "standalone-ui")]` before the `SassyTalkApp` struct definition and `android_main` function.

**Step 3: Rebuild .so**

Run the build script without the standalone-ui feature (default):
```bash
cd android-native
cargo ndk -t arm64-v8a build --release
```
Expected: Smaller binary size, all JNI exports still present.

**Step 4: Verify JNI exports**

```bash
nm -D target/aarch64-linux-android/release/libsassytalkie.so | grep Java_
```
Expected: All 58 JNI functions still listed.

**Step 5: Copy updated .so to jniLibs**

```bash
cp target/aarch64-linux-android/release/libsassytalkie.so ../android-app/app/src/main/jniLibs/arm64-v8a/
```

**Step 6: Commit**

```bash
git add android-native/Cargo.toml android-native/src/lib.rs
git add android-app/app/src/main/jniLibs/arm64-v8a/libsassytalkie.so
git commit -m "perf(android): gate eframe/egui behind feature flag, reduce binary ~2MB"
```

---

## Task 9: Commit All Uncommitted Work

**Files:**
- All 23 modified + 9 untracked files currently in working tree

**Step 1: Review all changes**

```bash
cd sassy-talks
git status
git diff --stat
```

Review the list carefully. Ensure no secrets, temp files, or build artifacts are included.

**Step 2: Pull remote changes first**

```bash
git pull --rebase origin main
```

Expected: Fast-forward or clean rebase (branch is 2 commits behind).

**Step 3: Stage and commit by category**

Commit new transport files:
```bash
git add android-native/src/cellular_transport.rs android-native/src/wifi_direct.rs android-native/src/audio_pipeline.rs
git add android-app/app/src/main/java/com/sassyconsulting/sassytalkie/CellularWebSocketClient.kt
git add android-app/app/src/main/java/com/sassyconsulting/sassytalkie/WalkieService.kt
git commit -m "feat: add cellular relay, WiFi Direct, and ADPCM audio pipeline"
```

Commit x86_64 build:
```bash
git add android-app/app/src/main/jniLibs/x86_64/
git commit -m "feat(android): add x86_64 native library for emulator support"
```

Commit remaining modified files:
```bash
git add -u  # stages all modified tracked files
git commit -m "refactor: update state machine, transport layer, and UI for new transports"
```

Commit Play Store listing:
```bash
git add android-app/playstore-complete/
git commit -m "docs: add Play Store listing assets"
```

**Step 4: Push**

```bash
git push origin main
```

---

## Task 10: Verification - Android Build

**Step 1: Verify Gradle build succeeds**

```bash
cd android-app
./gradlew assembleDebug --no-daemon
```

Expected: BUILD SUCCESSFUL. APK at `app/build/outputs/apk/debug/app-debug.apk`

**Step 2: Verify native library loads**

```bash
unzip -l app/build/outputs/apk/debug/app-debug.apk | grep libsassytalkie
```

Expected: Both `lib/arm64-v8a/libsassytalkie.so` and `lib/x86_64/libsassytalkie.so` listed.

**Step 3: Verify ProGuard release build**

```bash
./gradlew assembleRelease --no-daemon
```

Expected: BUILD SUCCESSFUL (will use debug signing if no release keystore present - this is expected for local dev).

---

## Task 11: Verification - iOS Rust Build (requires macOS)

**Step 1: Build Rust libraries**

```bash
cd ios-native
chmod +x build.sh
./build.sh
```

Expected: Libraries at:
- `target/aarch64-apple-ios/release/libsassytalkie_ios.a`
- `target/universal-sim/release/libsassytalkie_ios.a`

**Step 2: Generate headers**

```bash
chmod +x generate_headers.sh
./generate_headers.sh
```

Expected: `SassyTalkie-Generated.h` created with all FFI function declarations.

**Step 3: Generate Xcode project (requires xcodegen)**

```bash
brew install xcodegen  # if not installed
xcodegen generate
```

Expected: `SassyTalkie.xcodeproj` created.

**Step 4: Build in Xcode**

```bash
xcodebuild -project SassyTalkie.xcodeproj -scheme SassyTalkie -sdk iphonesimulator -configuration Debug build
```

Expected: BUILD SUCCEEDED.

---

## Summary of Execution Order

| Priority | Task | Platform | Risk |
|----------|------|----------|------|
| P0 | Task 1: Remove hardcoded password | Android | Security exposure |
| P0 | Task 2: Release signing config | Android | Play Store rejection |
| P0 | Task 3: Fix CI workflow syntax | Android | CI broken |
| P1 | Task 4: Gitignore local.properties | Android | CI breaks on non-Windows |
| P1 | Task 7: Fix armv7 → arm64 | iOS | Device exclusion |
| P1 | Task 9: Commit all work | Both | Data loss risk |
| P2 | Task 5: iOS CI workflow | iOS | No automated builds |
| P2 | Task 6: XcodeGen project spec | iOS | Manual Xcode setup |
| P2 | Task 8: eframe feature flag | Android | Binary bloat |
| P3 | Task 10: Android build verification | Android | Validation |
| P3 | Task 11: iOS build verification | iOS | Validation |

**Total tasks:** 11
**Estimated time:** Tasks 1-9 can be done now (code changes). Tasks 10-11 require build environments (Android SDK / macOS with Xcode).
