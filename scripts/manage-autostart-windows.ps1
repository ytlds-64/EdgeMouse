[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateSet("Install", "Start", "Stop", "Status", "Uninstall")]
    [string]$Action,

    [Parameter(Position = 1)]
    [string]$ConfigPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$BinaryPath = Join-Path $ProjectRoot "target\release\edgemouse.exe"
$RunnerPath = Join-Path $PSScriptRoot "run-windows-with-log.ps1"
if ([string]::IsNullOrWhiteSpace($ConfigPath)) {
    $ConfigPath = Join-Path $ProjectRoot "edgemouse.toml"
} elseif (-not [System.IO.Path]::IsPathRooted($ConfigPath)) {
    $ConfigPath = Join-Path $ProjectRoot $ConfigPath
}
$ConfigPath = [System.IO.Path]::GetFullPath($ConfigPath)
$StartupDirectory = [Environment]::GetFolderPath("Startup")
$ShortcutPath = Join-Path $StartupDirectory "EdgeMouse.lnk"
$CurrentLog = Join-Path $ProjectRoot "windows-current.log"

function Test-EdgeMouseFiles {
    if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
        throw "EdgeMouse executable not found: $BinaryPath"
    }
    if (-not (Test-Path -LiteralPath $RunnerPath -PathType Leaf)) {
        throw "EdgeMouse log runner not found: $RunnerPath"
    }
    if (-not (Test-Path -LiteralPath $ConfigPath -PathType Leaf)) {
        throw "EdgeMouse configuration not found: $ConfigPath"
    }
}

function Test-EdgeMouseRunning {
    if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
        return $false
    }
    $StatusText = & $BinaryPath status 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to query EdgeMouse status: $StatusText"
    }
    return $StatusText -match "EdgeMouse is running"
}

function Show-EdgeMouseStatus {
    if (Test-Path -LiteralPath $ShortcutPath -PathType Leaf) {
        Write-Host "Login startup: installed"
    } else {
        Write-Host "Login startup: not installed"
    }
    if (Test-Path -LiteralPath $BinaryPath -PathType Leaf) {
        & $BinaryPath status
        if ($LASTEXITCODE -ne 0) {
            throw "EdgeMouse status command failed with exit code $LASTEXITCODE."
        }
    } else {
        Write-Host "EdgeMouse executable not found: $BinaryPath"
    }
    Write-Host "Log: $CurrentLog"
}

function Start-EdgeMouse {
    Test-EdgeMouseFiles
    if (Test-EdgeMouseRunning) {
        Write-Host "EdgeMouse is already running"
        return
    }

    $Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$RunnerPath`" -ConfigPath `"$ConfigPath`""
    Start-Process -FilePath "powershell.exe" `
        -ArgumentList $Arguments `
        -WorkingDirectory $ProjectRoot `
        -WindowStyle Minimized | Out-Null

    $Deadline = (Get-Date).AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 250
        if (Test-EdgeMouseRunning) {
            Write-Host "EdgeMouse started"
            return
        }
    } while ((Get-Date) -lt $Deadline)

    throw "EdgeMouse did not start within 10 seconds. Check: $CurrentLog"
}

function Stop-EdgeMouse {
    if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
        Write-Host "EdgeMouse is not installed"
        return
    }
    & $BinaryPath stop
    if ($LASTEXITCODE -ne 0) {
        throw "EdgeMouse stop command failed with exit code $LASTEXITCODE."
    }
}

switch ($Action) {
    "Install" {
        Test-EdgeMouseFiles
        & $BinaryPath version
        if ($LASTEXITCODE -ne 0) {
            throw "EdgeMouse version check failed."
        }
        & $BinaryPath check-config $ConfigPath
        if ($LASTEXITCODE -ne 0) {
            throw "EdgeMouse configuration check failed."
        }

        $Shell = New-Object -ComObject WScript.Shell
        $Shortcut = $Shell.CreateShortcut($ShortcutPath)
        $Shortcut.TargetPath = "powershell.exe"
        $Shortcut.Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$RunnerPath`" -ConfigPath `"$ConfigPath`""
        $Shortcut.WorkingDirectory = $ProjectRoot
        $Shortcut.WindowStyle = 7
        $Shortcut.Description = "Start EdgeMouse with logging after Windows sign-in"
        $Shortcut.Save()
        Write-Host "EdgeMouse login startup installed"
        Start-EdgeMouse
        Show-EdgeMouseStatus
    }
    "Start" {
        Start-EdgeMouse
        Show-EdgeMouseStatus
    }
    "Stop" {
        Stop-EdgeMouse
        Show-EdgeMouseStatus
    }
    "Status" {
        Show-EdgeMouseStatus
    }
    "Uninstall" {
        Stop-EdgeMouse
        if (Test-Path -LiteralPath $ShortcutPath -PathType Leaf) {
            Remove-Item -LiteralPath $ShortcutPath -Force
        }
        Write-Host "EdgeMouse login startup removed"
        Show-EdgeMouseStatus
    }
}
