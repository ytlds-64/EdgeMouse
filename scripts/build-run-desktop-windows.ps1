[CmdletBinding()]
param(
    [string]$ConfigPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$DesktopBinary = Join-Path $ProjectRoot "target\release\edgemouse-desktop.exe"
$AgentBinary = Join-Path $ProjectRoot "target\release\edgemouse.exe"
$DesktopIcon = Join-Path $ProjectRoot "crates\edgemouse-desktop\icons\icon.ico"
Set-Location $ProjectRoot

if (-not (Get-Command "cargo" -ErrorAction SilentlyContinue)) {
    throw "Missing cargo. Install the stable Rust toolchain and rerun this script."
}

if ([string]::IsNullOrWhiteSpace($ConfigPath)) {
    $ConfigPath = Join-Path $ProjectRoot "edgemouse.toml"
} elseif (-not [System.IO.Path]::IsPathRooted($ConfigPath)) {
    $ConfigPath = Join-Path $ProjectRoot $ConfigPath
}

if (-not (Test-Path -LiteralPath $ConfigPath -PathType Leaf)) {
    throw "EdgeMouse configuration was not found: $ConfigPath"
}
if (-not (Test-Path -LiteralPath $DesktopIcon -PathType Leaf)) {
    throw "EdgeMouse Windows application icon was not found: $DesktopIcon. Pull the latest source and rerun this script."
}
$ResolvedConfigPath = (Resolve-Path -LiteralPath $ConfigPath).Path

Write-Host "==> Building EdgeMouse desktop application"
& cargo build --release -p edgemouse-desktop
if ($LASTEXITCODE -ne 0) {
    throw "Desktop release build failed with exit code $LASTEXITCODE."
}
if (-not (Test-Path -LiteralPath $DesktopBinary -PathType Leaf)) {
    throw "Desktop executable was not created: $DesktopBinary"
}

if (Test-Path -LiteralPath $AgentBinary -PathType Leaf) {
    Write-Host "==> Current EdgeMouse version"
    & $AgentBinary version
    if ($LASTEXITCODE -ne 0) {
        throw "Version check failed with exit code $LASTEXITCODE."
    }

    Write-Host "==> Validating current configuration"
    & $AgentBinary check-config $ResolvedConfigPath
    if ($LASTEXITCODE -ne 0) {
        throw "Configuration validation failed with exit code $LASTEXITCODE."
    }
}

Write-Host "==> Opening EdgeMouse desktop application"
$DesktopProcess = Start-Process -FilePath $DesktopBinary -ArgumentList @(
    "--config",
    "`"$ResolvedConfigPath`""
) -PassThru
Write-Host "EdgeMouse desktop started (process $($DesktopProcess.Id))."
Write-Host "The existing EdgeMouse background agent can keep running while this window is open."
