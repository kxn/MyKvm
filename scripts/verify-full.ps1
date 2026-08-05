# 本机全量门禁（PowerShell 版本，对应 verify-full.sh）：快速门禁 + 全量编译检查。
# 合并前运行本脚本；仅开发迭代时可用 verify.ps1 快速门禁替代。
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

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $repositoryRoot
try {
    Invoke-CheckedCommand "Run quick verification gate" {
        & (Join-Path $PSScriptRoot "verify.ps1")
    }
    Invoke-CheckedCommand "Run workspace tests" {
        cargo test --workspace --all-features
    }
    Invoke-CheckedCommand "Run Clippy" {
        cargo clippy --workspace --all-targets --all-features -- -D warnings
    }

    $hadRustdocFlags = Test-Path Env:RUSTDOCFLAGS
    $previousRustdocFlags = $env:RUSTDOCFLAGS
    try {
        $env:RUSTDOCFLAGS = "-D warnings"
        Invoke-CheckedCommand "Build Rust documentation" {
            cargo doc --workspace --all-features --no-deps
        }
    }
    finally {
        if ($hadRustdocFlags) {
            $env:RUSTDOCFLAGS = $previousRustdocFlags
        }
        else {
            Remove-Item Env:RUSTDOCFLAGS -ErrorAction SilentlyContinue
        }
    }
}
finally {
    Pop-Location
}

Write-Host "Full verification passed."
