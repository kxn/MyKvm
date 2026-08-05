Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:NoVncVersion = "1.7.0"
$script:NoVncCommit = "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e"
$script:NoVncTarball = (
    "https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz"
)
$script:NoVncIntegrity = (
    "sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/" +
    "yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA=="
)
$script:NoVncShasum = "7f832cf07c66475a81a25708b8e5299a5c4efec5"
$script:NoVncArchiveSize = 155185
$script:NoVncArchiveSha256 = (
    "32689f18d6abe96bc6530828a6bd0b9ae33bda07c083a6575ed255b5a8f2e903"
)
$script:NoVncArchiveSha512 = (
    "b9c1093b1e13d9abc844295ea1d93b6286d98f93b619ad8078b7d0ebd03fca31" +
    "bd1c76dadbe3c38304a437716db6b986fbbd191b1db5b0494d168b8ad77473c8"
)
$script:NoVncMetadataUrl = (
    "https://registry.npmjs.org/@novnc%2Fnovnc/1.7.0"
)
$script:NoVncAttestationsUrl = (
    "https://registry.npmjs.org/-/npm/v1/attestations/" +
    "@novnc%2Fnovnc@1.7.0"
)
$script:PlaywrightVersion = "1.62.1"
$script:PlaywrightResolved = (
    "https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz"
)
$script:PlaywrightIntegrity = (
    "sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS" +
    "7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw=="
)

function ConvertFrom-JsonToHashtable {
    # Parse JSON text into a Hashtable, compatible with PowerShell 5.1 and 7+.
    # Callers can then use [] indexing, Keys and ContainsKey (needed for package-lock
    # entries whose key is the empty string, e.g. the lockfile root).
    # - PS7+: ConvertFrom-Json -AsHashtable returns a Hashtable directly and handles
    #   empty keys. This is what GitHub Actions runners (pwsh 7) use.
    # - PS5.1: ConvertFrom-Json cannot represent empty-string keys (throws), so fall
    #   back to System.Web.Script.Serialization.JavaScriptSerializer, which is
    #   available on .NET Framework / Windows PowerShell and returns a Hashtable.
    #   System.Web is NOT available on PS7/.NET Core, which is exactly why the PS7
    #   branch avoids it.
    param(
        [Parameter(Mandatory)]
        [string]$Json
    )

    if ($PSVersionTable.PSVersion.Major -ge 6) {
        return $Json | ConvertFrom-Json -AsHashtable
    }

    Add-Type -AssemblyName System.Web.Extensions
    $serializer = [System.Web.Script.Serialization.JavaScriptSerializer]::new()
    $serializer.MaxJsonLength = [int]::MaxValue
    return $serializer.DeserializeObject($Json)
}

function Get-PathPrefix {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    return $Path.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
}

function Assert-PathUnderRoot {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Message
    )

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $fullRoot = [System.IO.Path]::GetFullPath($Root)
    if (
        $fullPath -eq $fullRoot -or
        -not $fullPath.StartsWith(
            (Get-PathPrefix $fullRoot),
            [System.StringComparison]::OrdinalIgnoreCase
        )
    ) {
        throw "$Message`: $fullPath"
    }

    return $fullPath
}

function Assert-SafeTemporaryPath {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $temporaryRoot = [System.IO.Path]::GetFullPath(
        [System.IO.Path]::GetTempPath()
    )
    Assert-PathUnderRoot `
        -Path $Path `
        -Root $temporaryRoot `
        -Message "Path is outside the system temporary directory" |
        Out-Null
}

function Assert-SafeRepositoryTarget {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$RepositoryRoot,

        [Parameter(Mandatory)]
        [string]$AllowedRelativeRoot
    )

    $allowedRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $RepositoryRoot $AllowedRelativeRoot)
    )
    Assert-PathUnderRoot `
        -Path $Path `
        -Root $allowedRoot `
        -Message "Path is outside the approved repository target" |
        Out-Null
}

function Get-NoVncReleasePolicy {
    return [pscustomobject]@{
        Version = $script:NoVncVersion
        Commit = $script:NoVncCommit
        Tarball = $script:NoVncTarball
        Integrity = $script:NoVncIntegrity
        Shasum = $script:NoVncShasum
        ArchiveSize = $script:NoVncArchiveSize
        ArchiveSha256 = $script:NoVncArchiveSha256
        ArchiveSha512 = $script:NoVncArchiveSha512
        MetadataUrl = $script:NoVncMetadataUrl
        AttestationsUrl = $script:NoVncAttestationsUrl
    }
}

function Get-AssetRelativePath {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Path
    )

    $fullRoot = [System.IO.Path]::GetFullPath($Root)
    $fullPath = Assert-PathUnderRoot `
        -Path $Path `
        -Root $fullRoot `
        -Message "Asset is outside its root"
    return $fullPath.Substring((Get-PathPrefix $fullRoot).Length).Replace(
        [System.IO.Path]::DirectorySeparatorChar,
        [char]"/"
    )
}

function Assert-SafeManifestPath {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    if (
        [string]::IsNullOrWhiteSpace($Path) -or
        $Path.Contains("\") -or
        $Path.Contains([char]0) -or
        $Path.StartsWith("/") -or
        $Path -match "^[A-Za-z]:"
    ) {
        throw "Unsafe manifest path: $Path"
    }

    $segments = $Path.Split("/")
    if ($segments.Count -eq 0) {
        throw "Unsafe manifest path: $Path"
    }
    foreach ($segment in $segments) {
        if (
            [string]::IsNullOrEmpty($segment) -or
            $segment -eq "." -or
            $segment -eq ".."
        ) {
            throw "Unsafe manifest path: $Path"
        }
    }
}

function Write-WebAssetManifest {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Path
    )

    $fullRoot = (Resolve-Path -LiteralPath $Root).Path
    $lines = @(
        Get-ChildItem -LiteralPath $fullRoot -File -Recurse |
            ForEach-Object {
                $relativePath = Get-AssetRelativePath `
                    -Root $fullRoot `
                    -Path $_.FullName
                Assert-SafeManifestPath -Path $relativePath
                [pscustomobject]@{
                    Path = $relativePath
                    Hash = (Get-FileHash `
                        -LiteralPath $_.FullName `
                        -Algorithm SHA256).Hash.ToLowerInvariant()
                }
            } |
            Sort-Object -Property Path |
            ForEach-Object { "$($_.Hash)  $($_.Path)" }
    )
    $content = if ($lines.Count -eq 0) {
        ""
    }
    else {
        ($lines -join "`n") + "`n"
    }
    $parent = Split-Path -Parent $Path
    $null = New-Item -ItemType Directory -Path $parent -Force
    [System.IO.File]::WriteAllText(
        $Path,
        $content,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Read-WebAssetManifest {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $entries = [System.Collections.Generic.Dictionary[string, string]]::new(
        [System.StringComparer]::Ordinal
    )
    $lineNumber = 0
    foreach ($line in Get-Content -LiteralPath $Path -Encoding utf8) {
        $lineNumber += 1
        if ($line -notmatch "^([0-9a-fA-F]{64})  (.+)$") {
            throw "Invalid manifest line $lineNumber"
        }
        $relativePath = $Matches[2]
        Assert-SafeManifestPath -Path $relativePath
        if ($entries.ContainsKey($relativePath)) {
            throw "Duplicate manifest path: $relativePath"
        }
        $entries.Add($relativePath, $Matches[1].ToLowerInvariant())
    }
    return $entries
}

function Assert-WebAssetTree {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$ManifestPath
    )

    $fullRoot = (Resolve-Path -LiteralPath $Root).Path
    $expected = Read-WebAssetManifest -Path $ManifestPath
    $actual = [System.Collections.Generic.Dictionary[string, string]]::new(
        [System.StringComparer]::Ordinal
    )
    foreach ($file in Get-ChildItem -LiteralPath $fullRoot -File -Recurse) {
        $relativePath = Get-AssetRelativePath `
            -Root $fullRoot `
            -Path $file.FullName
        if ($actual.ContainsKey($relativePath)) {
            throw "Duplicate web asset path: $relativePath"
        }
        $actual.Add(
            $relativePath,
            (Get-FileHash `
                -LiteralPath $file.FullName `
                -Algorithm SHA256).Hash.ToLowerInvariant()
        )
    }

    foreach ($relativePath in $expected.Keys) {
        if (-not $actual.ContainsKey($relativePath)) {
            throw "Web asset is missing: $relativePath"
        }
        if ($actual[$relativePath] -ne $expected[$relativePath]) {
            throw "Web asset hash mismatch: $relativePath"
        }
    }
    foreach ($relativePath in $actual.Keys) {
        if (-not $expected.ContainsKey($relativePath)) {
            throw "Unexpected web asset: $relativePath"
        }
    }
}

function Assert-JsonPropertyEquals {
    param(
        [Parameter(Mandatory)]
        [object]$Actual,

        [Parameter(Mandatory)]
        [object]$Expected,

        [Parameter(Mandatory)]
        [string]$Name
    )

    if ($Actual -ne $Expected) {
        throw "Unexpected $Name`: expected '$Expected', got '$Actual'"
    }
}

function Assert-NoVncPackage {
    param(
        [Parameter(Mandatory)]
        [string]$PackageRoot,

        [Parameter(Mandatory)]
        [string]$MetadataPath,

        [Parameter(Mandatory)]
        [string]$AttestationsPath
    )

    $requiredFiles = @(
        "AUTHORS",
        "LICENSE.txt",
        "docs/LICENSE.MPL-2.0",
        "vendor/pako/LICENSE",
        "core/crypto/des.js",
        "core/rfb.js",
        "package.json"
    )
    foreach ($relativePath in $requiredFiles) {
        $path = Join-Path $PackageRoot ($relativePath.Replace("/", "\"))
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Missing required noVNC file: $relativePath"
        }
    }

    $package = Get-Content `
        -Raw `
        -Encoding utf8 `
        -LiteralPath (Join-Path $PackageRoot "package.json") |
        ConvertFrom-Json
    Assert-JsonPropertyEquals $package.name "@novnc/novnc" "package name"
    Assert-JsonPropertyEquals `
        $package.version `
        $script:NoVncVersion `
        "package version"
    Assert-JsonPropertyEquals $package.license "MPL-2.0" "package license"
    if (@($package.dependencies.PSObject.Properties).Count -ne 0) {
        throw "noVNC package has unexpected runtime dependencies"
    }

    $metadata = Get-Content -Raw -Encoding utf8 -LiteralPath $MetadataPath |
        ConvertFrom-Json
    Assert-JsonPropertyEquals `
        $metadata.version `
        $script:NoVncVersion `
        "npm metadata version"
    Assert-JsonPropertyEquals `
        $metadata.gitHead `
        $script:NoVncCommit `
        "npm metadata gitHead"
    Assert-JsonPropertyEquals `
        $metadata.dist.tarball `
        $script:NoVncTarball `
        "npm metadata tarball"
    Assert-JsonPropertyEquals `
        $metadata.dist.integrity `
        $script:NoVncIntegrity `
        "npm metadata integrity"
    Assert-JsonPropertyEquals `
        $metadata.dist.shasum `
        $script:NoVncShasum `
        "npm metadata shasum"

    $attestations = Get-Content `
        -Raw `
        -Encoding utf8 `
        -LiteralPath $AttestationsPath |
        ConvertFrom-Json
    if (@($attestations.attestations).Count -eq 0) {
        throw "npm attestation reference is empty"
    }
    if (
        -not (
            @($attestations.attestations) |
                Where-Object {
                    $_.predicateType -eq "https://slsa.dev/provenance/v1"
                }
        )
    ) {
        throw "npm attestation reference lacks SLSA provenance"
    }
}

function Assert-BrowserPackageLock {
    param(
        [Parameter(Mandatory)]
        [string]$PackageJsonPath,

        [Parameter(Mandatory)]
        [string]$PackageLockPath
    )

    $package = Get-Content -Raw -Encoding utf8 -LiteralPath $PackageJsonPath |
        ConvertFrom-Json
    $dependencies = @($package.devDependencies.PSObject.Properties)
    if (
        $dependencies.Count -ne 1 -or
        $dependencies[0].Name -ne "playwright-core" -or
        $dependencies[0].Value -ne $script:PlaywrightVersion
    ) {
        throw "Browser package must pin only playwright-core 1.62.1"
    }

    $lock = ConvertFrom-JsonToHashtable `
        (Get-Content -Raw -Encoding utf8 -LiteralPath $PackageLockPath)
    Assert-JsonPropertyEquals `
        $lock["lockfileVersion"] `
        3 `
        "npm lockfile version"
    $packageEntries = $lock["packages"]
    $allowedEntries = @("", "node_modules/playwright-core")
    foreach ($entryName in $packageEntries.Keys) {
        if ($entryName -notin $allowedEntries) {
            throw "Browser lock contains unapproved package: $entryName"
        }
    }
    foreach ($allowedEntry in $allowedEntries) {
        if (-not $packageEntries.ContainsKey($allowedEntry)) {
            throw "Browser lock is missing package: $allowedEntry"
        }
    }

    $rootEntry = $packageEntries[""]
    Assert-JsonPropertyEquals `
        $rootEntry["devDependencies"]["playwright-core"] `
        $script:PlaywrightVersion `
        "root playwright-core version"

    $playwright = $packageEntries["node_modules/playwright-core"]
    Assert-JsonPropertyEquals `
        $playwright["version"] `
        $script:PlaywrightVersion `
        "playwright-core version"
    if ($playwright["resolved"] -ne $script:PlaywrightResolved) {
        throw (
            "Unexpected playwright-core registry source: " +
            $playwright["resolved"]
        )
    }
    Assert-JsonPropertyEquals `
        $playwright["integrity"] `
        $script:PlaywrightIntegrity `
        "playwright-core integrity"
    Assert-JsonPropertyEquals `
        $playwright["license"] `
        "Apache-2.0" `
        "playwright-core license"
}

function Assert-SafeTarEntries {
    param(
        [Parameter(Mandatory)]
        [string[]]$Names,

        [Parameter(Mandatory)]
        [string[]]$VerboseLines
    )

    if ($Names.Count -ne $VerboseLines.Count) {
        throw "Tar name and verbose entry counts differ"
    }

    for ($index = 0; $index -lt $Names.Count; $index += 1) {
        $name = $Names[$index]
        $entryType = if ($VerboseLines[$index].Length -eq 0) {
            [char]0
        }
        else {
            $VerboseLines[$index][0]
        }
        if ($entryType -ne "-" -and $entryType -ne "d") {
            throw "Unsafe tar entry type '$entryType': $name"
        }
        if (
            [string]::IsNullOrWhiteSpace($name) -or
            $name.Contains("\") -or
            $name.Contains([char]0) -or
            $name.StartsWith("/") -or
            $name -match "^[A-Za-z]:" -or
            ($name -ne "package" -and -not $name.StartsWith("package/"))
        ) {
            throw "Unsafe tar path: $name"
        }

        $segments = $name.Split("/")
        for ($segmentIndex = 0; $segmentIndex -lt $segments.Count; $segmentIndex += 1) {
            $segment = $segments[$segmentIndex]
            $isTrailingDirectorySeparator = (
                $segmentIndex -eq ($segments.Count - 1) -and
                [string]::IsNullOrEmpty($segment) -and
                $entryType -eq "d"
            )
            if (
                -not $isTrailingDirectorySeparator -and
                (
                    [string]::IsNullOrEmpty($segment) -or
                    $segment -eq "." -or
                    $segment -eq ".."
                )
            ) {
                throw "Unsafe tar path: $name"
            }
        }
    }
}

function Assert-SafeExtractedTree {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    $fullRoot = (Resolve-Path -LiteralPath $Root).Path
    Assert-SafeTemporaryPath -Path $fullRoot | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $fullRoot -Force -Recurse) {
        if (
            ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne
            0
        ) {
            throw "Extracted tree contains a reparse point: $($item.FullName)"
        }
        Assert-PathUnderRoot `
            -Path $item.FullName `
            -Root $fullRoot `
            -Message "Extracted item is outside its root" |
            Out-Null
    }
}

Export-ModuleMember -Function @(
    "Assert-BrowserPackageLock",
    "Assert-NoVncPackage",
    "Assert-SafeExtractedTree",
    "Assert-SafeRepositoryTarget",
    "Assert-SafeTarEntries",
    "Assert-SafeTemporaryPath",
    "Assert-WebAssetTree",
    "Get-NoVncReleasePolicy",
    "Write-WebAssetManifest"
)
