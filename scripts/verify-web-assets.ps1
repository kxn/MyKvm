[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Import-Module (Join-Path $PSScriptRoot "web-assets-tools.psm1") -Force

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$noVncRoot = Join-Path $repositoryRoot "third_party/novnc"
$policy = Get-NoVncReleasePolicy

Assert-WebAssetTree `
    -Root (Join-Path $noVncRoot $policy.Version) `
    -ManifestPath (Join-Path $noVncRoot "manifest.sha256")
Assert-NoVncPackage `
    -PackageRoot (Join-Path $noVncRoot $policy.Version) `
    -MetadataPath (Join-Path $noVncRoot "npm-metadata.json") `
    -AttestationsPath (Join-Path $noVncRoot "npm-attestations.json")
Assert-BrowserPackageLock `
    -PackageJsonPath (Join-Path $repositoryRoot "browser-tests/package.json") `
    -PackageLockPath (Join-Path $repositoryRoot "browser-tests/package-lock.json")

Write-Host "Web assets and browser dependency lock passed."
$global:LASTEXITCODE = 0
