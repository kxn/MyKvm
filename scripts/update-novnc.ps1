[CmdletBinding()]
param(
    [switch]$Replace
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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

# 调用跨平台 Python 校验工具 web_assets_tools.py（#9 阶段 B1 起替代 web-assets-tools.psm1）。
function Invoke-PyTool {
    param(
        [Parameter(Mandatory)]
        [string[]]$Arguments
    )

    # Python 解释器解析（既定模式优先 python3，Windows 下回退到 py 启动器）。
    $python = if (Get-Command python3 -ErrorAction SilentlyContinue) {
        "python3"
    }
    elseif (Get-Command py -ErrorAction SilentlyContinue) {
        "py"
    }
    else {
        throw "未找到 python3，请先安装 Python 3"
    }

    $toolPath = Join-Path $PSScriptRoot "web_assets_tools.py"
    return Invoke-CheckedNativeCommand `
        -Executable $python `
        -Arguments (@($toolPath) + $Arguments)
}

function ConvertFrom-PolicyJson {
    param(
        [Parameter(Mandatory)]
        [string[]]$Lines
    )

    $json = $Lines -join [Environment]::NewLine
    return $json | ConvertFrom-Json
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
$policy = ConvertFrom-PolicyJson (Invoke-PyTool @("policy"))
$target = Join-Path $noVncRoot $policy.Version
$novncApprovedRoot = Join-Path $repositoryRoot "third_party/novnc"
$null = Invoke-PyTool @(
    "check-path-under-root",
    "--path", $target,
    "--root", $novncApprovedRoot,
    "--message", "Path is outside the approved repository target"
)

$temporaryRoot = [System.IO.Path]::GetFullPath(
    (Join-Path `
        ([System.IO.Path]::GetTempPath()) `
        ("my-ipkvm-update-novnc-" + [guid]::NewGuid()))
)
$systemTemp = [System.IO.Path]::GetTempPath()
$null = Invoke-PyTool @(
    "check-path-under-root",
    "--path", $temporaryRoot,
    "--root", $systemTemp,
    "--message", "Path is outside the system temporary directory"
)
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
    $namesFile = Join-Path $temporaryRoot "tar-names.txt"
    $verboseFile = Join-Path $temporaryRoot "tar-verbose.txt"
    [System.IO.File]::WriteAllLines($namesFile, $names)
    [System.IO.File]::WriteAllLines($verboseFile, $verboseLines)
    $null = Invoke-PyTool @(
        "check-tar-entries",
        "--names", $namesFile,
        "--verbose", $verboseFile
    )

    $extractRoot = Join-Path $temporaryRoot "extracted"
    $null = New-Item -ItemType Directory -Path $extractRoot
    $null = Invoke-CheckedNativeCommand `
        -Executable "tar.exe" `
        -Arguments @("-xf", $archivePath, "-C", $extractRoot)
    $null = Invoke-PyTool @("check-extracted-tree", "--root", $extractRoot)
    $packageRoot = Join-Path $extractRoot "package"
    $null = Invoke-PyTool @(
        "check-novnc-package",
        "--package-root", $packageRoot,
        "--metadata", $metadataPath,
        "--attestations", $attestationsPath
    )

    if (Test-Path -LiteralPath $target) {
        if (-not $Replace) {
            throw (
                "noVNC target already exists; rerun with -Replace after review: " +
                $target
            )
        }
        $null = Invoke-PyTool @(
            "check-path-under-root",
            "--path", $target,
            "--root", $novncApprovedRoot,
            "--message", "Path is outside the approved repository target"
        )
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
    $manifestPath = Join-Path $noVncRoot "manifest.sha256"
    $null = Invoke-PyTool @(
        "write-manifest",
        "--root", $target,
        "--path", $manifestPath
    )
    $null = Invoke-PyTool @(
        "check-tree",
        "--root", $target,
        "--manifest", $manifestPath
    )
    $null = Invoke-PyTool @(
        "check-novnc-package",
        "--package-root", $target,
        "--metadata", (Join-Path $noVncRoot "npm-metadata.json"),
        "--attestations", (Join-Path $noVncRoot "npm-attestations.json")
    )
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        $null = Invoke-PyTool @(
            "check-path-under-root",
            "--path", $temporaryRoot,
            "--root", $systemTemp,
            "--message", "Path is outside the system temporary directory"
        )
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}

Write-Host "noVNC $($policy.Version) assets updated."
$global:LASTEXITCODE = 0
