[CmdletBinding()]
param(
    [string]$ConfigPath,
    [ValidateRange(1, 10)]
    [int]$UpdateAttempts = 3,
    [switch]$SkipUpdate
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$BinaryPath = Join-Path $ProjectRoot "target\release\edgemouse.exe"
$RunScript = Join-Path $PSScriptRoot "run-windows-with-log.ps1"
Set-Location $ProjectRoot

foreach ($RequiredCommand in @("git", "cargo")) {
    if (-not (Get-Command $RequiredCommand -ErrorAction SilentlyContinue)) {
        throw "Missing $RequiredCommand. Install it and rerun this script."
    }
}

& git rev-parse --is-inside-work-tree 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "$ProjectRoot is not a Git working tree."
}

$CurrentBranch = (& git branch --show-current).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Could not determine the current Git branch."
}
if ($CurrentBranch -ne "main") {
    throw "Expected Git branch main, but the current branch is $CurrentBranch."
}

$TrackedChanges = @(& git status --porcelain --untracked-files=no)
if ($LASTEXITCODE -ne 0) {
    throw "Could not inspect the Git working tree."
}
if ($TrackedChanges.Count -ne 0) {
    Write-Host "Tracked files contain local changes:" -ForegroundColor Yellow
    $TrackedChanges | ForEach-Object { Write-Host $_ -ForegroundColor Yellow }
    throw "Commit or discard those tracked changes before updating. Ignored configuration, certificates, and logs do not block this script."
}

if ($SkipUpdate) {
    Write-Host "==> Skipping GitHub update; building the currently available local source" -ForegroundColor Yellow
} else {
    $GitExitCode = 1
    for ($Attempt = 1; $Attempt -le $UpdateAttempts; $Attempt++) {
        Write-Host "==> Updating source from origin/main (attempt $Attempt of $UpdateAttempts)"
        & git pull --ff-only origin main
        $GitExitCode = $LASTEXITCODE
        if ($GitExitCode -eq 0) {
            break
        }
        if ($Attempt -lt $UpdateAttempts) {
            $DelaySeconds = [Math]::Min(15, $Attempt * 5)
            Write-Host "GitHub is currently unreachable; retrying in $DelaySeconds second(s)..." -ForegroundColor Yellow
            Start-Sleep -Seconds $DelaySeconds
        }
    }
    if ($GitExitCode -ne 0) {
        Write-Host ""
        Write-Host "Windows could not connect to github.com on TCP port 443. No tracked source files were changed." -ForegroundColor Yellow
        Write-Host "To start the already-built local version without updating, run:" -ForegroundColor Yellow
        Write-Host "  powershell -ExecutionPolicy Bypass -File .\scripts\run-windows-with-log.ps1" -ForegroundColor Yellow
        throw "Git update failed after $UpdateAttempts attempt(s). Check the Windows network or proxy settings, then retry."
    }
}

Write-Host "==> Building EdgeMouse release"
& cargo build --release -p edgemouse-agent
if ($LASTEXITCODE -ne 0) {
    throw "Release build failed with exit code $LASTEXITCODE."
}
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "Release executable was not created: $BinaryPath"
}

Write-Host "==> Current software version"
& $BinaryPath version
if ($LASTEXITCODE -ne 0) {
    throw "Version check failed with exit code $LASTEXITCODE."
}

Write-Host "==> Starting EdgeMouse with automatic logging"
if ([string]::IsNullOrWhiteSpace($ConfigPath)) {
    & $RunScript
} else {
    & $RunScript -ConfigPath $ConfigPath
}
exit $LASTEXITCODE
