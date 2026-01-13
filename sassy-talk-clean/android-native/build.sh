#!/bin/bash
# SassyTalkie Complete Build Script
# Builds production-ready Android APK

set -e

echo "==================================="
echo "  SassyTalkie Android Builder v1.0"
echo "  Sassy Consulting LLC"
echo "==================================="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
BUILD_MODE="${1:-debug}"
TARGET="aarch64-linux-android"
NDK_VERSION="25.2.9519653"

echo -e "${CYAN}Build Configuration:${NC}"
echo "  Mode: $BUILD_MODE"
echo "  Target: $TARGET"
echo ""

# Step 1: Check prerequisites
echo -e "${CYAN}[1/6] Checking prerequisites...${NC}"

if ! command -v cargo &> /dev/null; then
    echo -e "${RED}✗ Cargo not found. Please install Rust.${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Cargo found${NC}"

if ! command -v cargo-ndk &> /dev/null; then
    echo -e "${YELLOW}⚠ cargo-ndk not found. Installing...${NC}"
    cargo install cargo-ndk
fi
echo -e "${GREEN}✓ cargo-ndk found${NC}"

# Step 2: Check Android target
echo ""
echo -e "${CYAN}[2/6] Checking Android target...${NC}"

if ! rustup target list --installed | grep -q "$TARGET"; then
    echo -e "${YELLOW}⚠ Target $TARGET not installed. Installing...${NC}"
    rustup target add "$TARGET"
fi
echo -e "${GREEN}✓ Target $TARGET installed${NC}"

# Step 3: Verify all modules exist
echo ""
echo -e "${CYAN}[3/6] Verifying source files...${NC}"

REQUIRED_FILES=(
    "src/lib.rs"
    "src/bluetooth.rs"
    "src/jni_bridge.rs"
    "src/audio.rs"
    "src/state.rs"
    "src/permissions.rs"
    "Cargo.toml"
    "AndroidManifest.xml"
)

for file in "${REQUIRED_FILES[@]}"; do
    if [ ! -f "$file" ]; then
        echo -e "${RED}✗ Missing required file: $file${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ $file${NC}"
done

# Step 4: Run cargo check
echo ""
echo -e "${CYAN}[4/6] Running cargo check...${NC}"

if cargo check --target "$TARGET" 2>&1 | tee /tmp/cargo_check.log; then
    echo -e "${GREEN}✓ Cargo check passed${NC}"
else
    echo -e "${RED}✗ Cargo check failed. See errors above.${NC}"
    exit 1
fi

# Step 5: Build APK
echo ""
echo -e "${CYAN}[5/6] Building APK ($BUILD_MODE mode)...${NC}"

if [ "$BUILD_MODE" = "release" ]; then
    cargo ndk -t "$TARGET" -o ./jniLibs build --release
    echo -e "${GREEN}✓ Release APK built${NC}"
else
    cargo ndk -t "$TARGET" -o ./jniLibs build
    echo -e "${GREEN}✓ Debug APK built${NC}"
fi

# Step 6: Show results
echo ""
echo -e "${CYAN}[6/6] Build Summary${NC}"

if [ -d "./jniLibs/$TARGET" ]; then
    LIB_SIZE=$(du -h "./jniLibs/$TARGET/libsassytalkie.so" | cut -f1)
    echo -e "${GREEN}✓ Native library built: $LIB_SIZE${NC}"
    echo "  Location: ./jniLibs/$TARGET/libsassytalkie.so"
else
    echo -e "${RED}✗ Build artifacts not found${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}==================================="
echo "  ✓ Build completed successfully!"
echo "===================================${NC}"
echo ""
echo -e "${CYAN}Next Steps:${NC}"
echo "  1. Package APK: ./package_apk.sh"
echo "  2. Or use Android Studio to build complete APK"
echo "  3. Test on device: adb install -r app-debug.apk"
echo ""
echo -e "${YELLOW}Note: For production release:${NC}"
echo "  - Create keystore: keytool -genkey -v -keystore sassy-talk.keystore"
echo "  - Build release: ./build.sh release"
echo "  - Sign APK with keystore"
echo ""
