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
        & git ls-files -- "*.json" "*.md" "*.ps1" "*.rs" "*.toml" "*.yaml" "*.yml" "AGENTS.md" "Cargo.lock"
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
$hadRustdocFlags = Test-Path Env:RUSTDOCFLAGS
$previousRustdocFlags = $env:RUSTDOCFLAGS

Push-Location $repositoryRoot
try {
    Write-Host "==> Check text encoding"
    Test-TrackedTextEncoding

    Invoke-CheckedCommand "Check Rust formatting" {
        cargo fmt --all --check
    }
    Invoke-CheckedCommand "Run workspace tests" {
        cargo test --workspace --all-features
    }
    Invoke-CheckedCommand "Run Clippy" {
        cargo clippy --workspace --all-targets --all-features -- -D warnings
    }

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

Write-Host "Local verification passed."
