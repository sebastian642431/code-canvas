param(
    [string]$Exe,
    [string]$Data,
    [string]$Script,
    [string]$Out,
    [int]$After = 3500,
    [string[]]$Folders,
    [switch]$Marks,
    [int]$Limit = 90
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
"@

function Save-Window($process, $file) {
    $process.Refresh()
    $h = $process.MainWindowHandle
    if ($process.HasExited -or $h -eq [IntPtr]::Zero) {
        Write-Output "no window for $file (exited: $($process.HasExited))"
        return
    }
    $r = New-Object Win+RECT
    [Win]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    $ok = [Win]::PrintWindow($h, $dc, 2)
    $g.ReleaseHdc($dc)
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Output "captured $file ${w}x${hh} ok=$ok"
}

New-Item -ItemType Directory -Force -Path $Data | Out-Null
$env:CODE_CANVAS_DATA_DIR = $Data
$env:CODE_CANVAS_SCRIPT = $Script
$list = @($Folders | ForEach-Object { $_ -split ',' })
$p = Start-Process -FilePath $Exe -ArgumentList $list -PassThru

if ($Marks) {
    $folder = Join-Path $Data 'marks'
    $pictures = Split-Path -Parent $Out
    $seen = @{}
    $deadline = (Get-Date).AddSeconds($Limit)
    while (-not $p.HasExited -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 100
        if (-not (Test-Path $folder)) { continue }
        foreach ($mark in Get-ChildItem -Path $folder -File) {
            if ($seen.ContainsKey($mark.Name)) { continue }
            $seen[$mark.Name] = $true
            Start-Sleep -Milliseconds 350
            Save-Window $p (Join-Path $pictures ($mark.Name + '.png'))
        }
    }
} else {
    Start-Sleep -Milliseconds $After
    Save-Window $p $Out
}

if (-not $p.WaitForExit(15000)) { $p.Kill(); Write-Output "had to be stopped" }
Write-Output "exit code: $($p.ExitCode)"
