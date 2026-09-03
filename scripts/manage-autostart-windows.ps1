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
$HiddenRunnerPath = Join-Path $PSScriptRoot "run-windows-hidden.vbs"
$WscriptPath = Join-Path $env:SystemRoot "System32\wscript.exe"
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
    if (-not (Test-Path -LiteralPath $HiddenRunnerPath -PathType Leaf)) {
        throw "EdgeMouse hidden runner not found: $HiddenRunnerPath"
    }
    if (-not (Test-Path -LiteralPath $WscriptPath -PathType Leaf)) {
        throw "Windows Script Host was not found: $WscriptPath"
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

function Set-EdgeMouseStartupShortcut {
    $Shell = New-Object -ComObject WScript.Shell
    $Shortcut = $Shell.CreateShortcut($ShortcutPath)
    $Shortcut.TargetPath = $WscriptPath
    $Shortcut.Arguments = "`"$HiddenRunnerPath`" `"$ConfigPath`""
    $Shortcut.WorkingDirectory = $ProjectRoot
    $Shortcut.WindowStyle = 1
    $Shortcut.Description = "Start EdgeMouse silently with logging after Windows sign-in"
    $Shortcut.Save()
}

function Start-EdgeMouse {
    Test-EdgeMouseFiles
    if (Test-Path -LiteralPath $ShortcutPath -PathType Leaf) {
        Set-EdgeMouseStartupShortcut
    }
    if (Test-EdgeMouseRunning) {
        Write-Host "EdgeMouse is already running"
        return
    }

    $Arguments = "`"$HiddenRunnerPath`" `"$ConfigPath`""
    Start-Process -FilePath $WscriptPath `
        -ArgumentList $Arguments `
        -WorkingDirectory $ProjectRoot | Out-Null

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
        Stop-EdgeMouse

        Set-EdgeMouseStartupShortcut
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
