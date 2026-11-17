param(
    [Parameter(Mandatory = $true)][string]$Scenario,
    [string]$Exe,
    [string]$Work
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repo 'target\release\code-canvas.exe' }
if (-not (Test-Path $Exe)) { Write-Output "FAIL: no program at $Exe (cargo build --release)"; exit 2 }
$Scenario = (Resolve-Path $Scenario).Path
$name = [IO.Path]::GetFileNameWithoutExtension($Scenario)
if (-not $Work) { $Work = Join-Path $env:TEMP "code-canvas-qa\$name" }

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class QaWin {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
"@

function Save-Window($process, $file) {
    $process.Refresh()
    $h = $process.MainWindowHandle
    if ($process.HasExited -or $h -eq [IntPtr]::Zero) { return $false }
    $r = New-Object QaWin+RECT
    [QaWin]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
    if ($w -le 0 -or $hh -le 0) { return $false }
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [QaWin]::PrintWindow($h, $dc, 2) | Out-Null
    $g.ReleaseHdc($dc)
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    return $true
}

$head = @{}
$many = @{ 'expect-log' = @(); 'forbid-log' = @(); 'allow-log' = @(); 'expect-file' = @(); 'absent-file' = @(); 'guest-expect-log' = @() }
$steps = @{ host = @(); guest = @() }
$part = 'head'
foreach ($line in Get-Content -LiteralPath $Scenario -Encoding UTF8) {
    $text = $line.Trim()
    if ($part -eq 'head') {
        if ($text -eq '') { $part = 'host'; continue }
        $at = $text.IndexOf(':')
        if ($at -lt 1) { Write-Output "FAIL $name`: the header line '$text' is not 'key: value'"; exit 2 }
        $key = $text.Substring(0, $at).Trim(); $value = $text.Substring($at + 1).Trim()
        if ($many.ContainsKey($key)) { $many[$key] += $value } else { $head[$key] = $value }
        continue
    }
    if ($text -eq '=== guest') { $part = 'guest'; continue }
    if ($text -ne '') { $steps[$part] += $text }
}
if ($steps.host.Count -eq 0) { Write-Output "FAIL $name`: no steps"; exit 2 }

$limit = 90
if ($head.ContainsKey('limit')) { $limit = [int]$head['limit'] }
$fix = Join-Path $Work 'fix'
$out = Join-Path $Work 'shots'
$data = Join-Path $Work 'data'
$guestData = Join-Path $Work 'data-guest'
if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
New-Item -ItemType Directory -Force -Path $fix, $out, $data, (Join-Path $Work 'claude') | Out-Null

$wanted = @()
if ($head.ContainsKey('folders')) { $wanted = @($head['folders'].Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }
$made = @()
if ($head.ContainsKey('fixtures')) { $made = @($head['fixtures'].Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }
$all = @($wanted + $made | ForEach-Object { $_.Split('/')[0] } | Select-Object -Unique)
if ($all.Count -gt 0) {
    & python (Join-Path $PSScriptRoot 'fixtures.py') $fix @all
    if ($LASTEXITCODE -ne 0) { Write-Output "FAIL $name`: the fixtures could not be made"; exit 2 }
}
$folders = @($wanted | ForEach-Object { Join-Path $fix ($_ -replace '/', '\') })

function Fill([string]$text) {
    return $text.Replace('{FIX}', $fix).Replace('{WORK}', $Work).Replace('{REPO}', $repo)
}
$hostScript = Fill (($steps.host) -join '; ')
$guestScript = Fill (($steps.guest) -join '; ')

$env:CODE_CANVAS_SHARE_ON = '127.0.0.1'
$env:CLAUDE_CONFIG_DIR = Join-Path $Work 'claude'
$env:CODE_CANVAS_DATA_DIR = $data
$env:CODE_CANVAS_SCRIPT = $hostScript
$running = @()
if ($folders.Count -gt 0) { $first = Start-Process -FilePath $Exe -ArgumentList $folders -PassThru } else { $first = Start-Process -FilePath $Exe -PassThru }
$running += , @($first, $data, '')
$problems = @()

$invite = Join-Path $data 'invite.txt'
$guestDue = $steps.guest.Count -gt 0
$inviteBy = (Get-Date).AddSeconds(30)
$seen = @{}
$pictures = 0
$deadline = (Get-Date).AddSeconds($limit)
while (@($running | Where-Object { -not $_[0].HasExited }).Count -gt 0 -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 100
    if ($guestDue -and (Test-Path $invite)) {
        $guestDue = $false
        Start-Sleep -Milliseconds 200
        New-Item -ItemType Directory -Force -Path $guestData | Out-Null
        $env:CODE_CANVAS_DATA_DIR = $guestData
        $env:CODE_CANVAS_SCRIPT = $guestScript.Replace('{INVITE}', (Get-Content $invite -Raw).Trim())
        $second = Start-Process -FilePath $Exe -PassThru
        $running += , @($second, $guestData, 'guest-')
    } elseif ($guestDue -and (Get-Date) -gt $inviteBy) {
        $guestDue = $false
        $problems += 'the host never shared its session (no invite.txt)'
    }
    foreach ($one in $running) {
        $marks = Join-Path $one[1] 'marks'
        if (-not (Test-Path $marks)) { continue }
        foreach ($mark in Get-ChildItem -Path $marks -File) {
            $key = "$($one[2])$($mark.Name)"
            if ($seen.ContainsKey($key)) { continue }
            $seen[$key] = $true
            Start-Sleep -Milliseconds 350
            if (Save-Window $one[0] (Join-Path $out "$key.png")) { $pictures += 1 } else { $problems += "no window to picture at mark '$key'" }
        }
    }
}
foreach ($one in $running) {
    if (-not $one[0].WaitForExit(8000)) {
        $problems += "$($one[2])instance did not end by itself within $limit s (stopped)"
        try { & taskkill /PID $one[0].Id /T /F | Out-Null } catch {}
    } elseif ($one[0].ExitCode -ne 0) {
        $problems += "$($one[2])instance ended with code $($one[0].ExitCode)"
    }
}

function Read-Log($dir) {
    $file = Join-Path $dir 'code-canvas.log'
    if (Test-Path $file) { return @(Get-Content -LiteralPath $file -Encoding UTF8) }
    return @()
}
$log = Read-Log $data
$guestLog = Read-Log $guestData
$forbidden = @('panicked', 'panic:', '\] ERROR ') + $many['forbid-log']
foreach ($pair in @(@('', $log), @('guest ', $guestLog))) {
    foreach ($line in $pair[1]) {
        $allowed = $false
        foreach ($ok in $many['allow-log']) { if ($line -match $ok) { $allowed = $true } }
        if ($allowed) { continue }
        foreach ($bad in $forbidden) {
            if ($line -match $bad) { $problems += "$($pair[0])log: $($line.Substring(0, [Math]::Min(220, $line.Length)))"; break }
        }
    }
}
foreach ($want in $many['expect-log']) {
    if (-not ($log | Where-Object { $_ -match $want })) { $problems += "the log never said /$want/" }
}
foreach ($want in $many['guest-expect-log']) {
    if (-not ($guestLog | Where-Object { $_ -match $want })) { $problems += "the guest's log never said /$want/" }
}
foreach ($want in $many['expect-file']) {
    $at = $want.IndexOf('::')
    $file = Fill ($want.Substring(0, $at).Trim()); $regex = $want.Substring($at + 2).Trim()
    if (-not [IO.Path]::IsPathRooted($file)) { $file = Join-Path $fix $file }
    if (-not (Test-Path -LiteralPath $file)) { $problems += "missing file $file" }
    elseif (-not ([IO.File]::ReadAllText($file) -match $regex)) { $problems += "$file does not match /$regex/" }
}
foreach ($want in $many['absent-file']) {
    $file = Fill $want
    if (-not [IO.Path]::IsPathRooted($file)) { $file = Join-Path $fix $file }
    if (Test-Path -LiteralPath $file) { $problems += "$file should not be there" }
}

$ours = @('code-canvas.exe', 'node.exe', 'rust-analyzer.exe', 'rust-analyzer-proc-macro-srv.exe')
$leftover = @(Get-CimInstance Win32_Process | Where-Object { $ours -contains $_.Name -and $_.CommandLine -and $_.CommandLine.Contains($Work) })
foreach ($process in $leftover) {
    $problems += "left running: $($process.Name) ($($process.ProcessId))"
    try { & taskkill /PID $process.ProcessId /T /F | Out-Null } catch {}
}

if ($problems.Count -eq 0) {
    Write-Output "PASS $name ($pictures pictures in $out)"
    exit 0
}
Write-Output "FAIL $name"
foreach ($problem in ($problems | Select-Object -Unique)) { Write-Output "  - $problem" }
Write-Output "  work folder: $Work"
exit 1
