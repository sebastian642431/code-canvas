param([switch]$Purge)
$ErrorActionPreference = "Stop"
$dir = Join-Path $env:LOCALAPPDATA "Programs\Hawk Canvas"
$exe = Join-Path $dir "hawk-canvas.exe"

$running = Get-Process hawk-canvas -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
if ($running) { throw "Hawk Canvas is running. Close it and run this script again." }

foreach ($link in @(
        (Join-Path ([Environment]::GetFolderPath("Programs")) "Hawk Canvas.lnk"),
        (Join-Path ([Environment]::GetFolderPath("Desktop")) "Hawk Canvas.lnk"))) {
    if (Test-Path $link) { Remove-Item $link; Write-Host "Removed $link" }
}
foreach ($key in @("HKCU:\Software\Classes\Directory\shell\HawkCanvas", "HKCU:\Software\Classes\Directory\Background\shell\HawkCanvas")) {
    if (Test-Path $key) { Remove-Item $key -Recurse; Write-Host "Removed $key" }
}
if (Test-Path $dir) { Remove-Item $dir -Recurse; Write-Host "Removed $dir" }
if ($Purge) {
    foreach ($name in @("hawk-canvas", "code-canvas")) {
        $data = Join-Path $env:APPDATA $name
        if (Test-Path $data) { Remove-Item $data -Recurse; Write-Host "Removed $data" }
    }
}
Write-Host "Hawk Canvas was uninstalled."
