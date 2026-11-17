# Starts a test instance of Code Canvas with a data folder of its own, runs a
# script in it (see "Testing the real app" in docs/ARCHITECTURE.md), and saves
# a picture of ITS window only, not of the screen. For checking how a change
# looks without clicking around:
#
#   scriptsscreenshot.ps1 -Exe targetdebugde-canvas.exe -Data $env:TEMP-test `
#       -Script "left settings; wait 4000; quit" -Out shot.png -After 4500 -Folders "C:someolder"
param(
    [string]$Exe,
    [string]$Data,
    [string]$Script,
    [string]$Out,
    [int]$After = 3500,
    [string[]]$Folders
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

New-Item -ItemType Directory -Force -Path $Data | Out-Null
$env:CODE_CANVAS_DATA_DIR = $Data
$env:CODE_CANVAS_SCRIPT = $Script
$list = @($Folders | ForEach-Object { $_ -split ',' })
$p = Start-Process -FilePath $Exe -ArgumentList $list -PassThru
Start-Sleep -Milliseconds $After
$p.Refresh()
$h = $p.MainWindowHandle
if ($h -eq [IntPtr]::Zero) {
    Write-Output "no window (exited: $($p.HasExited), code: $($p.ExitCode))"
} else {
    $r = New-Object Win+RECT
    [Win]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    # 2 = PW_RENDERFULLCONTENT: what the GPU drew, not just the frame.
    $ok = [Win]::PrintWindow($h, $dc, 2)
    $g.ReleaseHdc($dc)
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Output "captured ${w}x${hh} ok=$ok"
}
if (-not $p.WaitForExit(15000)) { $p.Kill(); Write-Output "had to be stopped" }
Write-Output "exit code: $($p.ExitCode)"
