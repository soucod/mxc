<#
.SYNOPSIS
    Applies or reverts the SDK license override for internal npm publishes.

.DESCRIPTION
    Mirrors the telemetry GUID substitution pattern: when the MXC_LICENSE_OVERRIDE
    env var is set and points to a valid file, the public MIT-only LICENSE.md in
    sdk/ is backed up (.public) and replaced with the private EULA that includes
    the Section 2 DATA clause for telemetry disclosure.

    When the env var is unset or empty, the original MIT license is restored from
    the .public backup (if one exists). This prevents a stale private EULA from
    persisting across incremental builds.

    Usage:
      # Apply (CI pipeline sets the env var):
      $env:MXC_LICENSE_OVERRIDE = "path\to\private-eula.md"
      .\scripts\apply-license-override.ps1

      # Revert (local dev, env var not set):
      .\scripts\apply-license-override.ps1

.NOTES
    The private EULA file must NEVER be committed to the public repo.
    It is sourced from an internal artifact store at publish time.
#>

$ErrorActionPreference = 'Stop'

$sdkDir = Join-Path $PSScriptRoot '..' 'sdk'
$licensePath = Join-Path $sdkDir 'LICENSE.md'
$backupPath = "$licensePath.public"

$overridePath = $env:MXC_LICENSE_OVERRIDE

if ($overridePath -and (Test-Path $overridePath)) {
    # Save the public MIT license if not already backed up.
    if (-not (Test-Path $backupPath)) {
        Copy-Item $licensePath $backupPath -Force
        Write-Host "Backed up public LICENSE.md -> LICENSE.md.public"
    }

    Copy-Item $overridePath $licensePath -Force
    Write-Host "Applied private EULA from: $overridePath"
}
else {
    # Restore public license if a backup exists (revert scenario).
    if (Test-Path $backupPath) {
        Copy-Item $backupPath $licensePath -Force
        Remove-Item $backupPath -Force
        Write-Host "Restored public LICENSE.md from backup"
    }
    else {
        Write-Host "No override set and no backup found — LICENSE.md unchanged"
    }
}
