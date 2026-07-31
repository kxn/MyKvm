[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Import-Module (Join-Path $PSScriptRoot "license-policy-tools.psm1") -Force

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoDeny = Get-CargoDenyExecutable

Push-Location $repositoryRoot
try {
    & $cargoDeny --locked check licenses sources
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "Dependency license or source check failed with exit code $exitCode"
    }
}
finally {
    Pop-Location
}

Write-Host "Dependency licenses and sources passed."
$global:LASTEXITCODE = 0
