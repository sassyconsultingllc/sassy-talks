#!/usr/bin/env pwsh
Param(
  [string]$Message
)
try {
  $repoRoot = (& git rev-parse --show-toplevel) 2>$null
} catch {}
if (-not $repoRoot) {
  Write-Error "Not inside a git repository."
  exit 1
}
Push-Location $repoRoot
if (-not $Message) {
  $Message = Read-Host "Commit message (leave empty to skip commit)"
}
& git add -A
$staged = (& git diff --cached --name-only)
if ($staged) {
  if (-not $Message) {
    Write-Output "No commit message provided; skipping commit."
  } else {
    & git commit -m $Message
  }
} else {
  Write-Output "No changes to commit."
}
$branch = (& git rev-parse --abbrev-ref HEAD).Trim()
# Ensure an upstream exists or set it to origin/$branch when possible
$upstream = (& git rev-parse --abbrev-ref --symbolic-full-name '@{u}') 2>$null
if ($LASTEXITCODE -ne 0) {
  $hasOrigin = (& git remote) | Select-String -Pattern '^origin$' -Quiet
  if ($hasOrigin) {
    & git fetch origin $branch
    & git branch --set-upstream-to=origin/$branch $branch 2>$null
  } else {
    Write-Output "No 'origin' remote found; skipping pull/push."
    Pop-Location
    exit 0
  }
}
# Pull with rebase and autostash for a clean sync
& git pull --rebase --autostash
if ($LASTEXITCODE -ne 0) {
  Write-Error "git pull failed. Resolve conflicts and try again."
  Pop-Location
  exit 1
}
& git push --set-upstream origin $branch
Pop-Location
exit 0
