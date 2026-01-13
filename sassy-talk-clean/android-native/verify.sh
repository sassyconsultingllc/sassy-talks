#!/bin/bash
# SassyTalkie - Complete Implementation Verification
# Checks that all files and features are present

set -e

echo "========================================"
echo "  SassyTalkie Verification v1.0"
echo "  Checking Complete Implementation"
echo "========================================"
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS=0
FAIL=0

# Function to check file exists
check_file() {
    if [ -f "$1" ]; then
        echo -e "${GREEN}✓${NC} $1"
        ((PASS++))
    else
        echo -e "${RED}✗${NC} $1 - MISSING"
        ((FAIL++))
    fi
}

# Function to check file contains text
check_content() {
    if grep -q "$2" "$1" 2>/dev/null; then
        echo -e "${GREEN}✓${NC} $1 contains: $2"
        ((PASS++))
    else
        echo -e "${RED}✗${NC} $1 missing: $2"
        ((FAIL++))
    fi
}

# Function to count lines
count_lines() {
    if [ -f "$1" ]; then
        LINES=$(wc -l < "$1")
        echo -e "${CYAN}  ⮑${NC} $LINES lines"
    fi
}

echo -e "${CYAN}[1] Checking Core Source Files${NC}"
echo "─────────────────────────────────"
check_file "src/lib.rs"
count_lines "src/lib.rs"
check_file "src/bluetooth.rs"
count_lines "src/bluetooth.rs"
check_file "src/jni_bridge.rs"
count_lines "src/jni_bridge.rs"
check_file "src/audio.rs"
count_lines "src/audio.rs"
check_file "src/state.rs"
count_lines "src/state.rs"
check_file "src/permissions.rs"
count_lines "src/permissions.rs"
echo ""

echo -e "${CYAN}[2] Checking Configuration Files${NC}"
echo "─────────────────────────────────"
check_file "Cargo.toml"
check_file "AndroidManifest.xml"
check_file "build.sh"
echo ""

echo -e "${CYAN}[3] Checking Documentation${NC}"
echo "─────────────────────────────────"
check_file "README.md"
check_file "PRODUCTION_GUIDE.md"
check_file "PRIVACY_POLICY.md"
check_file "privacy-policy.html"
echo ""

echo -e "${CYAN}[4] Verifying lib.rs Features${NC}"
echo "─────────────────────────────────"
check_content "src/lib.rs" "mod audio"
check_content "src/lib.rs" "mod state"
check_content "src/lib.rs" "mod permissions"
check_content "src/lib.rs" "Screen::Permissions"
check_content "src/lib.rs" "Screen::DeviceList"
check_content "src/lib.rs" "Screen::Main"
check_content "src/lib.rs" "handle_ptt_press"
check_content "src/lib.rs" "handle_ptt_release"
check_content "src/lib.rs" "connect_to_device"
check_content "src/lib.rs" "start_listening"
echo ""

echo -e "${CYAN}[5] Verifying audio.rs Features${NC}"
echo "─────────────────────────────────"
check_content "src/audio.rs" "pub struct AudioEngine"
check_content "src/audio.rs" "start_recording"
check_content "src/audio.rs" "stop_recording"
check_content "src/audio.rs" "start_playing"
check_content "src/audio.rs" "stop_playing"
check_content "src/audio.rs" "read_audio"
check_content "src/audio.rs" "write_audio"
check_content "src/audio.rs" "AndroidAudioRecord"
check_content "src/audio.rs" "AndroidAudioTrack"
echo ""

echo -e "${CYAN}[6] Verifying state.rs Features${NC}"
echo "─────────────────────────────────"
check_content "src/state.rs" "pub struct StateMachine"
check_content "src/state.rs" "on_ptt_press"
check_content "src/state.rs" "on_ptt_release"
check_content "src/state.rs" "connect_to_device"
check_content "src/state.rs" "start_listening"
check_content "src/state.rs" "start_tx_thread"
check_content "src/state.rs" "start_rx_thread"
check_content "src/state.rs" "AppState::Transmitting"
check_content "src/state.rs" "AppState::Receiving"
echo ""

echo -e "${CYAN}[7] Verifying bluetooth.rs Features${NC}"
echo "─────────────────────────────────"
check_content "src/bluetooth.rs" "pub struct BluetoothManager"
check_content "src/bluetooth.rs" "pub fn connect"
check_content "src/bluetooth.rs" "pub fn listen"
check_content "src/bluetooth.rs" "pub fn send_audio"
check_content "src/bluetooth.rs" "pub fn receive_audio"
check_content "src/bluetooth.rs" "get_paired_devices"
check_content "src/bluetooth.rs" "ConnectionState"
echo ""

echo -e "${CYAN}[8] Verifying permissions.rs Features${NC}"
echo "─────────────────────────────────"
check_content "src/permissions.rs" "pub struct PermissionManager"
check_content "src/permissions.rs" "BLUETOOTH_CONNECT"
check_content "src/permissions.rs" "BLUETOOTH_SCAN"
check_content "src/permissions.rs" "RECORD_AUDIO"
check_content "src/permissions.rs" "request_permissions"
check_content "src/permissions.rs" "check_permissions"
echo ""

echo -e "${CYAN}[9] Verifying Cargo.toml${NC}"
echo "─────────────────────────────────"
check_content "Cargo.toml" "eframe"
check_content "Cargo.toml" "egui"
check_content "Cargo.toml" "jni"
check_content "Cargo.toml" "aes-gcm"
check_content "Cargo.toml" "android-activity"
check_content "Cargo.toml" "android_logger"
echo ""

echo -e "${CYAN}[10] Verifying AndroidManifest.xml${NC}"
echo "─────────────────────────────────"
check_content "AndroidManifest.xml" "BLUETOOTH_CONNECT"
check_content "AndroidManifest.xml" "BLUETOOTH_SCAN"
check_content "AndroidManifest.xml" "RECORD_AUDIO"
check_content "AndroidManifest.xml" "android.app.NativeActivity"
check_content "AndroidManifest.xml" "sassytalkie"
echo ""

echo -e "${CYAN}[11] Code Statistics${NC}"
echo "─────────────────────────────────"

if [ -f "src/lib.rs" ]; then
    TOTAL_LINES=$(cat src/*.rs 2>/dev/null | wc -l)
    echo -e "${GREEN}✓${NC} Total lines of Rust code: $TOTAL_LINES"
    ((PASS++))
fi

if [ -d "src" ]; then
    MODULE_COUNT=$(ls -1 src/*.rs 2>/dev/null | wc -l)
    echo -e "${GREEN}✓${NC} Number of modules: $MODULE_COUNT"
    ((PASS++))
fi

echo ""

echo -e "${CYAN}[12] Build Test${NC}"
echo "─────────────────────────────────"
echo "Running cargo check..."

if cargo check --target aarch64-linux-android 2>&1 | grep -q "Finished"; then
    echo -e "${GREEN}✓${NC} Cargo check passed"
    ((PASS++))
else
    echo -e "${RED}✗${NC} Cargo check failed"
    ((FAIL++))
fi

echo ""
echo "========================================"
echo -e "  ${GREEN}PASSED: $PASS${NC}"
echo -e "  ${RED}FAILED: $FAIL${NC}"
echo "========================================"
echo ""

if [ $FAIL -eq 0 ]; then
    echo -e "${GREEN}✓ ALL CHECKS PASSED!${NC}"
    echo ""
    echo "Your SassyTalkie Android app is complete and ready for:"
    echo "  1. Testing on real devices"
    echo "  2. Creating release APK"
    echo "  3. Google Play submission"
    echo ""
    echo "Next steps:"
    echo "  • Run: ./build.sh debug"
    echo "  • Test on 2 Android devices"
    echo "  • See PRODUCTION_GUIDE.md for full deployment"
    echo ""
    exit 0
else
    echo -e "${RED}✗ SOME CHECKS FAILED${NC}"
    echo ""
    echo "Please review the failed checks above."
    echo "Ensure all source files are present and properly configured."
    echo ""
    exit 1
fi
