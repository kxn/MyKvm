[CmdletBinding()]
param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot "..\target\release\ipkvm-desktop-iced.exe"),
    [ValidateRange(1, 120)]
    [int]$StartupTimeoutSeconds = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$resolvedExecutable = [System.IO.Path]::GetFullPath($ExecutablePath)
if (-not (Test-Path -LiteralPath $resolvedExecutable -PathType Leaf)) {
    throw "Desktop release executable does not exist: $resolvedExecutable"
}

$child = $null
try {
    $child = Start-Process `
        -FilePath $resolvedExecutable `
        -WorkingDirectory (Split-Path -Parent $resolvedExecutable) `
        -PassThru

    $deadline = [DateTime]::UtcNow.AddSeconds($StartupTimeoutSeconds)
    $windowHandle = [IntPtr]::Zero
    while ([DateTime]::UtcNow -lt $deadline) {
        $process = Get-Process -Id $child.Id -ErrorAction SilentlyContinue
        if ($null -eq $process) {
            throw "Desktop release exited before creating a top-level window (pid=$($child.Id))"
        }
        if ($process.HasExited) {
            throw "Desktop release exited with code $($process.ExitCode) before creating a top-level window"
        }

        $process.Refresh()
        $windowHandle = $process.MainWindowHandle
        if ($windowHandle -ne [IntPtr]::Zero) {
            break
        }
        Start-Sleep -Milliseconds 100
    }

    if ($windowHandle -eq [IntPtr]::Zero) {
        throw "Desktop release stayed alive but created no top-level window within ${StartupTimeoutSeconds}s"
    }

    Write-Host "Desktop release startup passed: pid=$($child.Id), hwnd=$windowHandle"
}
finally {
    if ($null -ne $child) {
        $process = Get-Process -Id $child.Id -ErrorAction SilentlyContinue
        if ($null -ne $process -and -not $process.HasExited) {
            Stop-Process -Id $child.Id -Force
        }
        $child.Dispose()
    }
}
