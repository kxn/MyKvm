[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Import-Module (Join-Path $PSScriptRoot "license-policy-tools.psm1") -Force

function ConvertFrom-JsonString {
    param(
        [Parameter(Mandatory)]
        [string]$Value
    )

    return ConvertFrom-Json ('"' + $Value + '"')
}

function Assert-ThrowsLike {
    param(
        [Parameter(Mandatory)]
        [scriptblock]$Command,

        [Parameter(Mandatory)]
        [string]$Pattern
    )

    try {
        & $Command
    }
    catch {
        if ($_.Exception.Message -notmatch $Pattern) {
            throw "Exception did not match '$Pattern': $($_.Exception.Message)"
        }
        return
    }

    throw "Command succeeded but failure was expected"
}

function Set-Utf8File {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Content
    )

    $parent = Split-Path -Parent $Path
    $null = New-Item -ItemType Directory -Path $parent -Force
    [System.IO.File]::WriteAllText(
        $Path,
        $Content,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory)]
        [string]$Executable,

        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = @(& $Executable @Arguments 2>&1 | ForEach-Object { "$_" })
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    return [pscustomobject]@{
        ExitCode = $exitCode
        Output = $output -join [Environment]::NewLine
    }
}

function Assert-CommandSucceeded {
    param(
        [Parameter(Mandatory)]
        [psobject]$Result,

        [Parameter(Mandatory)]
        [string]$Name
    )

    if ($Result.ExitCode -ne 0) {
        throw "$Name failed with exit code $($Result.ExitCode): $($Result.Output)"
    }
}

function Assert-CommandRejected {
    param(
        [Parameter(Mandatory)]
        [psobject]$Result,

        [Parameter(Mandatory)]
        [int]$ExitCode,

        [Parameter(Mandatory)]
        [string[]]$Patterns,

        [Parameter(Mandatory)]
        [string]$Name
    )

    if ($Result.ExitCode -ne $ExitCode) {
        throw (
            "$Name returned $($Result.ExitCode), expected $ExitCode`: " +
            $Result.Output
        )
    }

    foreach ($pattern in $Patterns) {
        if ($Result.Output -notmatch $pattern) {
            throw "$Name output did not match '$pattern': $($Result.Output)"
        }
    }
}

function Get-ValidatedFixtureRoot {
    $tempBase = [System.IO.Path]::GetFullPath(
        [System.IO.Path]::GetTempPath()
    )
    $root = [System.IO.Path]::GetFullPath(
        (Join-Path $tempBase ("my-ipkvm-license-policy-" + [guid]::NewGuid()))
    )

    $baseWithSeparator = $tempBase.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $root.StartsWith(
        $baseWithSeparator,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "Fixture root is outside the system temporary directory: $root"
    }

    return $root
}

function Assert-SafeFixtureRoot {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $tempBase = [System.IO.Path]::GetFullPath(
        [System.IO.Path]::GetTempPath()
    )
    $baseWithSeparator = $tempBase.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if (-not $fullPath.StartsWith(
        $baseWithSeparator,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "Refusing to remove fixture outside temporary directory: $fullPath"
    }
}

function New-PathDependencyFixture {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$DependencyLicense
    )

    $manifest = @"
[package]
name = "policy-fixture-app"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false

[dependencies]
policy-fixture-dependency = { path = "dependency" }

[workspace]
members = ["dependency"]
"@
    $dependencyManifest = @"
[package]
name = "policy-fixture-dependency"
version = "0.1.0"
edition = "2024"
license = "$DependencyLicense"
publish = false
"@

    Set-Utf8File -Path (Join-Path $Root "Cargo.toml") -Content $manifest
    Set-Utf8File -Path (Join-Path $Root "src/lib.rs") -Content "pub fn app() {}"
    Set-Utf8File `
        -Path (Join-Path $Root "dependency/Cargo.toml") `
        -Content $dependencyManifest
    Set-Utf8File `
        -Path (Join-Path $Root "dependency/src/lib.rs") `
        -Content "pub fn dependency() {}"
}

function New-GitDependencyFixture {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    $dependencyRoot = Join-Path $Root "git-dependency"
    $consumerRoot = Join-Path $Root "git-consumer"
    $dependencyManifest = @"
[package]
name = "policy-git-dependency"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false
"@
    Set-Utf8File `
        -Path (Join-Path $dependencyRoot "Cargo.toml") `
        -Content $dependencyManifest
    Set-Utf8File `
        -Path (Join-Path $dependencyRoot "src/lib.rs") `
        -Content "pub fn dependency() {}"

    Assert-CommandSucceeded `
        -Result (Invoke-NativeCommand "git" @("-C", $dependencyRoot, "init")) `
        -Name "Initialize fixture Git repository"
    Assert-CommandSucceeded `
        -Result (Invoke-NativeCommand "git" @(
            "-C", $dependencyRoot, "config", "user.name",
            "my_ipkvm policy test"
        )) `
        -Name "Configure fixture Git user name"
    Assert-CommandSucceeded `
        -Result (Invoke-NativeCommand "git" @(
            "-C", $dependencyRoot, "config", "user.email",
            "policy-test@invalid.local"
        )) `
        -Name "Configure fixture Git user email"
    Assert-CommandSucceeded `
        -Result (Invoke-NativeCommand "git" @("-C", $dependencyRoot, "add", ".")) `
        -Name "Stage fixture Git repository"
    Assert-CommandSucceeded `
        -Result (Invoke-NativeCommand "git" @(
            "-C", $dependencyRoot, "commit", "-m", "fixture"
        )) `
        -Name "Commit fixture Git repository"

    $dependencyUri = [System.Uri]::new(
        [System.IO.Path]::GetFullPath($dependencyRoot)
    ).AbsoluteUri
    $consumerManifest = @"
[package]
name = "policy-git-consumer"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false

[dependencies]
policy-git-dependency = { git = "$dependencyUri" }

[workspace]
"@
    Set-Utf8File `
        -Path (Join-Path $consumerRoot "Cargo.toml") `
        -Content $consumerManifest
    Set-Utf8File `
        -Path (Join-Path $consumerRoot "src/lib.rs") `
        -Content "pub fn consumer() {}"

    return $consumerRoot
}

if ((Get-RequiredCargoDenyVersion) -ne "0.20.2") {
    throw "Required version is not 0.20.2"
}

if ((Assert-CargoDenyVersion -VersionOutput "cargo-deny 0.20.2") -ne "0.20.2") {
    throw "Expected version was rejected"
}

$expected = ConvertFrom-JsonString "\u671f\u671b"
$actual = ConvertFrom-JsonString "\u5b9e\u9645"
Assert-ThrowsLike {
    Assert-CargoDenyVersion -VersionOutput "cargo-deny 0.20.1"
} "$expected 0\.20\.2.*$actual 0\.20\.1"

$cannotParse = ConvertFrom-JsonString "\u65e0\u6cd5\u89e3\u6790 cargo-deny \u7248\u672c"
Assert-ThrowsLike {
    Assert-CargoDenyVersion -VersionOutput "not a version"
} $cannotParse

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$denyConfig = Join-Path $repositoryRoot "deny.toml"
if (-not (Test-Path -LiteralPath $denyConfig -PathType Leaf)) {
    throw "deny.toml was not found at repository root"
}

$cargoDeny = Get-CargoDenyExecutable
$fixtureRoot = Get-ValidatedFixtureRoot
$null = New-Item -ItemType Directory -Path $fixtureRoot

try {
    $pathFixture = Join-Path $fixtureRoot "path-dependency"
    New-PathDependencyFixture `
        -Root $pathFixture `
        -DependencyLicense "BSD-3-Clause"
    $lockResult = Invoke-NativeCommand "cargo" @(
        "generate-lockfile",
        "--manifest-path", (Join-Path $pathFixture "Cargo.toml"),
        "--offline"
    )
    Assert-CommandSucceeded $lockResult "Generate allowed fixture lock file"

    $allowed = Invoke-NativeCommand $cargoDeny @(
        "--config", $denyConfig,
        "--manifest-path", (Join-Path $pathFixture "Cargo.toml"),
        "--locked",
        "check", "licenses", "sources"
    )
    Assert-CommandSucceeded $allowed "Allowed license fixture"

    New-PathDependencyFixture `
        -Root $pathFixture `
        -DependencyLicense "GPL-3.0-only"
    $rejectedLicense = Invoke-NativeCommand $cargoDeny @(
        "--config", $denyConfig,
        "--manifest-path", (Join-Path $pathFixture "Cargo.toml"),
        "--locked",
        "check", "licenses", "sources"
    )
    Assert-CommandRejected `
        -Result $rejectedLicense `
        -ExitCode 4 `
        -Patterns @("rejected", "GPL-3\.0-only") `
        -Name "Rejected license fixture"

    $gitConsumer = New-GitDependencyFixture -Root $fixtureRoot
    $gitLock = Invoke-NativeCommand "cargo" @(
        "generate-lockfile",
        "--manifest-path", (Join-Path $gitConsumer "Cargo.toml")
    )
    Assert-CommandSucceeded $gitLock "Generate Git fixture lock file"

    $rejectedGit = Invoke-NativeCommand $cargoDeny @(
        "--config", $denyConfig,
        "--manifest-path", (Join-Path $gitConsumer "Cargo.toml"),
        "--locked",
        "check", "sources"
    )
    Assert-CommandRejected `
        -Result $rejectedGit `
        -ExitCode 8 `
        -Patterns @(
            "source-not-allowed",
            "git-source-underspecified",
            "file://"
        ) `
        -Name "Rejected Git source fixture"
}
finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Assert-SafeFixtureRoot -Path $fixtureRoot
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}

Write-Host "Dependency license policy tests passed."
