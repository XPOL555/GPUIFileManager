<#
.SYNOPSIS
  Builds the Windows installers into dist/: an MSI (WiX 5) and/or an NSIS setup.exe.
.PARAMETER Format
  msi, nsis or all (default). "all" builds whichever the machine has the tool for
  and fails if none is available.
.PARAMETER SkipBuild
  Reuse target/release/filemanager.exe instead of running cargo build --release.
.PARAMETER Tag
  Suffix used in the file names (default: v<version from Cargo.toml>).
#>
param(
    [ValidateSet('all', 'msi', 'nsis')][string]$Format = 'all',
    [switch]$SkipBuild,
    [string]$Tag
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'
$exe = Join-Path $root 'target\release\filemanager.exe'

$cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)') { throw 'no version in Cargo.toml' }
$version = $Matches[1]
if (-not $Tag) { $Tag = "v$version" }
$name = "filemanager-$Tag-windows-x64"

if (-not $SkipBuild) {
    Push-Location $root
    try { cargo build --release --locked; if ($LASTEXITCODE) { throw 'cargo build failed' } } finally { Pop-Location }
}
if (-not (Test-Path $exe)) { throw "$exe not found: build first" }
New-Item -ItemType Directory -Force $dist | Out-Null

$icon = Join-Path $root 'assets\app.ico'
$built = @()

if ($Format -in 'all', 'msi') {
    $wix = Get-Command wix -ErrorAction SilentlyContinue
    if ($wix) {
        $out = Join-Path $dist "$name.msi"
        & wix build (Join-Path $PSScriptRoot 'filemanager.wxs') -arch x64 -o $out `
            -d "Version=$version" -d "ExePath=$exe" -d "IconPath=$icon"
        if ($LASTEXITCODE) { throw 'wix build failed' }
        $built += $out
    } elseif ($Format -eq 'msi') {
        throw 'wix not found: dotnet tool install --global wix --version 5.0.0'
    } else { Write-Warning 'wix not found, skipping the MSI' }
}

if ($Format -in 'all', 'nsis') {
    $makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
    if (-not $makensis) {
        $makensis = @("$env:ProgramFiles\NSIS\makensis.exe", "${env:ProgramFiles(x86)}\NSIS\makensis.exe") |
            Where-Object { Test-Path $_ } | Select-Object -First 1
    }
    if ($makensis) {
        $out = Join-Path $dist "$name-setup.exe"
        & $makensis /V2 "/DVERSION=$version" "/DEXE=$exe" "/DICON=$icon" `
            "/DLICENSE=$(Join-Path $root 'LICENSE')" "/DOUT=$out" (Join-Path $PSScriptRoot 'filemanager.nsi')
        if ($LASTEXITCODE) { throw 'makensis failed' }
        $built += $out
    } elseif ($Format -eq 'nsis') {
        throw 'makensis not found: install NSIS (winget install NSIS.NSIS)'
    } else { Write-Warning 'makensis not found, skipping the NSIS setup' }
}

if (-not $built) { throw 'no installer built' }
$built | ForEach-Object { Write-Host "built $_" }
