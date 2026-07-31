Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RequiredCargoDenyVersion = "0.20.2"
$script:CargoDenyInstallCommand =
    "cargo install --locked --version 0.20.2 cargo-deny"

function ConvertFrom-PolicyJsonString {
    param(
        [Parameter(Mandatory)]
        [string]$Value
    )

    return ConvertFrom-Json ('"' + $Value + '"')
}

function Get-RequiredCargoDenyVersion {
    return $script:RequiredCargoDenyVersion
}

function Assert-CargoDenyVersion {
    param(
        [Parameter(Mandatory)]
        [string]$VersionOutput
    )

    $match = [regex]::Match(
        $VersionOutput.Trim(),
        "^cargo-deny\s+([0-9]+\.[0-9]+\.[0-9]+)(?:\s.*)?$"
    )
    if (-not $match.Success) {
        $message = ConvertFrom-PolicyJsonString (
            "\u65e0\u6cd5\u89e3\u6790 cargo-deny \u7248\u672c\u3002" +
            "\u8bf7\u6267\u884c\uff1a$script:CargoDenyInstallCommand"
        )
        throw $message
    }

    $actual = $match.Groups[1].Value
    if ($actual -ne $script:RequiredCargoDenyVersion) {
        $message = ConvertFrom-PolicyJsonString (
            "cargo-deny \u7248\u672c\u4e0d\u7b26\uff1a" +
            "\u671f\u671b $script:RequiredCargoDenyVersion\uff0c" +
            "\u5b9e\u9645 $actual\u3002" +
            "\u8bf7\u6267\u884c\uff1a$script:CargoDenyInstallCommand"
        )
        throw $message
    }

    return $actual
}

function Get-CargoDenyExecutable {
    $command = Get-Command "cargo-deny" -CommandType Application `
        -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        $message = ConvertFrom-PolicyJsonString (
            "\u672a\u627e\u5230 cargo-deny\u3002" +
            "\u8bf7\u6267\u884c\uff1a$script:CargoDenyInstallCommand"
        )
        throw $message
    }

    $output = & $command.Source --version
    if ($LASTEXITCODE -ne 0) {
        $message = ConvertFrom-PolicyJsonString (
            "cargo-deny --version \u6267\u884c\u5931\u8d25\uff0c" +
            "\u9000\u51fa\u7801\uff1a$LASTEXITCODE"
        )
        throw $message
    }

    $null = Assert-CargoDenyVersion -VersionOutput ($output -join "`n")
    return $command.Source
}

Export-ModuleMember -Function @(
    "Get-RequiredCargoDenyVersion",
    "Assert-CargoDenyVersion",
    "Get-CargoDenyExecutable"
)
