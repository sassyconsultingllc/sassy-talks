#!/usr/bin/env bash
# Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
# Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
# scripts/release-play.sh: bump the Android version, push, and have CI build,
# sign and upload the Play AAB (build-aab.yml, publish_to_play=true).
#
# Usage:
#   scripts/release-play.sh [--no-bump] [--track internal|alpha|beta|production]
#
#   (default)  versionCode += 1, patch of versionName += 1, CHANGELOG header,
#              commit "release(android): ship vX.Y.Z (N)"
#   --no-bump  release the version already in build.gradle.kts (must be committed)
#
# Then: push the current branch, dispatch build-aab.yml on it, and watch the run.
# Play rejects a versionCode it has seen before, so bumping is the default.
#
# Needs: gh (authenticated), git. Repo secrets RELEASE_* , GOOGLE_SERVICES_JSON,
# PLAY_SERVICE_ACCOUNT_JSON (the workflow checks their contents itself).
set -euo pipefail

TRACK=internal
BUMP=1
while [ $# -gt 0 ]; do
  case "$1" in
    --no-bump) BUMP=0 ;;
    --track) TRACK="${2:?--track needs a value}"; shift ;;
    -h|--help) sed -n '5,18p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done
case "$TRACK" in internal|alpha|beta|production) ;; *) echo "bad track: $TRACK" >&2; exit 2 ;; esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
GRADLE=android-app/app/build.gradle.kts
WORKFLOW=build-aab.yml

# Preflight: secrets the workflow needs (names only; values never leave GitHub).
have=$(gh secret list --json name -q '.[].name')
missing=0
for s in RELEASE_KEYSTORE_BASE64 RELEASE_STORE_PASSWORD RELEASE_KEY_ALIAS RELEASE_KEY_PASSWORD \
         GOOGLE_SERVICES_JSON PLAY_SERVICE_ACCOUNT_JSON; do
  grep -qx "$s" <<<"$have" || { echo "missing repo secret: $s" >&2; missing=1; }
done
if [ "$missing" -ne 0 ]; then
  echo "Set them with: gh secret set NAME < file" >&2
  exit 1
fi

code=$(sed -nE 's/^[[:space:]]*versionCode = ([0-9]+).*/\1/p' "$GRADLE")
name=$(sed -nE 's/^[[:space:]]*versionName = "([^"]+)".*/\1/p' "$GRADLE")
[ -n "$code" ] && [ -n "$name" ] || { echo "could not read version from $GRADLE" >&2; exit 1; }

if [ "$BUMP" -eq 1 ]; then
  new_code=$((code + 1))
  IFS=. read -r ma mi pa <<<"$name"
  new_name="$ma.$mi.$((pa + 1))"
  # [[:space:]]*$ keeps the file's CRLF endings intact.
  sed -i -E "s/(versionCode = )$code([[:space:]]*)$/\1$new_code\2/; s/(versionName = \")$name(\"[[:space:]]*)$/\1$new_name\2/" "$GRADLE"
  sed -i "0,/^## \[Unreleased\]\r\?$/s//## [Unreleased]\n\n## [$new_name] ($new_code) - $(date +%F)/" CHANGELOG.md
  git add "$GRADLE" CHANGELOG.md
  git commit -q -m "release(android): ship v$new_name ($new_code)"
  code=$new_code name=$new_name
fi

if ! git diff --quiet HEAD -- "$GRADLE"; then
  echo "$GRADLE has uncommitted changes; commit them first" >&2
  exit 1
fi

branch=$(git rev-parse --abbrev-ref HEAD)
sha=$(git rev-parse HEAD)
echo "releasing v$name ($code) from $branch @ ${sha:0:7} to Play track '$TRACK'"
git push -u origin "$branch"

since=$(date -u +%Y-%m-%dT%H:%M:%SZ)
gh workflow run "$WORKFLOW" --ref "$branch" -f publish_to_play=true -f play_track="$TRACK"

# Find the run we just dispatched (matching commit, created after dispatch).
run=""
for _ in $(seq 1 20); do
  sleep 3
  run=$(gh run list --workflow "$WORKFLOW" --event workflow_dispatch --branch "$branch" -L 5 \
          --json databaseId,headSha,createdAt \
          -q "[.[] | select(.headSha==\"$sha\" and .createdAt>=\"$since\")][0].databaseId // empty")
  [ -n "$run" ] && break
done
[ -n "$run" ] || { echo "dispatched, but could not find the run; check: gh run list --workflow $WORKFLOW" >&2; exit 1; }

echo "run: $(gh run view "$run" --json url -q .url)"
if gh run watch "$run" --exit-status --interval 20; then
  echo "v$name ($code) uploaded to Play track '$TRACK'"
else
  echo "run failed; failing step log:" >&2
  gh run view "$run" --log-failed | tail -40 >&2
  exit 1
fi
