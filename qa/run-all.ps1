param(
    [string]$Only = '*',
    [string]$Exe,
    [string]$Work
)

$scenarios = Get-ChildItem -Path (Join-Path $PSScriptRoot 'scenarios') -Recurse -Filter '*.scn' | Where-Object { $_.FullName -like "*$Only*" } | Sort-Object FullName
$failed = @()
foreach ($scenario in $scenarios) {
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'run.ps1'), '-Scenario', $scenario.FullName)
    if ($Exe) { $arguments += @('-Exe', $Exe) }
    if ($Work) { $arguments += @('-Work', (Join-Path $Work $scenario.BaseName)) }
    & powershell @arguments
    if ($LASTEXITCODE -ne 0) { $failed += $scenario.BaseName }
}
Write-Output ''
Write-Output "$($scenarios.Count - $failed.Count) of $($scenarios.Count) scenarios passed"
if ($failed.Count -gt 0) {
    Write-Output "failed: $($failed -join ', ')"
    exit 1
}
