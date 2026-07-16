# Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
# Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
# CodeMark: SCLLC1-sassytalkie-KZSNVINZQ22J
#requires -Version 7
<#
.SYNOPSIS
  One-shot: deploy relay license service, wire website checkout, create F&F promo.

  Run from: sassy-talk-clean/cloudflare-worker/

  Requires: wrangler auth, CLOUDFLARE_ACCOUNT_ID (optional if wrangler.toml has account_id)

  Optional env:
    LICENSE_SALT              — reuse existing salt (else generated)
    LICENSE_ADMIN_TOKEN       — reuse existing admin token (else generated)
    LEMON_SQUEEZY_TEST_KEY    — for finish-lemonsqueezy.ps1 variant wiring
    SKIP_LS                   — set to skip Lemon Squeezy variant wiring
    SKIP_PROMO                — set to skip FRIENDS-FAMILY promo creation
#>
[CmdletBinding()]
param(
    [string]$PromoCode = "10241991",
    [int]$PromoMaxRedemptions = 200
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$RelayDir = Join-Path $Root "cloudflare-worker"
$WebsiteDir = "V:\Projects\sassyconsultingllc-cloudflare"

function New-Hex([int]$Bytes = 32) {
    -join ((1..$Bytes) | ForEach-Object { '{0:x2}' -f (Get-Random -Max 256) })
}

function Set-Secret([string]$Name, [string]$Value, [string]$Dir) {
    Push-Location $Dir
    try {
        $Value | npx --yes wrangler@latest secret put $Name | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "wrangler secret put $Name failed" }
        Write-Host "  secret: $Name" -ForegroundColor Green
    } finally { Pop-Location }
}

Write-Host "=== SassyTalk licensing setup ===" -ForegroundColor Cyan

$salt = if ($env:LICENSE_SALT) { $env:LICENSE_SALT } else { New-Hex 32 }
$admin = if ($env:LICENSE_ADMIN_TOKEN) { $env:LICENSE_ADMIN_TOKEN } else { New-Hex 32 }

Write-Host "[1/6] D1 schema (sassytalkie-licenses)"
Push-Location $RelayDir
npx --yes wrangler@latest d1 execute sassytalkie-licenses --remote --file=schema/licenses.sql
if ($LASTEXITCODE -ne 0) { throw "D1 schema apply failed" }
Pop-Location

Write-Host "[2/6] Relay secrets + deploy"
Set-Secret 'LICENSE_SALT' $salt $RelayDir
Set-Secret 'LICENSE_ADMIN_TOKEN' $admin $RelayDir
Push-Location $RelayDir
npx --yes wrangler@latest deploy 2>&1 | Select-Object -Last 5
if ($LASTEXITCODE -ne 0) { throw "relay deploy failed" }
Pop-Location

Write-Host "[3/6] Website worker secret (relay admin for paid-key issuance)"
Set-Secret 'LICENSE_RELAY_ADMIN_TOKEN' $admin $WebsiteDir

Write-Host "[4/6] Website worker deploy"
Push-Location $WebsiteDir
npx --yes wrangler@latest deploy 2>&1 | Select-Object -Last 5
if ($LASTEXITCODE -ne 0) { throw "website deploy failed" }
Pop-Location

if (-not $env:SKIP_LS) {
    Write-Host "[5/6] Lemon Squeezy variant wiring"
    $lsScript = Join-Path $WebsiteDir "scripts/finish-lemonsqueezy.ps1"
    if ($env:LEMON_SQUEEZY_TEST_KEY -or $env:LEMONSQUEEZY_API_KEY) {
        Push-Location $WebsiteDir
        & $lsScript
        Pop-Location
    } else {
        Write-Host "  SKIP: set LEMON_SQUEEZY_TEST_KEY and re-run finish-lemonsqueezy.ps1" -ForegroundColor Yellow
        Write-Host "  Also set Sassy-Talk to `$3.99 in the Lemon Squeezy dashboard."
    }
} else {
    Write-Host "[5/6] Lemon Squeezy skipped (SKIP_LS set)"
}

if (-not $env:SKIP_PROMO) {
    Write-Host "[6/6] Friends & family promo: $PromoCode"
    $env:LICENSE_ADMIN_TOKEN = $admin
    & (Join-Path $PSScriptRoot "create-sassytalk-promo.ps1") `
        -Code $PromoCode -MaxRedemptions $PromoMaxRedemptions -Note "friends & family launch"
} else {
    Write-Host "[6/6] Promo skipped (SKIP_PROMO set)"
}

Write-Host ""
Write-Host "Done. Save these offline (shown once if generated):" -ForegroundColor Green
Write-Host "  LICENSE_SALT=$salt"
Write-Host "  LICENSE_ADMIN_TOKEN=$admin"
Write-Host ""
Write-Host "Verify:"
Write-Host "  curl -X POST https://relay.sassyconsultingllc.com/license/promo -H 'Content-Type: application/json' -d '{\"code\":\"$PromoCode\",\"device_id\":\"test\"}'"
