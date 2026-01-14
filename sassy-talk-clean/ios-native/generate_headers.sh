#!/bin/bash
# Generate Xcode project using cbindgen for header generation

set -e

echo "🔧 Generating C headers from Rust..."

# Install cbindgen if not present
if ! command -v cbindgen &> /dev/null; then
    echo "📦 Installing cbindgen..."
    cargo install cbindgen
fi

# Generate header
cbindgen --config cbindgen.toml --crate sassytalkie-ios --output SassyTalkie-Generated.h

echo "✅ Header generated: SassyTalkie-Generated.h"
echo ""
echo "Next steps:"
echo "1. Open Xcode"
echo "2. Create new iOS App project"
echo "3. Add Swift files to project"
echo "4. Add Bridging-Header.h to project"
echo "5. Link static libraries"
echo "6. Build and run"
