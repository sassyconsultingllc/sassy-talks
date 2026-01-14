#!/bin/bash
# SassyTalkie iOS Build Script
# Builds Rust library for iOS devices and simulator

set -e

echo "🍎 Building SassyTalkie for iOS..."

# iOS targets
TARGETS=(
    "aarch64-apple-ios"           # iPhone/iPad (arm64)
    "x86_64-apple-ios"            # Simulator (Intel)
    "aarch64-apple-ios-sim"       # Simulator (Apple Silicon)
)

# Add targets if not already installed
for TARGET in "${TARGETS[@]}"; do
    echo "📦 Adding target: $TARGET"
    rustup target add $TARGET 2>/dev/null || true
done

# Build for all targets
for TARGET in "${TARGETS[@]}"; do
    echo "🔨 Building for $TARGET..."
    cargo build --release --target $TARGET
done

# Create universal library for simulator
echo "🔗 Creating universal simulator library..."
mkdir -p target/universal-sim/release

lipo -create \
    target/x86_64-apple-ios/release/libsassytalkie_ios.a \
    target/aarch64-apple-ios-sim/release/libsassytalkie_ios.a \
    -output target/universal-sim/release/libsassytalkie_ios.a

echo "✅ iOS build complete!"
echo ""
echo "📱 Device library: target/aarch64-apple-ios/release/libsassytalkie_ios.a"
echo "🖥️  Simulator library: target/universal-sim/release/libsassytalkie_ios.a"
echo ""
echo "Next steps:"
echo "1. Open Xcode project"
echo "2. Link libraries in Build Phases"
echo "3. Add Bridging-Header.h to project"
echo "4. Build and run"
