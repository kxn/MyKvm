# 本机快速门禁（PowerShell 版本，对应 verify.sh）：静态检查，无全量编译，环境无关。
# 提交前运行本脚本；合并前请运行 verify-full.ps1 完成全量门禁（test/clippy/doc）。
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

function Test-TrackedTextEncoding {
    $strictUtf8 = [System.Text.UTF8Encoding]::new($false, $true)
    $trackedFiles = @(
        & git ls-files -- "*.css" "*.html" "*.js" "*.json" "*.md" "*.mjs" "*.ps1" "*.psm1" "*.py" "*.rs" "*.sha256" "*.sh" "*.toml" "*.yaml" "*.yml" "AGENTS.md" "Cargo.lock"
    )
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to list tracked text files, exit code: $LASTEXITCODE"
    }

    foreach ($relativePath in $trackedFiles) {
        $path = Join-Path (Get-Location) $relativePath
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            continue
        }

        $bytes = [System.IO.File]::ReadAllBytes($path)
        try {
            $null = $strictUtf8.GetString($bytes)
        }
        catch {
            throw "$relativePath is not valid UTF-8: $($_.Exception.Message)"
        }

        if (
            $bytes.Length -ge 3 -and
            $bytes[0] -eq 0xEF -and
            $bytes[1] -eq 0xBB -and
            $bytes[2] -eq 0xBF
        ) {
            throw "$relativePath contains a UTF-8 BOM"
        }
    }
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $repositoryRoot
try {
    Write-Host "==> Check text encoding"
    Test-TrackedTextEncoding

    Invoke-CheckedCommand "Test web asset policy" {
        & (Join-Path $PSScriptRoot "test-web-assets.ps1")
    }
    Invoke-CheckedCommand "Check web assets and browser dependency lock" {
        & (Join-Path $PSScriptRoot "verify-web-assets.ps1")
    }
    Invoke-CheckedCommand "Test dependency license policy" {
        & (Join-Path $PSScriptRoot "test-license-policy.ps1")
    }
    Invoke-CheckedCommand "Check dependency licenses and sources" {
        & (Join-Path $PSScriptRoot "verify-licenses.ps1")
    }
    Invoke-CheckedCommand "Check iced M5 desktop retirement" {
        & (Join-Path $PSScriptRoot "test-iced-m5-retirement.ps1")
    }
    Invoke-CheckedCommand "Check crate dependency boundaries" {
        & (Join-Path $PSScriptRoot "test-crate-boundaries.ps1")
    }
    Invoke-CheckedCommand "Check Rust formatting" {
        cargo fmt --all --check
    }
    Invoke-CheckedCommand "Check working tree diff" {
        git diff --check
    }
    Invoke-CheckedCommand "Check staged diff" {
        git diff --cached --check
    }
}
finally {
    Pop-Location
}

Write-Host "Quick verification passed."
