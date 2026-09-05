param([string]$Installer = (Join-Path $PSScriptRoot '..\..\build\release\typsmthng-windows-x64.exe'))
$ErrorActionPreference = 'Stop'
$Installer = (Resolve-Path $Installer).Path
$InstallRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('typsmthng-install-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force "$InstallRoot\bin", "$InstallRoot\lib", "$InstallRoot\personal" | Out-Null
$Sentinels = @('notes.txt', 'bin\unrelated-tool.txt', 'lib\unrelated-data.txt', 'personal\document.typ')
foreach ($File in $Sentinels) { Set-Content (Join-Path $InstallRoot $File) 'Preserve this unrelated user file.' }
function Run-Checked([string]$Binary, [string]$Arguments) {
  $Process = Start-Process -FilePath $Binary -ArgumentList $Arguments -PassThru
  if (!$Process.WaitForExit(60000)) { $Process.Kill(); throw "Timed out: $Binary" }
  if ($Process.ExitCode -ne 0) { throw "$Binary exited with $($Process.ExitCode)" }
}
function Assert-Sentinels {
  foreach ($File in $Sentinels) {
    $Path = Join-Path $InstallRoot $File
    if (!(Test-Path $Path) -or (Get-Content $Path -Raw).Trim() -ne 'Preserve this unrelated user file.') {
      throw "Installer modified unrelated file: $Path"
    }
  }
}
# NSIS requires /D to be the final argument, without quotes even for spaces.
Run-Checked $Installer "/S /D=$InstallRoot"
Assert-Sentinels
$Binary = Join-Path $InstallRoot 'bin\typsmthng.exe'
if (!(Test-Path $Binary)) { throw 'Installed application is missing' }
Run-Checked $Binary '--smoke-test'
# Exercise upgrade, then uninstall. _?= keeps the uninstaller in this process
# so WaitForExit observes the actual uninstall rather than a temporary launcher.
Run-Checked $Installer "/S /D=$InstallRoot"
Assert-Sentinels
Run-Checked (Join-Path $InstallRoot 'uninstall.exe') "/S _?=$InstallRoot"
Assert-Sentinels
if (Test-Path $Binary) { throw 'Uninstaller left the application binary behind' }
Write-Host 'Install, upgrade, smoke, and uninstall passed; unrelated files survived.'
Remove-Item $InstallRoot -Recurse -Force
