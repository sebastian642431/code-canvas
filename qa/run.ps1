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
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    public static IntPtr Largest(uint pid) {
        IntPtr best = IntPtr.Zero; long most = 0;
        EnumWindows((h, p) => {
            uint owner; GetWindowThreadProcessId(h, out owner);
            if (owner != pid || !IsWindowVisible(h)) return true;
            RECT r; GetWindowRect(h, out r);
            long area = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
            if (area > most) { most = area; best = h; }
            return true;
        }, IntPtr.Zero);
        return best;
    }
}
"@

function Save-Window($process, $file) {
    $process.Refresh()
    if ($process.HasExited) { return $false }
    $h = [QaWin]::Largest([uint32]$process.Id)
    if ($h -eq [IntPtr]::Zero) { return $false }
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
$many = @{ 'expect-log' = @(); 'forbid-log' = @(); 'allow-log' = @(); 'expect-file' = @(); 'absent-file' = @(); 'guest-expect-log' = @(); 'again-expect-log' = @(); 'again-forbid-log' = @(); 'relay-expect-log' = @() }
$steps = @{ host = @(); guest = @(); again = @() }
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
    if ($text -eq '=== again') { $part = 'again'; continue }
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
New-Item -ItemType Directory -Force -Path $fix, $out, $data, (Join-Path $Work 'claude'), (Join-Path $Work 'codex') | Out-Null
$prefs = $null
if ($head.ContainsKey('prefs')) { $prefs = $head['prefs'] }
if ($prefs) { Set-Content -LiteralPath (Join-Path $data 'prefs.json') -Value $prefs -Encoding ASCII }
if ($head.ContainsKey('extensions')) {
    $into = Join-Path $data 'extensions'
    New-Item -ItemType Directory -Force -Path $into | Out-Null
    foreach ($one in @($head['extensions'].Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ })) {
        Copy-Item -Recurse -Path (Join-Path $PSScriptRoot "fake\$one") -Destination (Join-Path $into $one)
    }
}

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

$relay = $null
$relayAddress = ''
$relayLog = Join-Path $Work 'relay.log'
if ($head.ContainsKey('relay')) {
    $relayExe = Join-Path (Split-Path -Parent $Exe) 'canvas-relay.exe'
    if (-not (Test-Path $relayExe)) { Write-Output "FAIL $name`: no relay at $relayExe (cargo build --release)"; exit 2 }
    $relayAddress = "127.0.0.1:$(Get-Random -Minimum 40001 -Maximum 50000)"
    $relay = Start-Process -FilePath $relayExe -ArgumentList @('--listen', $relayAddress) -PassThru -WindowStyle Hidden -RedirectStandardError $relayLog -RedirectStandardOutput (Join-Path $Work 'relay.out')
    Start-Sleep -Milliseconds 600
    if ($relay.HasExited) { Write-Output "FAIL $name`: the relay did not start: $(Get-Content $relayLog -Raw)"; exit 2 }
}

function Fill([string]$step) {
    $places = @{ '{FIX}' = $fix; '{WORK}' = $Work; '{REPO}' = $repo; '{RELAY}' = $relayAddress }
    foreach ($name in $places.Keys) {
        $path = $places[$name]
        if ($step.TrimStart().StartsWith('agent ') -or $step.TrimStart().StartsWith('touch ')) { $path = $path.Replace('\', '/') }
        $step = $step.Replace($name, $path)
    }
    return $step
}
$hostScript = (@($steps.host) | ForEach-Object { Fill $_ }) -join '; '
$guestScript = (@($steps.guest) | ForEach-Object { Fill $_ }) -join '; '
$againScript = (@($steps.again) | ForEach-Object { Fill $_ }) -join '; '

function Read-Log($dir) {
    $file = Join-Path $dir 'code-canvas.log'
    if (Test-Path $file) { return @(Get-Content -LiteralPath $file -Encoding UTF8) }
    return @()
}

$env:CODE_CANVAS_SHARE_ON = '127.0.0.1'
$env:CODE_CANVAS_QUIET = '1'
$env:CLAUDE_CONFIG_DIR = Join-Path $Work 'claude'
$env:CODEX_HOME = Join-Path $Work 'codex'
if ($head.ContainsKey('codex')) { $env:CODEX_HOME = Join-Path $fix $head['codex'] }
$fakeGitHub = $null
foreach ($variable in 'CODE_CANVAS_GITHUB_SITE', 'CODE_CANVAS_GITHUB_API', 'CODE_CANVAS_GITHUB_CLIENT') { Remove-Item "Env:$variable" -ErrorAction SilentlyContinue }
if ($head.ContainsKey('github')) {
    $port = Get-Random -Minimum 20000 -Maximum 40000
    $fakeArgs = @('-I', (Join-Path $PSScriptRoot 'fake\github.py'), $port)
    if ($head['github'] -ne 'empty') { $fakeArgs += (Join-Path $PSScriptRoot "fake\$($head['github'])") }
    $fakeGitHub = Start-Process -FilePath python -ArgumentList $fakeArgs -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 800
    $env:CODE_CANVAS_GITHUB_SITE = "http://127.0.0.1:$port"
    $env:CODE_CANVAS_GITHUB_API = "http://127.0.0.1:$port"
    $env:CODE_CANVAS_GITHUB_CLIENT = 'fake'
}
$problems = @()
$seen = @{}
$pictures = 0
$logs = @{ '' = @(); 'again-' = @() }
$rounds = @('')
if ($steps.again.Count -gt 0) { $rounds += 'again-' }
foreach ($round in $rounds) {
$env:CODE_CANVAS_DATA_DIR = $data
$running = @()
if ($round -eq '') {
    $env:CODE_CANVAS_SCRIPT = $hostScript
    if ($folders.Count -gt 0) { $first = Start-Process -FilePath $Exe -ArgumentList $folders -PassThru } else { $first = Start-Process -FilePath $Exe -PassThru }
} else {
    if ($problems.Count -gt 0) { break }
    $old = Join-Path $data 'marks'
    if (Test-Path $old) { Remove-Item -Recurse -Force $old }
    $env:CODE_CANVAS_SCRIPT = $againScript
    $first = Start-Process -FilePath $Exe -PassThru
}
$running += , @($first, $data, $round)

$invite = Join-Path $data 'invite.txt'
$guestDue = $round -eq '' -and $steps.guest.Count -gt 0
$inviteBy = (Get-Date).AddSeconds(30)
$deadline = (Get-Date).AddSeconds($limit)
while (@($running | Where-Object { -not $_[0].HasExited }).Count -gt 0 -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 100
    if ($guestDue -and (Test-Path $invite)) {
        $guestDue = $false
        Start-Sleep -Milliseconds 200
        New-Item -ItemType Directory -Force -Path $guestData | Out-Null
        if ($prefs) { Set-Content -LiteralPath (Join-Path $guestData 'prefs.json') -Value $prefs -Encoding ASCII }
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
$logs[$round] = Read-Log $data
}

$log = $logs['']
$againLog = $logs['again-']
$guestLog = Read-Log $guestData
$always = @('panicked', 'panic:', '\] ERROR ')
foreach ($pair in @(@('', $log, 'forbid-log'), @('guest ', $guestLog, 'forbid-log'), @('again ', $againLog, 'again-forbid-log'))) {
    $forbidden = $always + $many[$pair[2]]
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
foreach ($want in $many['again-expect-log']) {
    if (-not ($againLog | Where-Object { $_ -match $want })) { $problems += "the log of the second start never said /$want/" }
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

if ($fakeGitHub) {
    try { & taskkill /PID $fakeGitHub.Id /T /F | Out-Null } catch {}
    foreach ($variable in 'CODE_CANVAS_GITHUB_SITE', 'CODE_CANVAS_GITHUB_API', 'CODE_CANVAS_GITHUB_CLIENT') { Remove-Item "Env:$variable" -ErrorAction SilentlyContinue }
}

if ($relay) {
    try { & taskkill /PID $relay.Id /T /F | Out-Null } catch {}
    $relay.WaitForExit(3000) | Out-Null
    $said = @()
    if (Test-Path $relayLog) { $said = @(Get-Content -LiteralPath $relayLog -Encoding UTF8) }
    foreach ($want in $many['relay-expect-log']) {
        if (-not ($said | Where-Object { $_ -match $want })) { $problems += "the relay never said /$want/" }
    }
    foreach ($line in $said) {
        if ($line -match 'panicked') { $problems += "relay: $line" }
    }
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
