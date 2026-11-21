# Silent install, check and uninstall of the Windows installer (release
# workflow, Windows runner).
#
#   packaging/windows/test-installer.ps1 -Installer setup.exe [-Mode CurrentUser|AllUsers]
#
# AllUsers needs an administrator (GitHub runners are).
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [ValidateSet('CurrentUser', 'AllUsers')][string]$Mode = 'CurrentUser'
)
$ErrorActionPreference = 'Stop'
$Installer = (Resolve-Path $Installer).Path

$dir = Join-Path $env:RUNNER_TEMP "PwTool test $Mode"
if (-not $env:RUNNER_TEMP) { $dir = Join-Path $env:TEMP "PwTool test $Mode" }
if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
if ($Mode -eq 'AllUsers') {
    $root = 'HKLM:'
    $menu = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Password Tool'
} else {
    $root = 'HKCU:'
    $menu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Password Tool'
}
$uninstallKey = "$root\Software\Microsoft\Windows\CurrentVersion\Uninstall\PwTool"
$settings = Join-Path $env:APPDATA 'PwTool'

# /D= must be the last argument and is not quoted
$p = Start-Process -FilePath $Installer -ArgumentList @('/S', "/$Mode", "/D=$dir") -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "installer exit code $($p.ExitCode)" }

foreach ($f in 'pwtool.exe', 'uninstall.exe', 'LICENSE.txt') {
    if (-not (Test-Path (Join-Path $dir $f))) { throw "missing $f" }
}
if (Test-Path (Join-Path $dir 'PwTool.ini')) { throw 'PwTool.ini in the installation directory (portable mode)' }
if (-not (Test-Path (Join-Path $menu 'Password Tool.lnk'))) { throw "no Start menu shortcut in $menu" }
$key = Get-ItemProperty $uninstallKey
if ($key.InstallLocation -ne $dir) { throw "InstallLocation $($key.InstallLocation)" }
if ($key.UninstallString -notmatch "/$Mode") { throw "UninstallString $($key.UninstallString)" }
Write-Output "installed $($key.DisplayName) $($key.DisplayVersion) ($Mode) into $dir"

# the installed program works (console mode) and keeps its settings outside
$out = & (Join-Path $dir 'pwtool.exe') --gen 3 | Out-String
if (($out -split "`n" | Where-Object { $_ -match '^\S{16}\s*$' }).Count -ne 3) { throw "console mode: $out" }

# settings of the user survive a silent uninstall
New-Item -ItemType Directory -Force $settings | Out-Null
$ini = Join-Path $settings 'PwTool.ini'
$hadIni = Test-Path $ini
if (-not $hadIni) { Set-Content $ini "[Main]`r`n" }

# _?= runs the uninstaller in place, so Start-Process can wait for it; it
# cannot delete itself then
$p = Start-Process -FilePath (Join-Path $dir 'uninstall.exe') -ArgumentList @('/S', "/$Mode", "_?=$dir") -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "uninstaller exit code $($p.ExitCode)" }
foreach ($f in 'pwtool.exe', 'LICENSE.txt') {
    if (Test-Path (Join-Path $dir $f)) { throw "$f left after uninstall" }
}
if (Test-Path $uninstallKey) { throw 'uninstall registry key left' }
if (Test-Path $menu) { throw 'Start menu folder left' }
if (-not (Test-Path $ini)) { throw 'a silent uninstall deleted the settings' }
if (-not $hadIni) { Remove-Item $ini }
Remove-Item -Recurse -Force $dir
Write-Output "uninstalled ($Mode)"
