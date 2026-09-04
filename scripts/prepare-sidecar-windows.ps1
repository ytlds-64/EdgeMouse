[CmdletBinding()]
param(
    [string]$TargetTriple = ""
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($TargetTriple)) {
    $hostLine = rustc -vV | Select-String '^host:' | Select-Object -First 1
    if ($null -eq $hostLine) {
        throw "Unable to determine the Rust host target."
    }
    $TargetTriple = ($hostLine.Line -split ':', 2)[1].Trim()
}

& cargo build --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') --release -p edgemouse-agent --target $TargetTriple
if ($LASTEXITCODE -ne 0) {
    throw "EdgeMouse background service build failed with exit code $LASTEXITCODE."
}

$BinaryDirectory = Join-Path $ProjectRoot 'crates\edgemouse-desktop\binaries'
New-Item -ItemType Directory -Path $BinaryDirectory -Force | Out-Null
$Source = Join-Path $ProjectRoot "target\$TargetTriple\release\edgemouse.exe"
$Destination = Join-Path $BinaryDirectory "edgemouse-$TargetTriple.exe"
Copy-Item -LiteralPath $Source -Destination $Destination -Force
Write-Host "Prepared EdgeMouse sidecar: $Destination"
