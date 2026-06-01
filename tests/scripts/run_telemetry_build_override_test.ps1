<#
.SYNOPSIS
    Validates the MXC telemetry build.rs override mechanism.

.DESCRIPTION
    Exercises three scenarios for the MXC_TELEMETRY_CONFIG_OVERRIDE mechanism:
      1. Public build (no env var) — WIL's default traceloggingconfig.h is used.
      2. Override build — a dummy .h file is copied over the WIL header.
      3. Revert build — removing the env var restores the .public backup.

    All tests use a harmless dummy header (no private GUIDs).

    Requires: cargo, Rust toolchain, MSVC. Run from the repo root.
#>

[CmdletBinding()]
param(
    [switch]$SkipClean
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$srcDir = Join-Path $repoRoot 'src'
$crateDir = Join-Path $srcDir 'mxc_wil_telemetry'

Write-Host "=== Telemetry Build Override Validation ===" -ForegroundColor Cyan
Write-Host "Repo root: $repoRoot"
Write-Host "Crate dir: $crateDir"

# Create a dummy override header with safe placeholder content.
$dummyDir = Join-Path $env:TEMP 'mxc_telemetry_test'
if (Test-Path $dummyDir) { Remove-Item -Recurse -Force $dummyDir }
New-Item -ItemType Directory -Path $dummyDir -Force | Out-Null
$dummyHeader = Join-Path $dummyDir 'MicrosoftTelemetry.h'
Set-Content -Path $dummyHeader -Value @"
// Dummy override header for build validation test.
// This is NOT a real telemetry config — it is a safe placeholder.
#pragma once
#define DUMMY_TELEMETRY_BUILD_TEST 1
"@

# ---------------------------------------------------------------------------
# Scenario 1: Public build (no override)
# ---------------------------------------------------------------------------
Write-Host "`n--- Scenario 1: Public build (no env var) ---" -ForegroundColor Yellow

# Ensure env var is not set.
$env:MXC_TELEMETRY_CONFIG_OVERRIDE = $null
Remove-Item Env:\MXC_TELEMETRY_CONFIG_OVERRIDE -ErrorAction SilentlyContinue

Push-Location $srcDir
try {
    Write-Host "Building mxc_wil_telemetry (public stub)..."
    cargo build -p mxc_wil_telemetry 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Scenario 1 FAILED: public build should succeed"
    }
    Write-Host "Scenario 1 PASSED: public build succeeded" -ForegroundColor Green
} finally {
    Pop-Location
}

# ---------------------------------------------------------------------------
# Scenario 2: Override build (dummy header)
# ---------------------------------------------------------------------------
Write-Host "`n--- Scenario 2: Override build (dummy header) ---" -ForegroundColor Yellow

$env:MXC_TELEMETRY_CONFIG_OVERRIDE = $dummyHeader
Write-Host "MXC_TELEMETRY_CONFIG_OVERRIDE = $dummyHeader"

Push-Location $srcDir
try {
    Write-Host "Building mxc_wil_telemetry (override)..."
    cargo build -p mxc_wil_telemetry 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Scenario 2 FAILED: override build should succeed"
    }
    Write-Host "Scenario 2 PASSED: override build succeeded" -ForegroundColor Green
} finally {
    Pop-Location
}

# ---------------------------------------------------------------------------
# Scenario 3: Revert build (remove override, .public backup restored)
# ---------------------------------------------------------------------------
Write-Host "`n--- Scenario 3: Revert build (no env var, .public restored) ---" -ForegroundColor Yellow

$env:MXC_TELEMETRY_CONFIG_OVERRIDE = $null
Remove-Item Env:\MXC_TELEMETRY_CONFIG_OVERRIDE -ErrorAction SilentlyContinue

Push-Location $srcDir
try {
    Write-Host "Building mxc_wil_telemetry (revert)..."
    cargo build -p mxc_wil_telemetry 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Scenario 3 FAILED: revert build should succeed"
    }
    Write-Host "Scenario 3 PASSED: revert build succeeded" -ForegroundColor Green
} finally {
    Pop-Location
}

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------
if (-not $SkipClean) {
    Remove-Item -Recurse -Force $dummyDir -ErrorAction SilentlyContinue
}

Write-Host "`n=== All telemetry build override scenarios PASSED ===" -ForegroundColor Green
