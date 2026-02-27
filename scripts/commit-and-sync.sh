#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || true)
if [ -z "$REPO_ROOT" ]; then
  echo "Not inside a git repository." >&2
  exit 1
fi
cd "$REPO_ROOT"

MSG=""
if [ $# -gt 0 ]; then
  MSG="$*"
else
  read -r -p "Commit message (leave empty to skip commit): " MSG
fi

git add -A
if git diff --cached --quiet; then
  echo "No changes to commit."
else
  if [ -z "$MSG" ]; then
    echo "No commit message provided; skipping commit."
  else
    git commit -m "$MSG"
  fi
fi

BRANCH=$(git rev-parse --abbrev-ref HEAD)
if git rev-parse --abbrev-ref --symbolic-full-name @{u} >/dev/null 2>&1; then
  : # upstream exists
else
  if git remote | grep -q '^origin$'; then
    git fetch origin "$BRANCH" || true
    git branch --set-upstream-to=origin/"$BRANCH" "$BRANCH" 2>/dev/null || true
  else
    echo "No 'origin' remote found; skipping pull/push."
    exit 0
  fi
fi

# Pull with rebase and autostash
if ! git pull --rebase --autostash; then
  echo "git pull failed. Resolve conflicts and try again." >&2
  exit 1
fi

git push --set-upstream origin "$BRANCH"
