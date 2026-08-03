# Darwin cross-check script for iced spike (#73 macOS portability gate).
#
# Goal (#73): catch cfg/platform mistakes early via
#   cargo check --target x86_64-apple-darwin
# for all three spikes (code-level cross-platform, no real macOS verification).
#
# ENVIRONMENT LIMITATION (Windows host): a real cross-compile to darwin needs a
# macOS C toolchain (cc/clang + macOS SDK). On this Windows host, the check fails
# at the build-script stage of macOS-only C deps (objc_exception, core-foundation
# chain) with "failed to find tool cc" — NOT a Rust cfg error. This is an
# environment limit, not a code defect, and matches #73's note that real macOS
# verification waits for a macOS machine/CI.
#
# This script:
#   1. Reports the host environment.
#   2. Runs the darwin cargo check and classifies the result:
#      - SUCCESS: no platform cfg errors (ideal).
#      - CC-MISSING (expected on Windows): documents the env limit; code review
#        of cfg(windows)/cfg(target_os) guards is the fallback.
#   3. On CC-MISSING, greps the spike crate sources for cfg gates so a human can
#      eyeball that platform differences are isolated.

[CmdletBinding()]
param()

$ErrorActionPreference = "Continue"
Set-StrictMode -Version Latest

Write-Host "==> Darwin cross-check (spike crate)"
Write-Host "    host: $($env:OS) / target: x86_64-apple-darwin"

$repoRoot = (Resolve-Path (Join-Path (Join-Path (Join-Path $PSScriptRoot "..") "..") "..")).Path
Set-Location $repoRoot

# Try the real darwin check. Capture all output; do not abort on nonzero exit.
$output = & cargo check -p ipkvm-desktop-iced-spike --target x86_64-apple-darwin --lib 2>&1
$exitCode = $LASTEXITCODE
$joined = ($output | Out-String)

if ($exitCode -eq 0) {
    Write-Host "==> result: darwin cargo check PASSED (no platform cfg errors)"
    exit 0
}

# Classify failure: cc-missing (env limit) vs real cfg error.
if ($joined -match "failed to find tool `"cc`"" -or $joined -match "ToolNotFound") {
    Write-Host "==> result: CC-MISSING (expected on Windows host)"
    Write-Host "    The darwin cargo check failed on a macOS C dependency (objc/core-foundation),"
    Write-Host "    NOT on a Rust cfg error. This is an environment limit (no macOS C toolchain)."
    Write-Host "    Fallback: code review of cfg gates below."
    Write-Host ""
    Write-Host "==> cfg gate review (manual portability audit)"
    # Grep spike crate for cfg windows/target_os guards.
    $spikeSrc = Join-Path $repoRoot "crates\ipkvm-desktop-iced-spike\src"
    $cfgLines = Select-String -Path (Join-Path $spikeSrc "*.rs") -Pattern "cfg\(" -ErrorAction SilentlyContinue
    if ($cfgLines) {
        $cfgLines | ForEach-Object { Write-Host "    $($_.Filename):$($_.LineNumber): $($_.Line.Trim())" }
    } else {
        Write-Host "    (no cfg gates in spike src — platform differences not yet isolated;"
        Write-Host "     revisit when platform-specific code lands in Spike 3)"
    }
    Write-Host ""
    Write-Host "==> darwin check: SKIPPED (env), portability audit done -- wait for macOS machine/CI"
    exit 0
}

# Real cfg error.
Write-Host "==> result: darwin cargo check FAILED with platform cfg error"
Write-Host $joined
exit $exitCode
