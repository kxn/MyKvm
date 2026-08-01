[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Import-Module (Join-Path $PSScriptRoot "web-assets-tools.psm1") -Force

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

function New-TestRoot {
    $temporaryRoot = [System.IO.Path]::GetFullPath(
        [System.IO.Path]::GetTempPath()
    )
    $root = [System.IO.Path]::GetFullPath(
        (Join-Path $temporaryRoot ("my-ipkvm-web-assets-" + [guid]::NewGuid()))
    )
    Assert-SafeTemporaryPath -Path $root
    return $root
}

function New-NoVncFixture {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    $package = Join-Path $Root "package"
    Set-Utf8File (Join-Path $package "AUTHORS") "fixture authors"
    Set-Utf8File (Join-Path $package "LICENSE.txt") "fixture license"
    Set-Utf8File `
        (Join-Path $package "docs/LICENSE.MPL-2.0") `
        "fixture MPL"
    Set-Utf8File `
        (Join-Path $package "vendor/pako/LICENSE") `
        "fixture pako license"
    Set-Utf8File `
        (Join-Path $package "core/crypto/des.js") `
        "fixture BSD notice"
    Set-Utf8File `
        (Join-Path $package "core/rfb.js") `
        "export default class RFB {}"
    Set-Utf8File `
        (Join-Path $package "package.json") `
        @'
{
  "name": "@novnc/novnc",
  "version": "1.7.0",
  "license": "MPL-2.0",
  "dependencies": {}
}
'@

    Set-Utf8File `
        (Join-Path $Root "npm-metadata.json") `
        @'
{
  "version": "1.7.0",
  "gitHead": "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e",
  "dist": {
    "tarball": "https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz",
    "integrity": "sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA==",
    "shasum": "7f832cf07c66475a81a25708b8e5299a5c4efec5"
  }
}
'@

    Set-Utf8File `
        (Join-Path $Root "npm-attestations.json") `
        @'
{
  "attestations": [
    {
      "predicateType": "https://slsa.dev/provenance/v1",
      "repository": "https://github.com/novnc/noVNC",
      "commit": "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e"
    }
  ]
}
'@

    return $package
}

function New-BrowserLockFixture {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    $browserRoot = Join-Path $Root "browser-tests"
    Set-Utf8File `
        (Join-Path $browserRoot "package.json") `
        @'
{
  "name": "my-ipkvm-browser-tests",
  "private": true,
  "type": "module",
  "devDependencies": {
    "playwright-core": "1.62.1"
  }
}
'@
    Set-Utf8File `
        (Join-Path $browserRoot "package-lock.json") `
        @'
{
  "name": "my-ipkvm-browser-tests",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": {
      "name": "my-ipkvm-browser-tests",
      "devDependencies": {
        "playwright-core": "1.62.1"
      }
    },
    "node_modules/playwright-core": {
      "version": "1.62.1",
      "resolved": "https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz",
      "integrity": "sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw==",
      "dev": true,
      "license": "Apache-2.0",
      "engines": {
        "node": ">=20"
      }
    }
  }
}
'@

    return $browserRoot
}

$root = New-TestRoot
$null = New-Item -ItemType Directory -Path $root

try {
    $noVncRoot = Join-Path $root "novnc"
    $packageRoot = New-NoVncFixture -Root $noVncRoot
    $manifestPath = Join-Path $noVncRoot "manifest.sha256"
    Write-WebAssetManifest -Root $packageRoot -Path $manifestPath

    Assert-WebAssetTree -Root $packageRoot -ManifestPath $manifestPath
    Assert-NoVncPackage `
        -PackageRoot $packageRoot `
        -MetadataPath (Join-Path $noVncRoot "npm-metadata.json") `
        -AttestationsPath (Join-Path $noVncRoot "npm-attestations.json")

    $rfbPath = Join-Path $packageRoot "core/rfb.js"
    Set-Utf8File $rfbPath "tampered"
    Assert-ThrowsLike {
        Assert-WebAssetTree -Root $packageRoot -ManifestPath $manifestPath
    } "hash mismatch.*core/rfb\.js"
    Set-Utf8File $rfbPath "export default class RFB {}"

    $authorsPath = Join-Path $packageRoot "AUTHORS"
    Remove-Item -LiteralPath $authorsPath
    Assert-ThrowsLike {
        Assert-WebAssetTree -Root $packageRoot -ManifestPath $manifestPath
    } "missing.*AUTHORS"
    Set-Utf8File $authorsPath "fixture authors"

    $extraPath = Join-Path $packageRoot "unexpected.js"
    Set-Utf8File $extraPath "unexpected"
    Assert-ThrowsLike {
        Assert-WebAssetTree -Root $packageRoot -ManifestPath $manifestPath
    } "unexpected.*unexpected\.js"
    Remove-Item -LiteralPath $extraPath

    $packageJsonPath = Join-Path $packageRoot "package.json"
    $validPackageJson = Get-Content -Raw -Encoding utf8 $packageJsonPath
    Set-Utf8File `
        $packageJsonPath `
        ($validPackageJson.Replace('"1.7.0"', '"1.7.1"'))
    Assert-ThrowsLike {
        Assert-NoVncPackage `
            -PackageRoot $packageRoot `
            -MetadataPath (Join-Path $noVncRoot "npm-metadata.json") `
            -AttestationsPath (Join-Path $noVncRoot "npm-attestations.json")
    } "package version"
    Set-Utf8File $packageJsonPath $validPackageJson

    $metadataPath = Join-Path $noVncRoot "npm-metadata.json"
    $validMetadata = Get-Content -Raw -Encoding utf8 $metadataPath
    Set-Utf8File `
        $metadataPath `
        ($validMetadata.Replace(
            "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e",
            "0000000000000000000000000000000000000000"
        ))
    Assert-ThrowsLike {
        Assert-NoVncPackage `
            -PackageRoot $packageRoot `
            -MetadataPath $metadataPath `
            -AttestationsPath (Join-Path $noVncRoot "npm-attestations.json")
    } "gitHead"
    Set-Utf8File $metadataPath $validMetadata

    $pakoLicense = Join-Path $packageRoot "vendor/pako/LICENSE"
    Remove-Item -LiteralPath $pakoLicense
    Assert-ThrowsLike {
        Assert-NoVncPackage `
            -PackageRoot $packageRoot `
            -MetadataPath $metadataPath `
            -AttestationsPath (Join-Path $noVncRoot "npm-attestations.json")
    } "required noVNC file.*vendor/pako/LICENSE"
    Set-Utf8File $pakoLicense "fixture pako license"

    $browserRoot = New-BrowserLockFixture -Root $root
    $browserPackage = Join-Path $browserRoot "package.json"
    $browserLock = Join-Path $browserRoot "package-lock.json"
    Assert-BrowserPackageLock `
        -PackageJsonPath $browserPackage `
        -PackageLockPath $browserLock

    $validLock = Get-Content -Raw -Encoding utf8 $browserLock
    Set-Utf8File `
        $browserLock `
        ($validLock.Replace(
            "https://registry.npmjs.org/playwright-core/",
            "https://example.invalid/playwright-core/"
        ))
    Assert-ThrowsLike {
        Assert-BrowserPackageLock `
            -PackageJsonPath $browserPackage `
            -PackageLockPath $browserLock
    } "registry source"
    Set-Utf8File $browserLock $validLock

    Set-Utf8File `
        $browserLock `
        ($validLock.Replace(
            '"node_modules/playwright-core": {',
            '"node_modules/unapproved": {' + [Environment]::NewLine +
            '      "version": "1.0.0",' + [Environment]::NewLine +
            '      "resolved": "https://registry.npmjs.org/unapproved/-/unapproved-1.0.0.tgz",' +
            [Environment]::NewLine +
            '      "integrity": "sha512-invalid",' + [Environment]::NewLine +
            '      "license": "MIT"' + [Environment]::NewLine +
            '    },' + [Environment]::NewLine +
            '    "node_modules/playwright-core": {'
        ))
    Assert-ThrowsLike {
        Assert-BrowserPackageLock `
            -PackageJsonPath $browserPackage `
            -PackageLockPath $browserLock
    } "unapproved package"
    Set-Utf8File $browserLock $validLock

    Assert-SafeTarEntries `
        -Names @("package/", "package/core/rfb.js") `
        -VerboseLines @(
            "drwxr-xr-x  0 0 0 0 Jan 01 00:00 package/",
            "-rw-r--r--  0 0 0 1 Jan 01 00:00 package/core/rfb.js"
        )

    Assert-ThrowsLike {
        Assert-SafeTarEntries `
            -Names @("package/../outside") `
            -VerboseLines @(
                "-rw-r--r--  0 0 0 1 Jan 01 00:00 package/../outside"
            )
    } "unsafe tar path"

    Assert-ThrowsLike {
        Assert-SafeTarEntries `
            -Names @("C:/outside") `
            -VerboseLines @(
                "-rw-r--r--  0 0 0 1 Jan 01 00:00 C:/outside"
            )
    } "unsafe tar path"

    Assert-ThrowsLike {
        Assert-SafeTarEntries `
            -Names @("package/link") `
            -VerboseLines @(
                "lrwxr-xr-x  0 0 0 0 Jan 01 00:00 package/link -> outside"
            )
    } "unsafe tar entry type"

    Assert-ThrowsLike {
        Assert-SafeTarEntries `
            -Names @("package/hardlink") `
            -VerboseLines @(
                "hrw-r--r--  0 0 0 0 Jan 01 00:00 package/hardlink link to outside"
            )
    } "unsafe tar entry type"

    Assert-ThrowsLike {
        Assert-SafeTemporaryPath -Path (Join-Path $PSScriptRoot "not-temporary")
    } "outside the system temporary directory"
}
finally {
    if (Test-Path -LiteralPath $root) {
        Assert-SafeTemporaryPath -Path $root
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}

Write-Host "Web asset policy tests passed."
$global:LASTEXITCODE = 0
