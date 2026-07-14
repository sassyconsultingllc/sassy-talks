#requires -Version 7
<#
.SYNOPSIS
  Create a SassyTalkie promo code on the relay license service (direct APK).

.EXAMPLE
  $env:LICENSE_ADMIN_TOKEN = '...'
  pwsh -File scripts/create-sassytalk-promo.ps1 -Code FRIENDS-FAMILY -MaxRedemptions 50 -Note "friends & family"

  pwsh -File scripts/create-sassytalk-promo.ps1   # server generates SASSYTALK-XXXXXX
#>
[CmdletBinding()]
param(
    [string]$Code,
    [int]$MaxRedemptions = 100,
    [int]$ExpiresDays = 0,
    [string]$Note = "friends & family",
    [string]$RelayUrl = "https://relay.sassyconsultingllc.com",
    [string]$AdminToken = $env:LICENSE_ADMIN_TOKEN
)

$ErrorActionPreference = 'Stop'
if (-not $AdminToken) { throw "Set LICENSE_ADMIN_TOKEN (relay admin bearer token)." }

$body = @{
    max_redemptions = $MaxRedemptions
    note            = $Note
}
if ($Code) { $body.code = $Code }
if ($ExpiresDays -gt 0) { $body.expires_days = $ExpiresDays }

$json = $body | ConvertTo-Json
$resp = Invoke-RestMethod -Uri "$RelayUrl/license/promo-create" -Method Post `
    -Headers @{ Authorization = "Bearer $AdminToken"; 'Content-Type' = 'application/json' } `
    -Body $json

if (-not $resp.ok) { throw "Promo create failed: $($resp | ConvertTo-Json -Compress)" }

Write-Host "Promo created:" -ForegroundColor Green
Write-Host "  Code:            $($resp.code)"
Write-Host "  Max redemptions: $($resp.max_redemptions)"
Write-Host "  Expires:         $(if ($resp.expires_at) { $resp.expires_at } else { 'never' })"
Write-Host ""
Write-Host "Share this code — recipients enter it on the app's activation screen (direct APK)."
