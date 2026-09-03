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
$AutostartManager = Join-Path $PSScriptRoot "manage-autostart-windows.ps1"
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
if (-not (Test-Path -LiteralPath $AutostartManager -PathType Leaf)) {
    throw "EdgeMouse background-service manager was not found: $AutostartManager"
}
$ResolvedConfigPath = (Resolve-Path -LiteralPath $ConfigPath).Path

$AgentWasRunning = $false
if (Test-Path -LiteralPath $AgentBinary -PathType Leaf) {
    $StatusText = & $AgentBinary status 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to query the current EdgeMouse background service: $StatusText"
    }
    $AgentWasRunning = $StatusText -match "EdgeMouse is running"
    if ($AgentWasRunning) {
        Write-Host "==> Stopping the current EdgeMouse background service for the upgrade"
        & $AgentBinary stop
        if ($LASTEXITCODE -ne 0) {
            throw "Could not stop the current EdgeMouse background service."
        }
    }
}

$DesktopProcesses = @(Get-Process -Name "edgemouse-desktop" -ErrorAction SilentlyContinue)
if ($DesktopProcesses.Count -gt 0) {
    Write-Host "==> Closing the previous EdgeMouse desktop window"
    $DesktopProcesses | Stop-Process -Force
}

$BuildSucceeded = $false
try {
    Write-Host "==> Building the EdgeMouse background service and desktop application"
    & cargo build --release -p edgemouse-agent -p edgemouse-desktop
    if ($LASTEXITCODE -ne 0) {
        throw "EdgeMouse release build failed with exit code $LASTEXITCODE."
    }
    if (-not (Test-Path -LiteralPath $AgentBinary -PathType Leaf)) {
        throw "Background-service executable was not created: $AgentBinary"
    }
    if (-not (Test-Path -LiteralPath $DesktopBinary -PathType Leaf)) {
        throw "Desktop executable was not created: $DesktopBinary"
    }
    $BuildSucceeded = $true
} finally {
    if (-not $BuildSucceeded -and $AgentWasRunning -and (Test-Path -LiteralPath $AgentBinary -PathType Leaf)) {
        Write-Host "Build failed; attempting to restore the previous background service..." -ForegroundColor Yellow
        try {
            & $AutostartManager Start $ResolvedConfigPath
        } catch {
            Write-Host "The previous background service could not be restored automatically: $_" -ForegroundColor Yellow
        }
    }
}

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

Write-Host "==> Starting the updated EdgeMouse background service"
& $AutostartManager Start $ResolvedConfigPath

Write-Host "==> Opening EdgeMouse desktop application"
$DesktopProcess = Start-Process -FilePath $DesktopBinary -ArgumentList @(
    "--config",
    "`"$ResolvedConfigPath`""
) -PassThru
Write-Host "EdgeMouse desktop started (process $($DesktopProcess.Id))."
Write-Host "The updated background service is running and supplying live status to this window."
