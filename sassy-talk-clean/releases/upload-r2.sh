#!/bin/bash
# Upload release APK to Cloudflare R2 (replaces previous build)
# Requires: CLOUDFLARE_API_TOKEN env var or wrangler login
#
# Usage: ./upload-r2.sh [version]
# Example: ./upload-r2.sh v1.1.0

set -e

VERSION="${1:-v1.1.0}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKER_DIR="$SCRIPT_DIR/../cloudflare-worker"

APK="$SCRIPT_DIR/$VERSION/sassytalkie.apk"
AAB="$SCRIPT_DIR/$VERSION/sassytalkie.aab"

if [ ! -f "$APK" ]; then
    echo "Error: $APK not found"
    exit 1
fi

echo "Uploading sassytalkie.apk ($VERSION) to R2..."
cd "$WORKER_DIR"

# Upload APK to R2 (replaces existing)
npx wrangler r2 object put sassy-talk-downloads/sassy-talk/android/sassytalkie.apk \
    --file "$APK" \
    --content-type "application/vnd.android.package-archive"

echo "APK uploaded to R2: sassy-talk/android/sassytalkie.apk"
echo "Download URL: https://sassyconsultingllc.com/download/sassy-talk/android/sassytalkie.apk"

if [ -f "$AAB" ]; then
    echo ""
    echo "AAB available at: $AAB"
    echo "Upload to Google Play Console manually."
fi

echo "Done."
