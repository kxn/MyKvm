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

Write-Host "cargo-deny tool version contract passed."
