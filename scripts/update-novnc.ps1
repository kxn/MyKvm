[CmdletBinding()]
param(
    [switch]$Replace
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Import-Module (Join-Path $PSScriptRoot "web-assets-tools.psm1") -Force

function Invoke-CheckedNativeCommand {
    param(
        [Parameter(Mandatory)]
        [string]$Executable,

        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = @(& $Executable @Arguments)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($exitCode -ne 0) {
        throw "$Executable failed with exit code $exitCode"
    }
    return $output
}

function Assert-Archive {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [psobject]$Policy
    )

    $file = Get-Item -LiteralPath $Path
    if ($file.Length -ne $Policy.ArchiveSize) {
        throw (
            "Unexpected noVNC archive size: expected " +
            "$($Policy.ArchiveSize), got $($file.Length)"
        )
    }
    $sha256 = (Get-FileHash `
        -LiteralPath $Path `
        -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($sha256 -ne $Policy.ArchiveSha256) {
        throw "Unexpected noVNC archive SHA-256: $sha256"
    }
    $sha512 = (Get-FileHash `
        -LiteralPath $Path `
        -Algorithm SHA512).Hash.ToLowerInvariant()
    if ($sha512 -ne $Policy.ArchiveSha512) {
        throw "Unexpected noVNC archive SHA-512: $sha512"
    }
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$noVncRoot = Join-Path $repositoryRoot "third_party/novnc"
$policy = Get-NoVncReleasePolicy
$target = Join-Path $noVncRoot $policy.Version
Assert-SafeRepositoryTarget `
    -Path $target `
    -RepositoryRoot $repositoryRoot `
    -AllowedRelativeRoot "third_party/novnc"

$temporaryRoot = [System.IO.Path]::GetFullPath(
    (Join-Path `
        ([System.IO.Path]::GetTempPath()) `
        ("my-ipkvm-update-novnc-" + [guid]::NewGuid()))
)
Assert-SafeTemporaryPath -Path $temporaryRoot
$null = New-Item -ItemType Directory -Path $temporaryRoot

try {
    $archivePath = Join-Path $temporaryRoot "novnc-1.7.0.tgz"
    $metadataPath = Join-Path $temporaryRoot "npm-metadata.json"
    $attestationsPath = Join-Path $temporaryRoot "npm-attestations.json"
    Invoke-WebRequest `
        -UseBasicParsing `
        -Uri $policy.Tarball `
        -OutFile $archivePath
    Invoke-WebRequest `
        -UseBasicParsing `
        -Uri $policy.MetadataUrl `
        -OutFile $metadataPath
    Invoke-WebRequest `
        -UseBasicParsing `
        -Uri $policy.AttestationsUrl `
        -OutFile $attestationsPath

    Assert-Archive -Path $archivePath -Policy $policy
    $names = @(
        Invoke-CheckedNativeCommand `
            -Executable "tar.exe" `
            -Arguments @("-tf", $archivePath)
    )
    $verboseLines = @(
        Invoke-CheckedNativeCommand `
            -Executable "tar.exe" `
            -Arguments @("-tvf", $archivePath)
    )
    Assert-SafeTarEntries -Names $names -VerboseLines $verboseLines

    $extractRoot = Join-Path $temporaryRoot "extracted"
    $null = New-Item -ItemType Directory -Path $extractRoot
    $null = Invoke-CheckedNativeCommand `
        -Executable "tar.exe" `
        -Arguments @("-xf", $archivePath, "-C", $extractRoot)
    Assert-SafeExtractedTree -Root $extractRoot
    $packageRoot = Join-Path $extractRoot "package"
    Assert-NoVncPackage `
        -PackageRoot $packageRoot `
        -MetadataPath $metadataPath `
        -AttestationsPath $attestationsPath

    if (Test-Path -LiteralPath $target) {
        if (-not $Replace) {
            throw (
                "noVNC target already exists; rerun with -Replace after review: " +
                $target
            )
        }
        Assert-SafeRepositoryTarget `
            -Path $target `
            -RepositoryRoot $repositoryRoot `
            -AllowedRelativeRoot "third_party/novnc"
        Remove-Item -LiteralPath $target -Recurse -Force
    }

    $null = New-Item -ItemType Directory -Path $noVncRoot -Force
    Copy-Item -LiteralPath $packageRoot -Destination $target -Recurse
    Copy-Item `
        -LiteralPath $metadataPath `
        -Destination (Join-Path $noVncRoot "npm-metadata.json")
    Copy-Item `
        -LiteralPath $attestationsPath `
        -Destination (Join-Path $noVncRoot "npm-attestations.json")
    Write-WebAssetManifest `
        -Root $target `
        -Path (Join-Path $noVncRoot "manifest.sha256")

    Assert-WebAssetTree `
        -Root $target `
        -ManifestPath (Join-Path $noVncRoot "manifest.sha256")
    Assert-NoVncPackage `
        -PackageRoot $target `
        -MetadataPath (Join-Path $noVncRoot "npm-metadata.json") `
        -AttestationsPath (Join-Path $noVncRoot "npm-attestations.json")
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        Assert-SafeTemporaryPath -Path $temporaryRoot
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}

Write-Host "noVNC $($policy.Version) assets updated."
$global:LASTEXITCODE = 0
