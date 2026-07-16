@echo off
REM Sassy-Talk Android App Build Script

echo ====================================
echo  Sassy-Talk Android Build
echo ====================================
echo.

REM Check for Android Studio installation
if not exist "%ProgramFiles%\Android\Android Studio\jbr\bin\java.exe" (
    echo ERROR: Android Studio not found
    echo Please install Android Studio from https://developer.android.com/studio
    pause
    exit /b 1
)

REM Set JAVA_HOME
set JAVA_HOME=%ProgramFiles%\Android\Android Studio\jbr
echo Using Java: %JAVA_HOME%

REM Build the native Rust library first
echo.
echo [1/3] Building Rust native library...
cd ..\android-native
set ANDROID_NDK_HOME=%LOCALAPPDATA%\Android\Sdk\ndk\29.0.14206865
cargo ndk -t arm64-v8a build --release
if errorlevel 1 (
    echo ERROR: Rust build failed
    pause
    exit /b 1
)

REM Copy the library
echo.
echo [2/3] Copying native library...
copy /Y "target\aarch64-linux-android\release\libsassytalkie.so" "..\android-app\app\src\main\jniLibs\arm64-v8a\"

REM Build the Android app
echo.
echo [3/3] Building Android APK...
cd ..\android-app

REM Use gradlew if exists, otherwise use gradle from Android Studio
if exist "gradlew.bat" (
    call gradlew.bat assembleRelease
) else (
    "%ProgramFiles%\Android\Android Studio\gradle\gradle-8.2\bin\gradle.bat" assembleRelease
)

if errorlevel 1 (
    echo ERROR: Android build failed
    pause
    exit /b 1
)

echo.
echo ====================================
echo  BUILD SUCCESSFUL!
echo ====================================
echo.
echo APK location: app\build\outputs\apk\release\app-release.apk
echo.
pause
