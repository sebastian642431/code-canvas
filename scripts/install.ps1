param(
    [switch]$NoBuild,
    [switch]$NoDesktop,
    [switch]$ContextMenu
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$source = Join-Path $repo "target\release\code-canvas.exe"
$dir = Join-Path $env:LOCALAPPDATA "Programs\Code Canvas"
$exe = Join-Path $dir "code-canvas.exe"

if (-not $NoBuild) {
    Write-Host "Building the release binary..."
    Push-Location $repo
    try { cargo build --release; if ($LASTEXITCODE -ne 0) { throw "cargo build failed" } } finally { Pop-Location }
}
if (-not (Test-Path $source)) { throw "Not found: $source (run without -NoBuild)" }

$running = Get-Process code-canvas -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
if ($running) { throw "Code Canvas is running from $exe. Close it and run this script again." }

New-Item -ItemType Directory -Force -Path $dir | Out-Null
Copy-Item $source $exe -Force
Write-Host "Installed: $exe"

$shell = New-Object -ComObject WScript.Shell
function New-Shortcut([string]$path) {
    $link = $shell.CreateShortcut($path)
    $link.TargetPath = $exe
    $link.WorkingDirectory = $dir
    $link.Description = "See a whole codebase on an infinite canvas"
    $link.IconLocation = "$exe,0"
    $link.Save()
    Write-Host "Shortcut:  $path"
}
New-Shortcut (Join-Path ([Environment]::GetFolderPath("Programs")) "Code Canvas.lnk")
if (-not $NoDesktop) { New-Shortcut (Join-Path ([Environment]::GetFolderPath("Desktop")) "Code Canvas.lnk") }
$pinned = Join-Path $env:APPDATA "Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"
if (Test-Path $pinned) {
    foreach ($file in Get-ChildItem -Path $pinned -Filter *.lnk) {
        $link = $shell.CreateShortcut($file.FullName)
        if ($link.TargetPath -eq $exe) {
            $link.IconLocation = "$exe,0"
            $link.Save()
            Write-Host "Pinned:    $($file.Name) shows the new icon"
        }
    }
}
try { & "$env:SystemRoot\System32\ie4uinit.exe" -show } catch {}

if ($ContextMenu) {
    foreach ($entry in @(@("Directory\shell\CodeCanvas", "%1"), @("Directory\Background\shell\CodeCanvas", "%V"))) {
        $key = "HKCU:\Software\Classes\$($entry[0])"
        New-Item -Path "$key\command" -Force | Out-Null
        Set-ItemProperty -Path $key -Name "(Default)" -Value "Open with Code Canvas"
        Set-ItemProperty -Path $key -Name "Icon" -Value "`"$exe`""
        Set-ItemProperty -Path "$key\command" -Name "(Default)" -Value "`"$exe`" `"$($entry[1])`""
    }
    Write-Host "Context menu: 'Open with Code Canvas' added for folders"
}
Write-Host "Done. Open 'Code Canvas' from the Start Menu."
