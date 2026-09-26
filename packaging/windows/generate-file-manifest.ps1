param(
  [string]$Stage = (Join-Path $PSScriptRoot '..\..\build\windows'),
  [string]$Output = (Join-Path $PSScriptRoot '..\..\build\windows-files.nsh')
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path $Stage).Path
$Lines = [System.Collections.Generic.List[string]]::new()
$Lines.Add('; Generated from the staged payload. Delete exact application files only.')
$Lines.Add('!macro RemoveInstalledFiles')
Get-ChildItem $Root -Recurse -File | Sort-Object FullName | ForEach-Object {
  $Relative = [System.IO.Path]::GetRelativePath($Root, $_.FullName).Replace('/', '\')
  if ($Relative -match '["$\r\n]' -or $Relative.StartsWith('..')) { throw "Unsafe installer path: $Relative" }
  $Lines.Add('  Delete "$INSTDIR\' + $Relative + '"')
}
# Nonrecursive RMDir removes empty application directories and preserves extra files.
Get-ChildItem $Root -Recurse -Directory | Sort-Object { $_.FullName.Length } -Descending | ForEach-Object {
  $Relative = [System.IO.Path]::GetRelativePath($Root, $_.FullName).Replace('/', '\')
  if ($Relative -match '["$\r\n]' -or $Relative.StartsWith('..')) { throw "Unsafe installer path: $Relative" }
  $Lines.Add('  RMDir "$INSTDIR\' + $Relative + '"')
}
$Lines.Add('!macroend')
[System.IO.File]::WriteAllLines([System.IO.Path]::GetFullPath($Output), $Lines)
