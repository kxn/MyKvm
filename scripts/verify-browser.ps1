[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [scriptblock]$Command
    )

    Write-Host "==> $Name"
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE"
    }
}

function Get-FixtureExecutable {
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $lines = @(
            & cargo build `
                -p ipkvm-browser-fixture `
                --bin ipkvm-browser-fixture `
                --message-format=json-render-diagnostics
        )
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($exitCode -ne 0) {
        throw "Build browser fixture failed with exit code $exitCode"
    }

    $executables = @(
        foreach ($line in $lines) {
            try {
                $message = ConvertFrom-Json $line
            }
            catch {
                continue
            }
            if (
                $message.reason -eq "compiler-artifact" -and
                $message.target.name -eq "ipkvm-browser-fixture" -and
                -not [string]::IsNullOrWhiteSpace($message.executable)
            ) {
                [System.IO.Path]::GetFullPath($message.executable)
            }
        }
    )
    $executables = @($executables | Select-Object -Unique)
    if ($executables.Count -ne 1) {
        throw (
            "Expected one browser fixture executable, got " +
            $executables.Count
        )
    }
    if (-not (Test-Path -LiteralPath $executables[0] -PathType Leaf)) {
        throw "Browser fixture executable does not exist: $($executables[0])"
    }
    return $executables[0]
}

function Assert-FixtureFeatureBoundary {
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $json = & cargo metadata --format-version 1 --no-deps
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($exitCode -ne 0) {
        throw "Cargo metadata failed with exit code $exitCode"
    }
    $metadata = $json | ConvertFrom-Json
    $fixturePackage = @(
        $metadata.packages |
            Where-Object { $_.name -eq "ipkvm-browser-fixture" }
    )
    if ($fixturePackage.Count -ne 1) {
        throw "Expected one ipkvm-browser-fixture package in Cargo metadata"
    }
    $fixture = @(
        $fixturePackage[0].targets |
            Where-Object { $_.name -eq "ipkvm-browser-fixture" }
    )
    if ($fixture.Count -ne 1) {
        throw "Expected one ipkvm-browser-fixture target"
    }
    $requiredFeatures = @($fixture[0]."required-features")
    if ($requiredFeatures.Count -ne 0) {
        throw "Browser fixture must not require a package feature"
    }
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$browserTestRoot = Join-Path $repositoryRoot "browser-tests"
$hadFixturePath = Test-Path Env:IPKVM_BROWSER_FIXTURE
$previousFixturePath = $env:IPKVM_BROWSER_FIXTURE

Push-Location $repositoryRoot
try {
    $nodeVersion = & node -p "process.versions.node"
    if ($LASTEXITCODE -ne 0) {
        throw "Node.js is required for browser verification"
    }
    $nodeMajor = [int]($nodeVersion.Split(".")[0])
    if ($nodeMajor -lt 20) {
        throw "Node.js 20 or newer is required, got $nodeVersion"
    }

    Invoke-CheckedCommand "Install locked browser test dependency" {
        npm ci --ignore-scripts --prefix $browserTestRoot
    }
    Assert-FixtureFeatureBoundary
    $env:IPKVM_BROWSER_FIXTURE = Get-FixtureExecutable
    Invoke-CheckedCommand "Run real browser verification" {
        node (Join-Path $browserTestRoot "novnc-browser.mjs")
    }
}
finally {
    if ($hadFixturePath) {
        $env:IPKVM_BROWSER_FIXTURE = $previousFixturePath
    }
    else {
        Remove-Item Env:IPKVM_BROWSER_FIXTURE -ErrorAction SilentlyContinue
    }
    Pop-Location
}

Write-Host "Real browser verification passed."
$global:LASTEXITCODE = 0
