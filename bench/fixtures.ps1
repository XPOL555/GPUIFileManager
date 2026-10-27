<#
.SYNOPSIS
    Generates the benchmark's test folders (idempotent), or removes them with -Clean.

.DESCRIPTION
    Under -Root (default: %TEMP%\fm-bench\fixtures, outside the repository):
      start\         10 files, the folder every run starts in
      flat-1k\       1 000 files     flat-10k\  10 000 files     flat-100k\  100 000 files
      images\        2 000 JPEGs (50 distinct pictures, 40 copies each: every file still
                     gets its own thumbnail)
      ue\            an Unreal-like project: Content, Source, Config, Plugins, Saved and the
                     heavy Intermediate and DerivedDataCache trees (~3 000 folders, ~40 000 files)
    A fixture whose `<name>.complete` marker exists is left alone, so reruns cost nothing.
#>
param(
    [string]$Root = (Join-Path $env:TEMP 'fm-bench\fixtures'),
    [switch]$Clean
)
$ErrorActionPreference = 'Stop'

if ($Clean) {
    if (Test-Path $Root) { Remove-Item -LiteralPath $Root -Recurse -Force }
    Write-Host "Removed $Root"
    return
}
New-Item -ItemType Directory -Force $Root | Out-Null

$extensions = 'txt', 'cpp', 'h', 'json', 'png', 'uasset', 'log', 'md', 'pdf', 'zip'

# Runs `$make` into a fresh folder unless the fixture is already complete.
function Fixture([string]$name, [scriptblock]$make) {
    $dir = Join-Path $Root $name
    # Beside the folder, not in it: it would be one more listed file.
    $marker = Join-Path $Root "$name.complete"
    if (Test-Path $marker) { Write-Host "$name ready"; return }
    if (Test-Path $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    New-Item -ItemType Directory $dir | Out-Null
    $watch = [Diagnostics.Stopwatch]::StartNew()
    & $make $dir
    Set-Content -LiteralPath $marker -Value (Get-Date -Format o)
    Write-Host ("{0} generated in {1:n1} s" -f $name, $watch.Elapsed.TotalSeconds)
}

# `count` small files with mixed extensions and sizes.
function Files([string]$dir, [int]$count, [string]$prefix = 'file') {
    $bytes = [byte[]]::new(4096)
    for ($i = 0; $i -lt $count; $i++) {
        $ext = $extensions[$i % $extensions.Count]
        $size = ($i * 37) % 4096
        $path = [IO.Path]::Combine($dir, ('{0}_{1:d6}.{2}' -f $prefix, $i, $ext))
        $stream = [IO.File]::Create($path)
        $stream.Write($bytes, 0, $size)
        $stream.Dispose()
    }
}

Fixture 'start' { param($d) Files $d 10 }
Fixture 'flat-1k' { param($d) Files $d 1000 }
Fixture 'flat-10k' { param($d) Files $d 10000 }
Fixture 'flat-100k' { param($d) Files $d 100000 }

Fixture 'images' {
    param($d)
    Add-Type -AssemblyName System.Drawing
    $random = [Random]::new(42)
    $jpeg = [Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object MimeType -eq 'image/jpeg'
    $params = [Drawing.Imaging.EncoderParameters]::new(1)
    $params.Param[0] = [Drawing.Imaging.EncoderParameter]::new([Drawing.Imaging.Encoder]::Quality, [long]85)
    $originals = for ($p = 0; $p -lt 50; $p++) {
        $bitmap = [Drawing.Bitmap]::new(1600, 1200)
        $g = [Drawing.Graphics]::FromImage($bitmap)
        $g.Clear([Drawing.Color]::FromArgb($random.Next(256), $random.Next(256), $random.Next(256)))
        for ($r = 0; $r -lt 40; $r++) {
            $brush = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(200, $random.Next(256), $random.Next(256), $random.Next(256)))
            $g.FillEllipse($brush, $random.Next(-200, 1500), $random.Next(-200, 1100), $random.Next(100, 700), $random.Next(100, 700))
            $brush.Dispose()
        }
        $g.Dispose()
        $path = Join-Path $d ('photo_{0:d4}.jpg' -f $p)
        $bitmap.Save($path, $jpeg, $params)
        $bitmap.Dispose()
        $path
    }
    for ($i = 50; $i -lt 2000; $i++) {
        [IO.File]::Copy($originals[$i % 50], (Join-Path $d ('photo_{0:d4}.jpg' -f $i)))
    }
}

Fixture 'ue' {
    param($d)
    $game = Join-Path $d 'MyGame'
    # A tree `depth` levels deep with `fanout` folders per level and `files` files in each.
    function Tree([string]$at, [int]$depth, [int]$fanout, [int]$files, [string]$names) {
        New-Item -ItemType Directory -Force $at | Out-Null
        Files $at $files 'asset'
        if ($depth -le 0) { return }
        for ($i = 0; $i -lt $fanout; $i++) {
            Tree (Join-Path $at ('{0}{1:d2}' -f $names, $i)) ($depth - 1) $fanout $files $names
        }
    }
    New-Item -ItemType Directory -Force $game | Out-Null
    Set-Content (Join-Path $game 'MyGame.uproject') '{ "FileVersion": 3, "EngineAssociation": "5.4" }'
    Tree (Join-Path $game 'Config') 0 0 20 'Config'
    Tree (Join-Path $game 'Content') 3 6 12 'Pack'                        # 259 folders
    Tree (Join-Path $game 'Source') 2 5 15 'Module'                        # 31 folders
    Tree (Join-Path $game 'Plugins') 2 4 10 'Plugin'                       # 21 folders
    Tree (Join-Path $game 'Saved') 1 4 25 'Logs'                           # 5 folders
    Tree (Join-Path $game 'Intermediate') 4 6 10 'Build'                   # 1 555 folders
    Tree (Join-Path $game 'DerivedDataCache') 3 8 15 'DDC'                 # 585 folders
    Tree (Join-Path $game 'Binaries') 1 2 30 'Win64'
}
