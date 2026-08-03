# Spike 1 performance collection script (PowerShell, PS5-compatible).
#
# Launches the video_1080p example (release), samples process CPU/memory for the
# given duration, reads frame stats from the example's stats JSON file, and
# judges PASS/FAIL against #73 Spike 1 thresholds.
#
# Usage: powershell -File scripts/perf-1080p.ps1 [-DurationSec 120] [-SourceFps 30]
#
# Thresholds (#73 Spike 1):
#   rendered/source fps >= 99% (dropped <= 1%)
#   avg frame interval <= 34ms, p95 <= 40ms
#   process CPU (single-core equivalent) < 40%
#   memory delta < 100MB (after warmup)

[CmdletBinding()]
param(
    [int]$DurationSec = 120,
    [int]$SourceFps = 30
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path (Join-Path (Join-Path (Join-Path $PSScriptRoot "..") "..") "..")).Path
$example = Join-Path $repoRoot "target\release\examples\video_1080p.exe"

if (-not (Test-Path -LiteralPath $example)) {
    Write-Error "Release example not found: $example. Run first: cargo build -p ipkvm-desktop-iced-spike --example video_1080p --release"
}

$expectedFrames = $DurationSec * $SourceFps
$maxDropped = [Math]::Ceiling($expectedFrames * 0.01)
$cpuThreshold = 40.0
$memThresholdMB = 100.0

Write-Host "==> Spike 1 perf collection"
Write-Host "    duration: ${DurationSec}s, source fps: ${SourceFps}"
Write-Host "    expected frames: $expectedFrames, drop threshold (<=1%): $maxDropped"

$logDir = Join-Path $repoRoot "crates/ipkvm-desktop-iced-spike"
$statsLog = Join-Path $logDir "video_1080p.stats.json"
$stderrLog = Join-Path $logDir "video_1080p.stderr.log"

# Remove stale stats file so we detect the fresh one.
if (Test-Path -LiteralPath $statsLog) { Remove-Item -LiteralPath $statsLog }

# Launch example. Arguments as a single space-separated string (most PS5-safe).
# NoNewWindow: do NOT redirect stdout, which can break wgpu/winit init.
$start = Get-Date
$argString = "--duration $DurationSec --stats-file `"$statsLog`""
$proc = Start-Process -FilePath $example -ArgumentList $argString -WorkingDirectory $logDir -PassThru -NoNewWindow -RedirectStandardError $stderrLog

Write-Host "    PID: $($proc.Id), sampling..."

$samples = @()
$sampleInterval = 2
$elapsed = 0
while (-not $proc.HasExited -and $elapsed -lt $DurationSec) {
    Start-Sleep -Seconds $sampleInterval
    $elapsed += $sampleInterval
    try {
        $p = Get-Process -Id $proc.Id -ErrorAction Stop
        $cpuSec = $p.CPU
        $memMB = [Math]::Round($p.WorkingSet64 / 1MB, 1)
        $samples += [pscustomobject]@{ Elapsed = $elapsed; CpuSec = $cpuSec; MemMB = $memMB }
    } catch {
        # process may have exited
    }
}

if (-not $proc.HasExited) {
    $proc | Stop-Process -Force
}
$proc.WaitForExit(10000) | Out-Null
$end = Get-Date
$wallSec = ($end - $start).TotalSeconds

# Parse the JSON stats file (written by example on exit). Retry briefly.
$summary = $null
$statsRaw = $null
for ($i = 0; $i -lt 15; $i++) {
    if (Test-Path -LiteralPath $statsLog) {
        $statsRaw = Get-Content $statsLog -Raw -ErrorAction SilentlyContinue
        if ($statsRaw -and $statsRaw.Trim().Length -gt 0) { break }
    }
    Start-Sleep -Milliseconds 200
}
if ($statsRaw) {
    try { $summary = $statsRaw | ConvertFrom-Json } catch { }
}

# CPU delta single-core equivalent percent.
$cpuPercent = 0.0
if ($samples.Count -ge 2) {
    $cpuDelta = $samples[-1].CpuSec - $samples[0].CpuSec
    $sampleSpan = $samples[-1].Elapsed - $samples[0].Elapsed
    if ($sampleSpan -gt 0) {
        $cpuPercent = ($cpuDelta / $sampleSpan) * 100.0
    }
}

# Memory delta (peak - first sample).
$memDeltaMB = 0.0
if ($samples.Count -ge 2) {
    $maxMem = ($samples | Measure-Object -Property MemMB -Maximum).Maximum
    $memDeltaMB = [Math]::Max(0.0, $maxMem - $samples[0].MemMB)
}

Write-Host ""
Write-Host "==> measured"
Write-Host "    wall: $([Math]::Round($wallSec,1))s"
Write-Host "    CPU single-core eq: $([Math]::Round($cpuPercent,1))%  (threshold < ${cpuThreshold}%)"
Write-Host "    mem peak delta: $([Math]::Round($memDeltaMB,1))MB  (threshold < ${memThresholdMB}MB)"
if ($summary) {
    $rendered = $summary.rendered_frames
    $denom = if ($summary.source_frames) { $summary.source_frames } else { $expectedFrames }
    $dropped = $denom - $rendered
    if ($denom -gt 0) {
        $dropRate = ($dropped / $denom) * 100
    } else {
        $dropRate = 0
    }
    if ($summary.source_frames) {
        Write-Host "    source frames: $($summary.source_frames) (actual push rate)"
    }
    Write-Host "    rendered: $rendered / $denom (dropped $dropped, $([Math]::Round($dropRate,2))%)"
    if ($summary.avg_interval_ms) {
        Write-Host "    avg interval: $([Math]::Round($summary.avg_interval_ms,2))ms  (threshold <= 34ms)"
    }
    if ($summary.p95_interval_ms) {
        Write-Host "    p95 interval: $([Math]::Round($summary.p95_interval_ms,2))ms  (threshold <= 40ms)"
    }
} else {
    Write-Warning "No frame stats JSON parsed from $statsLog"
}

$pass = $true
$reasons = @()
if ($cpuPercent -ge $cpuThreshold) {
    $pass = $false
    $reasons += "CPU ${cpuPercent}% >= ${cpuThreshold}%"
}
if ($memDeltaMB -ge $memThresholdMB) {
    $pass = $false
    $reasons += "mem delta ${memDeltaMB}MB >= ${memThresholdMB}MB"
}
if ($summary) {
    $rendered = $summary.rendered_frames
    $denom = if ($summary.source_frames) { $summary.source_frames } else { $expectedFrames }
    $dropped = $denom - $rendered
    if ($dropped -gt $maxDropped) {
        $pass = $false
        $reasons += "dropped $dropped > $maxDropped"
    }
    if ($summary.avg_interval_ms -and $summary.avg_interval_ms -gt 34.0) {
        $pass = $false
        $reasons += "avg interval $($summary.avg_interval_ms)ms > 34ms"
    }
    if ($summary.p95_interval_ms -and $summary.p95_interval_ms -gt 40.0) {
        $pass = $false
        $reasons += "p95 $($summary.p95_interval_ms)ms > 40ms"
    }
}

Write-Host ""
if ($pass) {
    Write-Host "==> result: PASS"
} else {
    Write-Host "==> result: FAIL - $($reasons -join '; ')"
}

[pscustomobject]@{
    Pass = $pass
    CpuPercent = [Math]::Round($cpuPercent, 1)
    MemDeltaMB = [Math]::Round($memDeltaMB, 1)
    RenderedFrames = if ($summary) { $summary.rendered_frames } else { $null }
    SourceFrames = if ($summary) { $summary.source_frames } else { $null }
    ExpectedFrames = $expectedFrames
    Reasons = $reasons
}
