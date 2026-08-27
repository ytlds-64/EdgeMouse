[CmdletBinding()]
param(
    [string]$ConfigPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$BinaryPath = Join-Path $ProjectRoot "target\release\edgemouse.exe"
if ([string]::IsNullOrWhiteSpace($ConfigPath)) {
    $ConfigPath = Join-Path $ProjectRoot "edgemouse.toml"
} elseif (-not [System.IO.Path]::IsPathRooted($ConfigPath)) {
    $ConfigPath = Join-Path $ProjectRoot $ConfigPath
}
$LogDirectory = Join-Path $ProjectRoot "logs"
$CurrentLog = Join-Path $ProjectRoot "windows-current.log"
$Timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$ArchiveLog = Join-Path $LogDirectory "windows-$Timestamp.log"

if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "EdgeMouse executable not found: $BinaryPath"
}
if (-not (Test-Path -LiteralPath $ConfigPath -PathType Leaf)) {
    throw "EdgeMouse configuration not found: $ConfigPath"
}

New-Item -ItemType Directory -Path $LogDirectory -Force | Out-Null
Set-Content -LiteralPath $CurrentLog -Value "" -Encoding UTF8
Set-Content -LiteralPath $ArchiveLog -Value "" -Encoding UTF8

function Write-LogLine {
    param([AllowEmptyString()][string]$Line)

    Add-Content -LiteralPath $CurrentLog -Value $Line -Encoding UTF8
    Add-Content -LiteralPath $ArchiveLog -Value $Line -Encoding UTF8
    Write-Host $Line
}

function Invoke-LoggedEdgeMouse {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    # Windows PowerShell 5 wraps a native program's redirected stderr as
    # NativeCommandError. EdgeMouse intentionally uses stderr for recoverable
    # network diagnostics, so do not let ErrorActionPreference=Stop terminate
    # the logging pipeline while the native process is running.
    $PreviousErrorActionPreference = $ErrorActionPreference
    $NativeExitCode = 1
    try {
        $ErrorActionPreference = "Continue"
        & $BinaryPath @Arguments 2>&1 | ForEach-Object {
            Write-LogLine -Line ([string]$_)
        }
        $NativeExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $PreviousErrorActionPreference
    }
    return $NativeExitCode
}

Write-LogLine "EdgeMouse Windows session"
Write-LogLine "Started : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss K')"
Write-LogLine "Config  : $ConfigPath"
Write-LogLine "Current : $CurrentLog"
Write-LogLine "Archive : $ArchiveLog"

$ExitCode = Invoke-LoggedEdgeMouse -Arguments @("version")
if ($ExitCode -ne 0) {
    Write-LogLine "Version check failed; EdgeMouse was not started."
    exit $ExitCode
}

Write-LogLine ""
$ExitCode = Invoke-LoggedEdgeMouse -Arguments @("check-config", $ConfigPath)
if ($ExitCode -ne 0) {
    Write-LogLine "Configuration check failed; EdgeMouse was not started."
    exit $ExitCode
}

Write-LogLine ""
$ExitCode = Invoke-LoggedEdgeMouse -Arguments @("run", $ConfigPath)

Write-Host ""
Write-Host "Latest log : $CurrentLog"
Write-Host "Archive log: $ArchiveLog"
exit $ExitCode
