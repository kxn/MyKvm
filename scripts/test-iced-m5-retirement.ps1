[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
    param(
        [Parameter(Mandatory)]
        [bool]$Condition,
        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $root
try {
    $metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed"
    }
    Assert-True (
        @($metadata.packages | Where-Object { $_.name -eq "ipkvm-desktop-iced-spike" }).Count -eq 0
    ) "workspace still contains ipkvm-desktop-iced-spike"

    $tree = cargo tree --workspace --all-features 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tree failed`n$tree"
    }
    Assert-True (
        $tree -notmatch '(?m)(^|[├└]── )(eframe|egui) v'
    ) "workspace dependency tree still contains egui"

    $desktopManifest = Get-Content -Raw -Encoding UTF8 crates/ipkvm-desktop/Cargo.toml
    Assert-True ($desktopManifest -notmatch '(?m)^eframe\s*=') "ipkvm-desktop still declares eframe"
    Assert-True ($desktopManifest -notmatch '(?m)^wgpu\s*=') "ipkvm-desktop still declares wgpu"
    Assert-True (-not (Test-Path crates/ipkvm-desktop/src/main.rs)) "egui desktop binary entry still exists"
    Assert-True (-not (Test-Path crates/ipkvm-desktop/src/app.rs)) "egui desktop UI source still exists"
    Assert-True (-not (Test-Path crates/ipkvm-desktop-iced-spike)) "iced spike directory still exists"

    $icedMain = Get-Content -Raw -Encoding UTF8 crates/ipkvm-desktop-iced/src/main.rs
    Assert-True (
        $icedMain.Contains('#![cfg_attr(windows, windows_subsystem = "windows")]')
    ) "iced entry lacks Windows GUI subsystem"
    Assert-True (Test-Path crates/ipkvm-desktop-iced/assets/icon.ico) "iced Windows icon is missing"
    Assert-True (Test-Path crates/ipkvm-desktop-iced/assets/icon.rc) "iced Windows resource script is missing"
}
finally {
    Pop-Location
}

Write-Host "M5 desktop retirement gate passed."
