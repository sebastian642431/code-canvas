# Removes what scripts\install.ps1 added. Settings and the session file in
# %APPDATA%\code-canvas are kept unless -Purge is given.
param([switch]$Purge)
$ErrorActionPreference = "Stop"
$dir = Join-Path $env:LOCALAPPDATA "Programs\Code Canvas"
$exe = Join-Path $dir "code-canvas.exe"

$running = Get-Process code-canvas -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
if ($running) { throw "Code Canvas is running. Close it and run this script again." }

foreach ($link in @(
        (Join-Path ([Environment]::GetFolderPath("Programs")) "Code Canvas.lnk"),
        (Join-Path ([Environment]::GetFolderPath("Desktop")) "Code Canvas.lnk"))) {
    if (Test-Path $link) { Remove-Item $link; Write-Host "Removed $link" }
}
foreach ($key in @("HKCU:\Software\Classes\Directory\shell\CodeCanvas", "HKCU:\Software\Classes\Directory\Background\shell\CodeCanvas")) {
    if (Test-Path $key) { Remove-Item $key -Recurse; Write-Host "Removed $key" }
}
if (Test-Path $dir) { Remove-Item $dir -Recurse; Write-Host "Removed $dir" }
if ($Purge) {
    $data = Join-Path $env:APPDATA "code-canvas"
    if (Test-Path $data) { Remove-Item $data -Recurse; Write-Host "Removed $data" }
}
Write-Host "Code Canvas was uninstalled."
