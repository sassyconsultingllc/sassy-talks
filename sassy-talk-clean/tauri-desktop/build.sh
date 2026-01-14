#!/bin/bash

# SassyTalkie Desktop - Build Script
# Builds for Windows, Mac, and Linux

set -e

echo "╔══════════════════════════════════════════╗"
echo "║  SassyTalkie Desktop Build Script       ║"
echo "║  v1.0.0                                  ║"
echo "╚══════════════════════════════════════════╝"
echo ""

# Check prerequisites
echo "→ Checking prerequisites..."

# Check Node.js
if ! command -v node &> /dev/null; then
    echo "❌ Node.js not found. Please install Node.js 18+"
    exit 1
fi
echo "✓ Node.js $(node --version)"

# Check npm
if ! command -v npm &> /dev/null; then
    echo "❌ npm not found. Please install npm"
    exit 1
fi
echo "✓ npm $(npm --version)"

# Check Rust
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust not found. Please install Rust from https://rustup.rs"
    exit 1
fi
echo "✓ Rust $(rustc --version)"

echo ""

# Install dependencies if needed
if [ ! -d "node_modules" ]; then
    echo "→ Installing Node dependencies..."
    npm install
    echo "✓ Dependencies installed"
    echo ""
fi

# Build type
BUILD_TYPE="${1:-dev}"

if [ "$BUILD_TYPE" = "release" ]; then
    echo "→ Building RELEASE version..."
    echo "  - Optimizations: ON"
    echo "  - Debug symbols: OFF"
    echo "  - Strip binaries: YES"
    echo ""
    
    npm run tauri build
    
    echo ""
    echo "╔══════════════════════════════════════════╗"
    echo "║  BUILD COMPLETE - RELEASE                ║"
    echo "╚══════════════════════════════════════════╝"
    echo ""
    echo "Release builds located in:"
    echo "  src-tauri/target/release/bundle/"
    echo ""
    
elif [ "$BUILD_TYPE" = "dev" ]; then
    echo "→ Building DEV version..."
    echo "  - Optimizations: OFF"
    echo "  - Debug symbols: ON"
    echo "  - Hot reload: YES"
    echo ""
    
    npm run tauri dev
    
else
    echo "❌ Invalid build type: $BUILD_TYPE"
    echo "Usage: ./build.sh [dev|release]"
    exit 1
fi
