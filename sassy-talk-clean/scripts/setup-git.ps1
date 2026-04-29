#!/usr/bin/env pwsh
Param(
  [string]$OriginUrl
)
try {
  $repoRoot = (& git rev-parse --show-toplevel) 2>$null
} catch {}
if (-not $repoRoot) {
  Write-Error "Not inside a git repository."
  exit 1
}
Push-Location $repoRoot
# Add origin remote if missing and URL provided
$hasOrigin = (& git remote) | Select-String -Pattern '^origin$' -Quiet
if (-not $hasOrigin) {
  if ($OriginUrl) {
    & git remote add origin $OriginUrl
    Write-Output "Added origin -> $OriginUrl"
  } else {
    Write-Output "No origin remote and no URL provided."
  }
} else {
  Write-Output "Origin remote already exists."
}

# Ensure user.name and user.email are set
$name = (& git config user.name) 2>$null
$email = (& git config user.email) 2>$null
if (-not $name) {
  $envName = $env:GIT_AUTHOR_NAME
  if ($envName) {
    & git config user.name "$envName"
    Write-Output "Set git user.name from GIT_AUTHOR_NAME"
  } else {
    Write-Output "Git user.name not set."
  }
}
if (-not $email) {
  $envEmail = $env:GIT_AUTHOR_EMAIL
  if ($envEmail) {
    & git config user.email "$envEmail"
    Write-Output "Set git user.email from GIT_AUTHOR_EMAIL"
  } else {
    Write-Output "Git user.email not set."
  }
}
Pop-Location
exit 0
