[CmdletBinding()]
param(
    [switch]$VerifyOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Invoke-CheckedStep {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Title,
        [Parameter(Mandatory = $true)]
        [scriptblock]$Action
    )

    Write-Host "==> $Title"
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Title failed with exit code $LASTEXITCODE."
    }
}

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $ProjectRoot

foreach ($RequiredCommand in @("cargo", "rustc", "rustup")) {
    if (-not (Get-Command $RequiredCommand -ErrorAction SilentlyContinue)) {
        throw "Missing $RequiredCommand. Install Rust from https://rustup.rs and rerun this script."
    }
}

Invoke-CheckedStep "Rust version" { rustc --version }
Invoke-CheckedStep "Cargo version" { cargo --version }
Invoke-CheckedStep "Rust components" { rustup component add rustfmt clippy }
Invoke-CheckedStep "Formatting" { cargo fmt --all -- --check }
Invoke-CheckedStep "Static analysis" { cargo clippy --workspace --all-targets -- -D warnings }
Invoke-CheckedStep "Tests" { cargo test --workspace }
Invoke-CheckedStep "Release build" { cargo build --release -p edgemouse-agent }

$BinaryPath = Join-Path $ProjectRoot "target\release\edgemouse.exe"
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "Release executable was not created at $BinaryPath."
}

Invoke-CheckedStep "Platform diagnostics" { & $BinaryPath doctor }

if ($VerifyOnly) {
    Write-Host ""
    Write-Host "Verification completed. Identity and configuration were not changed."
    exit 0
}

$IdentityDir = Join-Path $ProjectRoot "windows-identity"
$CertificatePath = Join-Path $IdentityDir "certificate.der"
$PrivateKeyPath = Join-Path $IdentityDir "private-key.der"
$HasCertificate = Test-Path -LiteralPath $CertificatePath -PathType Leaf
$HasPrivateKey = Test-Path -LiteralPath $PrivateKeyPath -PathType Leaf

if ($HasCertificate -and $HasPrivateKey) {
    Write-Host "==> Existing Windows identity kept"
} elseif ($HasCertificate -or $HasPrivateKey) {
    throw "The windows-identity directory is incomplete. Move it aside, then rerun this script."
} else {
    Invoke-CheckedStep "Generating Windows identity" { & $BinaryPath identity $IdentityDir }
}

$ConfigPath = Join-Path $ProjectRoot "edgemouse.toml"
if (Test-Path -LiteralPath $ConfigPath) {
    Write-Host "==> Existing edgemouse.toml kept"
} else {
    Write-Host "==> Creating edgemouse.toml from the Windows template"
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "examples\windows.toml") -Destination $ConfigPath
}

Write-Host ""
Write-Host "Windows preparation completed."
Write-Host "Executable : $BinaryPath"
Write-Host "Certificate: $CertificatePath"
Write-Host "Config     : $ConfigPath"
Write-Host ""
Write-Host "Manual steps still required:"
Write-Host "1. Send only windows-identity\certificate.der to the Mac. Never send private-key.der."
Write-Host "2. Save the Mac certificate as mac-certificate.der in this project folder."
Write-Host "3. Edit edgemouse.toml: peer.address, both screen sizes, and layout.peer_on."
Write-Host "4. Allow inbound UDP port 43891 in Windows Firewall."
Write-Host "5. Run: `"$BinaryPath`" check-config `"$ConfigPath`""
Write-Host "6. Run: `"$BinaryPath`" run `"$ConfigPath`""
