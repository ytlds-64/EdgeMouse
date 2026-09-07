[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$Version
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') {
    throw 'This installer smoke test is restricted to disposable GitHub Actions runners.'
}
$Installer = (Resolve-Path -LiteralPath $Installer).Path
$testDirectory = Join-Path $env:RUNNER_TEMP ('edgemouse-installer-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $testDirectory | Out-Null

Add-Type @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class InstallerWindows {
  private delegate bool Callback(IntPtr hwnd, IntPtr arg);
  [DllImport("user32.dll")] private static extern bool EnumWindows(Callback cb, IntPtr arg);
  [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr hwnd, Callback cb, IntPtr arg);
  [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
  public static IntPtr Find(int pid) {
    IntPtr result = IntPtr.Zero;
    EnumWindows((hwnd, unused) => { uint id; GetWindowThreadProcessId(hwnd, out id);
      if (id == pid && Text(hwnd).Length > 0) { result = hwnd; return false; } return true;
    }, IntPtr.Zero);
    return result;
  }
  public static string Text(IntPtr hwnd) { var text = new StringBuilder(8192); GetWindowText(hwnd, text, text.Capacity); return text.ToString(); }
  public static string Contents(IntPtr hwnd) {
    var texts = new List<string>(); texts.Add(Text(hwnd));
    EnumChildWindows(hwnd, (child, unused) => { texts.Add(Text(child)); return true; }, IntPtr.Zero);
    return String.Join("\n", texts);
  }
}
'@

function Wait-InstallerText($Process, [string]$Pattern) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $contents = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        $window = [InstallerWindows]::Find($Process.Id)
        if ($window -ne [IntPtr]::Zero) {
            $contents = [InstallerWindows]::Contents($window)
            if ($contents -match $Pattern) { return $window }
        }
        if ($Process.HasExited) { throw "Installer exited before showing $Pattern" }
        Start-Sleep -Milliseconds 150
    }
    throw "Installer did not show $Pattern. Visible strings: $contents"
}

# Simulate the English preference saved by older releases. The default selector
# must still appear with Chinese selected, followed by a Chinese welcome page.
$languageKey = 'HKCU:\Software\EdgeMouse contributors\EdgeMouse'
New-Item -Path $languageKey -Force | Out-Null
Set-ItemProperty -Path $languageKey -Name 'Installer Language' -Value '1033'
$installerProcess = Start-Process -FilePath $Installer -PassThru
try {
    $dialog = Wait-InstallerText $installerProcess '安装语言 / Setup language'
    $contents = [InstallerWindows]::Contents($dialog)
    if ($contents -notmatch '简体中文|中文（简体）|中文\(简体\)') {
        throw "Chinese was not selected by default: $contents"
    }
    [void][InstallerWindows]::SendMessage($dialog, 0x0111, [IntPtr]1, [IntPtr]::Zero)
    [void](Wait-InstallerText $installerProcess '欢迎')
    Write-Host 'Chinese default welcome page verified with legacy English preference.'
} finally {
    if (-not $installerProcess.HasExited) { Stop-Process -Id $installerProcess.Id -Force }
}
$installerProcess = Start-Process -FilePath $Installer -ArgumentList '/LANG=1033' -PassThru
try {
    [void](Wait-InstallerText $installerProcess 'Welcome to EdgeMouse Setup')
    Write-Host 'Explicit English welcome page verified.'
} finally {
    if (-not $installerProcess.HasExited) { Stop-Process -Id $installerProcess.Id -Force }
}

function Run-SilentInstaller {
    $process = Start-Process -FilePath $Installer -ArgumentList "/S /LANG=2052 /D=$testDirectory" -PassThru
    if (-not $process.WaitForExit(60000)) {
        Stop-Process -Id $process.Id -Force
        throw 'Silent installer timed out.'
    }
    return $process.ExitCode
}
if ((Run-SilentInstaller) -ne 0) { throw 'Fresh silent installation failed.' }
$agentPath = Join-Path $testDirectory 'edgemouse.exe'
$desktopPath = Join-Path $testDirectory 'edgemouse-desktop.exe'
$agentVersion = (& $agentPath version | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $agentVersion -ne "edgemouse $Version") {
    throw "Packaged background version mismatch: $agentVersion"
}
$desktopVersion = (Get-Item -LiteralPath $desktopPath).VersionInfo.ProductVersion
if ($desktopVersion -ne $Version) { throw "Packaged desktop version mismatch: $desktopVersion" }
Write-Host "Both installed components are $Version."

# Hold the sidecar against writes. The next install must fail before proceeding,
# not offer to skip this file and report a successful mixed-version update.
$lockedFile = [System.IO.File]::Open($agentPath, 'Open', 'Read', 'Read')
try {
    if ((Run-SilentInstaller) -eq 0) { throw 'Installer incorrectly succeeded while the sidecar was locked.' }
    Write-Host 'Locked-sidecar installation correctly refused.'
} finally {
    $lockedFile.Dispose()
}
if ((Run-SilentInstaller) -ne 0) { throw 'Repair installation failed after the lock was released.' }
Write-Host 'Repair installation succeeded after releasing the sidecar.'
