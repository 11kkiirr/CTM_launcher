[CmdletBinding()]
param(
    [string]$InstallDir = $PSScriptRoot,
    [switch]$NoStartMenu
)

$ErrorActionPreference = 'Stop'
$InstallDir = (Resolve-Path -LiteralPath $InstallDir).Path
$Executable = Join-Path $InstallDir 'ctmlauncher.exe'

if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "ctmlauncher.exe was not found in $InstallDir"
}

$Shell = New-Object -ComObject WScript.Shell

function New-LauncherShortcut([string]$Path) {
    $Shortcut = $Shell.CreateShortcut($Path)
    $Shortcut.TargetPath = $Executable
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.IconLocation = "$Executable,0"
    $Shortcut.Description = 'CTMLauncher - terminal Minecraft launcher'
    $Shortcut.Save()
}

$Desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
New-LauncherShortcut (Join-Path $Desktop 'CTMLauncher.lnk')

if (-not $NoStartMenu) {
    $Programs = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
    $StartMenuDir = Join-Path $Programs 'CTMLauncher'
    New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
    New-LauncherShortcut (Join-Path $StartMenuDir 'CTMLauncher.lnk')
}

Write-Host 'CTMLauncher shortcuts created.'
