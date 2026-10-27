<#
.SYNOPSIS
    Runs the benchmark and writes bench/results.json (plus raw logs in bench/raw).

.DESCRIPTION
    1. Generates the fixtures (fixtures.ps1, idempotent) under -Scratch.
    2. Builds the release exe into target/claude-test (so a running copy is left alone).
    3. FileManager, scripted: -Runs fresh processes (after one warm-up run that is thrown
       away) with FM_BENCH / FM_BENCH_STEPS set. The app logs, as JSON lines, when each
       listing's frame is painted and its own memory counters at chosen points.
    4. Every app, the same way: a fresh process opened on a folder, left until its memory
       stops growing, then read from outside (Get-Process). Apps already running are
       skipped: a new launch would hand the folder to the running instance.
    Every process started here is closed at the end of its run and killed if it is still
    there, frozen or not. They run with APPDATA / LOCALAPPDATA pointed at a scratch folder;
    apps that find their settings another way get their settings files put back afterwards.
#>
param(
    [int]$Runs = 5,
    [int]$CompareRuns = 3,
    [string]$Scratch = (Join-Path $env:TEMP 'fm-bench'),
    [switch]$SkipBuild,
    [switch]$SkipOthers
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot
$fixtures = Join-Path $Scratch 'fixtures'
$rawDir = Join-Path $PSScriptRoot 'raw'
New-Item -ItemType Directory -Force $Scratch, $rawDir | Out-Null

& (Join-Path $PSScriptRoot 'fixtures.ps1') -Root $fixtures

$target = Join-Path $repo 'target\claude-test'
$exe = Join-Path $target 'release\filemanager.exe'
if (-not $SkipBuild) {
    $env:CARGO_TARGET_DIR = $target
    try { cargo build --release --manifest-path (Join-Path $repo 'Cargo.toml') } finally { Remove-Item Env:CARGO_TARGET_DIR }
    if ($LASTEXITCODE) { throw 'cargo build failed' }
}

# ---- helpers -----------------------------------------------------------------

function Stats([double[]]$values) {
    $sorted = @($values | Sort-Object)
    $n = $sorted.Count
    $median = if ($n % 2) { $sorted[($n - 1) / 2] } else { ($sorted[$n / 2 - 1] + $sorted[$n / 2]) / 2 }
    [ordered]@{ median = [math]::Round($median, 1); min = [math]::Round($sorted[0], 1); max = [math]::Round($sorted[-1], 1); runs = $n }
}

# Starts `$path` with a scratch APPDATA / LOCALAPPDATA and extra environment variables.
function Launch([string]$path, [string]$arguments, [hashtable]$environment = @{}) {
    $profileDir = Join-Path $Scratch 'profile'
    if (Test-Path $profileDir) { Remove-Item -LiteralPath $profileDir -Recurse -Force }
    $roaming = Join-Path $profileDir 'Roaming'
    $local = Join-Path $profileDir 'Local'
    New-Item -ItemType Directory -Force $roaming, $local | Out-Null
    $info = [Diagnostics.ProcessStartInfo]::new($path, $arguments)
    $info.UseShellExecute = $false
    $info.Environment['APPDATA'] = $roaming
    $info.Environment['LOCALAPPDATA'] = $local
    foreach ($key in $environment.Keys) { $info.Environment[$key] = $environment[$key] }
    [Diagnostics.Process]::Start($info)
}

function Stop-Launched([int[]]$ids) {
    foreach ($id in $ids) { try { Stop-Process -Id $id -Force -ErrorAction Stop } catch {} }
}

# ---- FileManager, scripted ----------------------------------------------------

$navigation = @(Get-ChildItem -LiteralPath (Join-Path $fixtures 'ue\MyGame') -Directory -Recurse | Sort-Object FullName)
$step = [math]::Max(1, [math]::Floor($navigation.Count / 50))
$navigation = @(for ($i = 0; $i -lt 50; $i++) { $navigation[$i * $step].FullName })

$steps = Join-Path $Scratch 'steps.txt'
@(
    'wait 2000', 'mem idle'
    "open $fixtures\flat-1k", 'wait 1500', 'mem after-1k'
    "open $fixtures\flat-10k", 'wait 1500', 'mem after-10k'
    "open $fixtures\flat-100k", 'wait 1500', 'mem after-100k'
    'mem nav-start'
    $navigation | ForEach-Object { "open $_" }
    'mem nav-end', 'wait 1500', 'mem after-nav'
    "open $fixtures\images", 'view xl', 'wait 3000', 'mem images-xl-top'
    'scroll', 'wait 5000', 'mem images-xl-scrolled'
    'quit'
) | Set-Content -LiteralPath $steps

$logs = for ($run = 0; $run -le $Runs; $run++) {
    $log = Join-Path $rawDir ("filemanager-run{0}.jsonl" -f $run)
    if (Test-Path $log) { Remove-Item -LiteralPath $log }
    $process = Launch $exe "`"$fixtures\start`"" @{ FM_BENCH = $log; FM_BENCH_STEPS = $steps }
    try {
        if (-not $process.WaitForExit(240000)) { Write-Warning "run $run timed out" }
    } finally { Stop-Launched $process.Id }
    Write-Host ("FileManager run {0}{1} done" -f $run, $(if ($run -eq 0) { ' (warm-up)' } else { '' }))
    # Run 0 warms the disk cache and the shell's thumbnail cache; it is not counted.
    if ($run -gt 0) { , @(Get-Content -LiteralPath $log | ConvertFrom-Json) }
}

$scenarios = [Collections.Generic.List[object]]::new()
function Scenario($id, $name, $unit, $notes, $results) {
    $scenarios.Add([ordered]@{ id = $id; name = $name; unit = $unit; lower_is_better = $true; notes = $notes; results = @($results) })
}
function Result($app, [double[]]$values) { $r = [ordered]@{ app = $app }; (Stats $values).GetEnumerator() | ForEach-Object { $r[$_.Key] = $_.Value }; $r }

$listed = { param($events, $folder) $events | Where-Object { $_.event -eq 'listed' -and $_.path -eq (Join-Path $fixtures $folder) } | Select-Object -First 1 }
$mem = { param($events, $label) $events | Where-Object { $_.event -eq 'mem' -and $_.label -eq $label } | Select-Object -First 1 }

Scenario 'startup' 'Start up, first listing painted (10-file folder)' 'ms' `
    'From the OS creating the process to the end of painting the first frame that shows the listing (FM_BENCH hook).' `
    (Result 'FileManager' @($logs | ForEach-Object { ($_ | Where-Object event -eq 'listed' | Select-Object -First 1).t_ms }))
foreach ($size in '1k', '10k', '100k') {
    Scenario "list-$size" "Open a folder with $size files" 'ms' `
        'Navigation in a running window: from the request to the end of painting the frame that shows the listed, sorted rows (details view). Files were read recently, so the OS cache is warm.' `
        (Result 'FileManager' @($logs | ForEach-Object { (& $listed $_ "flat-$size").paint_ms }))
}
Scenario 'navigate-50' 'Navigate through 50 folders of an Unreal-like project, one after the other' 'ms' `
    'Total time for 50 navigations, each one starting as soon as the previous listing was painted (folders hold 10 to 31 items). Per navigation the listing is ready in 10-13 ms; the rest is waiting for the frame, which varied between runs (17 or 53 ms per folder; bench/raw has the per-folder events).' `
    (Result 'FileManager' @($logs | ForEach-Object { (& $mem $_ 'nav-end').t_ms - (& $mem $_ 'nav-start').t_ms }))

$states = [ordered]@{
    'idle' = 'Idle, 10-file folder open'
    'after-100k' = 'After opening the 100k-file folder'
    'after-nav' = 'After opening 1k, 10k, 100k files and navigating 50 folders'
    'images-xl-scrolled' = 'Image folder (2 000 JPEGs) in XL icons, scrolled to the end'
}
foreach ($state in $states.Keys) {
    foreach ($counter in @(@{ key = 'private_mb'; id = 'private'; label = 'private bytes' }, @{ key = 'working_set_mb'; id = 'ws'; label = 'working set' })) {
        Scenario "memory-$state-$($counter.id)" "Memory ($($counter.label)): $($states[$state])" 'MB' `
            'Read by the app itself (GetProcessMemoryInfo, the counters Get-Process reports) at that point of the scripted session.' `
            (Result 'FileManager' @($logs | ForEach-Object { (& $mem $_ $state).($counter.key) }))
    }
}

# ---- every app, a fresh process on one folder, memory read from outside ----------

$apps = @(
    @{ name = 'FileManager'; exe = $exe; process = 'filemanager'; args = { param($p) "`"$p`"" }; state = @() }
    @{ name = 'Windows Explorer'; exe = "$env:WINDIR\explorer.exe"; process = 'explorer'; args = { param($p) "/separate,`"$p`"" }; state = @() }
    # It finds its settings through the known-folder API, not APPDATA: what it saves on the
    # way out (last session) is put back after its runs.
    @{ name = 'File Pilot'; exe = "$env:LOCALAPPDATA\Voidstar\FilePilot\FPilot.exe"; process = 'FPilot'; args = { param($p) "`"$p`"" }
        state = @("$env:LOCALAPPDATA\Voidstar\FilePilot", "$env:APPDATA\Voidstar\FilePilot") }
    # OneCommander is left out: started with a folder on its command line it opens a window
    # with no folder pane (3.108), so its memory says nothing about the folder.
)

# Copies the top-level settings files (*.json, *.ini, *.xml, *.db) of `$dirs` away, or back.
function Save-State([string[]]$dirs, [string]$to) {
    if (Test-Path $to) { Remove-Item -LiteralPath $to -Recurse -Force }
    for ($i = 0; $i -lt $dirs.Count; $i++) {
        if (-not (Test-Path $dirs[$i])) { continue }
        $copy = New-Item -ItemType Directory -Force (Join-Path $to $i)
        Get-ChildItem -LiteralPath $dirs[$i] -File -Force | Where-Object Extension -in '.json', '.ini', '.xml', '.db' |
            Copy-Item -Destination $copy
    }
}
# Settings files the runs created (OneCommander leaves a backup per launch) are removed.
function Restore-State([string[]]$dirs, [string]$from) {
    for ($i = 0; $i -lt $dirs.Count; $i++) {
        $copy = Join-Path $from $i
        if (-not (Test-Path $copy)) { continue }
        $saved = @(Get-ChildItem -LiteralPath $copy -File -Force | ForEach-Object Name)
        Get-ChildItem -LiteralPath $dirs[$i] -File -Force | Where-Object { $_.Extension -in '.json', '.ini', '.xml', '.db' -and $saved -notcontains $_.Name } |
            Remove-Item -Force
        Get-ChildItem -LiteralPath $copy -File -Force | Copy-Item -Destination $dirs[$i] -Force
    }
}

# Closes the windows the way a user would (apps that are killed run recovery checks on
# their next start), then kills whatever is left.
function Close-Launched([int[]]$ids) {
    foreach ($id in $ids) { try { (Get-Process -Id $id -ErrorAction Stop).CloseMainWindow() | Out-Null } catch {} }
    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $ids -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
    Stop-Launched $ids
}
$folders = [ordered]@{ 'start' = 'a 10-file folder'; 'flat-100k' = 'the 100k-file folder'; 'images' = 'the image folder (2 000 JPEGs), default view' }
$skipped = @()
$external = @{}
$rawExternal = [Collections.Generic.List[object]]::new()

if (-not $SkipOthers) {
    foreach ($app in $apps) {
        if (-not (Test-Path $app.exe)) { $skipped += "$($app.name): not installed"; continue }
        if ($app.process -ne 'explorer' -and (Get-Process $app.process -ErrorAction SilentlyContinue)) {
            $skipped += "$($app.name): already running"; continue
        }
        $stateCopy = Join-Path $Scratch "state\$($app.process)"
        Save-State $app.state $stateCopy
        try {
        foreach ($folder in $folders.Keys) {
            # Run 0 warms the caches (disk, thumbnails, the app's own) and is not counted.
            for ($run = 0; $run -le $CompareRuns; $run++) {
                $before = @(Get-Process $app.process -ErrorAction SilentlyContinue | ForEach-Object Id)
                $launched = Launch $app.exe (& $app.args (Join-Path $fixtures $folder))
                $started = [Diagnostics.Stopwatch]::StartNew()
                $samples = [Collections.Generic.List[double]]::new()
                $ids = @()
                $shown = $stable = $false
                try {
                    # Once its window is up, until private bytes move by less than 2% over 3 s
                    # (at least 4 s from the launch, at most 45 s).
                    do {
                        Start-Sleep -Milliseconds 500
                        $procs = @(Get-Process $app.process -ErrorAction SilentlyContinue | Where-Object { $before -notcontains $_.Id })
                        $ids = @($procs | ForEach-Object Id) + $launched.Id | Select-Object -Unique
                        $shown = $shown -or [bool]($procs | Where-Object { $_.MainWindowHandle -ne 0 })
                        if ($shown) {
                            $private = ($procs | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB
                            $samples.Add($private)
                            $window = if ($samples.Count -ge 6) { $samples.GetRange($samples.Count - 6, 6) } else { $null }
                            $stable = $window -and ($started.Elapsed.TotalSeconds -ge 4) -and
                                (($window | Measure-Object -Maximum).Maximum - ($window | Measure-Object -Minimum).Minimum) -le [math]::Max(1, 0.02 * $private)
                        }
                    } until ($stable -or $started.Elapsed.TotalSeconds -ge 45)
                    $procs = @(Get-Process -Id $ids -ErrorAction SilentlyContinue)
                    $row = [ordered]@{
                        app = $app.name; folder = $folder; run = $run
                        private_mb = [math]::Round(($procs | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 1)
                        working_set_mb = [math]::Round(($procs | Measure-Object WorkingSet64 -Sum).Sum / 1MB, 1)
                        processes = $procs.Count; settled_s = [math]::Round($started.Elapsed.TotalSeconds, 1); shown = $shown
                        title = ($procs | Where-Object MainWindowTitle | Select-Object -First 1).MainWindowTitle
                    }
                    $rawExternal.Add($row)
                    if ($run -gt 0 -and $shown) { $external["$($app.name)|$folder"] += , $row }
                    Write-Host ("{0} {1} run {2}: {3} MB private, {4} MB working set ({5} s)" -f $app.name, $folder, $run, $row.private_mb, $row.working_set_mb, $row.settled_s)
                } finally {
                    Close-Launched $ids
                    Start-Sleep -Milliseconds 800
                    # Late children (a second process the app started) go too.
                    Stop-Launched @(Get-Process $app.process -ErrorAction SilentlyContinue | Where-Object { $before -notcontains $_.Id } | ForEach-Object Id)
                }
            }
        }
        } finally { Restore-State $app.state $stateCopy }
    }
    $rawExternal | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $rawDir 'apps.json')

    foreach ($folder in $folders.Keys) {
        foreach ($counter in @(@{ key = 'private_mb'; id = 'private'; label = 'private bytes' }, @{ key = 'working_set_mb'; id = 'ws'; label = 'working set' })) {
            $results = foreach ($app in $apps) {
                $rows = $external["$($app.name)|$folder"]
                if ($rows) { Result $app.name @($rows | ForEach-Object { $_.($counter.key) }) }
            }
            if ($results) {
                Scenario "fresh-$folder-$($counter.id)" "Memory ($($counter.label)): fresh process opened on $($folders[$folder])" 'MB' `
                    ('Each app started on the folder and left until its private bytes stopped growing (2% over 3 s, 30 s at most), then read with Get-Process (all processes it started; Explorer runs as a separate explorer.exe with /separate). Memory only: the other apps expose no hook to time when a listing is painted.' + $(if ($skipped) { ' Skipped: ' + ($skipped -join '; ') + '.' } else { '' })) `
                    $results
            }
        }
    }
}

# ---- results.json ---------------------------------------------------------------

$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
$ram = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
$os = Get-CimInstance Win32_OperatingSystem
$disk = try {
    $number = (Get-Partition -DriveLetter $fixtures.Substring(0, 1)).DiskNumber
    $physical = Get-PhysicalDisk | Where-Object DeviceId -eq "$number"
    "$($physical.FriendlyName) ($($physical.MediaType), $($physical.BusType))"
} catch { 'unknown' }
$version = (Select-String -LiteralPath (Join-Path $repo 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$commit = git -C $repo rev-parse --short HEAD
if (git -C $repo status --porcelain -- src Cargo.toml) { $commit += '-dirty' }

[ordered]@{
    date = Get-Date -Format 'yyyy-MM-dd'
    app_version = $version
    commit = $commit
    machine = [ordered]@{ cpu = $cpu; ram_gb = $ram; disk = $disk; os = "$($os.Caption) $($os.Version)" }
    scenarios = $scenarios
    method = "FileManager release build (LTO), $Runs runs after one warm-up run, each a fresh process with a fresh settings folder and a 1280x800 window. Timings come from an opt-in hook (FM_BENCH, off in normal use) that logs when the frame showing a listing has been painted; a steps file drives the session (open folders, switch to XL icons, scroll). The app memory figures are its own GetProcessMemoryInfo counters at those points. The comparison starts each app as a fresh process on one folder (one uncounted warm-up launch per folder first), reads its memory from outside once it settles, then closes its window. Apps run with their installed settings (File Pilot reads its own through the known-folder API); its settings files are put back afterwards. Fixtures are generated by bench/fixtures.ps1."
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'results.json')
Write-Host "Wrote $(Join-Path $PSScriptRoot 'results.json')"
