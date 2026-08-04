[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Get-Tree {
    param([string]$Package)
    $output = & cargo tree -p $Package --edges normal 2>&1
    if ($LASTEXITCODE -ne 0) { throw "cargo tree failed for $Package" }
    return ($output -join "`n")
}

$metadataJson = & cargo metadata --format-version 1 --no-deps
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed" }
$metadata = $metadataJson | ConvertFrom-Json
$packages = @{}
foreach ($package in $metadata.packages) { $packages[$package.name] = $package }

foreach ($name in @(
    "ipkvm-device",
    "ipkvm-headless",
    "ipkvm-headless-app",
    "ipkvm-headless-demo",
    "ipkvm-browser-fixture",
    "ipkvm-desktop-core"
)) {
    Assert-True $packages.ContainsKey($name) "Missing workspace package: $name"
}

Assert-True (@($packages["ipkvm-headless-app"].targets | Where-Object name -eq "ipkvm-headless").Count -eq 1) "Formal headless binary is missing"
Assert-True (@($packages["ipkvm-headless-demo"].targets | Where-Object name -eq "ipkvm-demo").Count -eq 1) "Demo binary is missing"
Assert-True (@($packages["ipkvm-browser-fixture"].targets | Where-Object name -eq "ipkvm-browser-fixture").Count -eq 1) "Browser fixture binary is missing"
Assert-True (-not $packages.ContainsKey("ipkvm-desktop-iced-spike")) "Retired iced spike remains in workspace"

$headlessTree = Get-Tree "ipkvm-headless"
$fixtureTree = Get-Tree "ipkvm-browser-fixture"
$desktopCoreTree = Get-Tree "ipkvm-desktop-core"

Assert-True ($headlessTree -notmatch "serialport|nokhwa|windows v|ipkvm-video.*camera") "Headless library leaks a hardware backend"
Assert-True ($fixtureTree -notmatch "serialport|nokhwa|ipkvm-device.*platform|windows v") "Browser fixture leaks a hardware provider"
Assert-True ($desktopCoreTree -notmatch "serialport|nokhwa|iced v|eframe|egui|windows v") "Desktop core leaks UI or hardware dependencies"

Write-Host "Crate boundary checks passed."
$global:LASTEXITCODE = 0
