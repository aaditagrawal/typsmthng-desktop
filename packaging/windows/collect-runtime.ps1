$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path "$PSScriptRoot\..\..").Path
$Stage = Join-Path $RepoRoot "build\windows"
$Msys = if ($env:MSYSTEM_PREFIX) { $env:MSYSTEM_PREFIX } else { "C:\msys64\mingw64" }
$Release = Join-Path $RepoRoot "target\x86_64-pc-windows-gnu\release"
$Binary = Join-Path $Release "typsmthng.exe"
$Typst = Join-Path $Release "typst.exe"

if (!(Test-Path $Binary)) { throw "Missing GNU target application binary: $Binary" }
if (!(Test-Path $Typst)) { throw "Missing bundled Typst 0.15.1 binary: $Typst" }
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item $Binary $Stage
Copy-Item $Typst $Stage

# Walk the PE import graph with the same MinGW objdump that supplied GTK.
$Objdump = Join-Path $Msys "bin\objdump.exe"
$Queue = [System.Collections.Generic.Queue[string]]::new()
$Seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
$Queue.Enqueue($Binary)
$QueryLoaders = Join-Path $Msys "bin\gdk-pixbuf-query-loaders.exe"
if (!(Test-Path $QueryLoaders)) { throw "Missing gdk-pixbuf-query-loaders.exe" }
Copy-Item $QueryLoaders $Stage
$Queue.Enqueue($QueryLoaders)

$PixbufRoot = Join-Path $Msys "lib\gdk-pixbuf-2.0"
if (Test-Path $PixbufRoot) {
  Get-ChildItem $PixbufRoot -Filter "*.dll" -Recurse | ForEach-Object { $Queue.Enqueue($_.FullName) }
}

while ($Queue.Count -gt 0) {
  $Item = $Queue.Dequeue()
  foreach ($Line in (& $Objdump -p $Item)) {
    if ($Line -match "DLL Name:\s+(.+\.dll)\s*$") {
      $Name = $Matches[1]
      $Dependency = Join-Path $Msys "bin\$Name"
      if ((Test-Path $Dependency) -and $Seen.Add($Name)) {
        Copy-Item $Dependency $Stage
        $Queue.Enqueue($Dependency)
      }
    }
  }
}

foreach ($Relative in @("share\glib-2.0", "share\icons", "share\gtksourceview-5", "lib\gdk-pixbuf-2.0", "lib\gio\modules")) {
  $Source = Join-Path $Msys $Relative
  if (Test-Path $Source) {
    $Target = Join-Path $Stage $Relative
    New-Item -ItemType Directory -Force (Split-Path $Target) | Out-Null
    Copy-Item $Source $Target -Recurse -Force
  }
}
$LanguageTarget = Join-Path $Stage "share\typsmthng\language-specs"
New-Item -ItemType Directory -Force $LanguageTarget | Out-Null
Copy-Item (Join-Path $RepoRoot "native\gtk\data\language-specs\typst.lang") $LanguageTarget
Write-Host "Collected $($Seen.Count) runtime DLLs plus GTK data and Typst in $Stage"
