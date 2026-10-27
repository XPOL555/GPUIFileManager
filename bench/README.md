# Benchmark

Measured numbers for loading large folders, navigating and memory, FileManager next to
Windows Explorer and File Pilot when it is installed.

## Run it

```powershell
pwsh bench/run.ps1
```

It generates the fixtures (once), builds the release exe into `target/claude-test`, runs
everything and writes `bench/results.json` plus the raw logs in `bench/raw/`. About 10 to
15 minutes; windows open and close on their own meanwhile, don't use the mouse over them.

Options: `-Runs 5` (FileManager scripted runs), `-CompareRuns 3` (runs per app and folder in
the comparison), `-Scratch <dir>` (default `%TEMP%\fm-bench`), `-SkipBuild`, `-SkipOthers`
(FileManager only). `pwsh bench/fixtures.ps1 -Clean` deletes the fixtures (~0.5 GB).

## Fixtures (`fixtures.ps1`)

Under `%TEMP%\fm-bench\fixtures`, outside the repository. Each is made once; a
`<name>.complete` file next to it marks it done.

| Folder | Content |
| --- | --- |
| `start` | 10 files, where every run starts |
| `flat-1k`, `flat-10k`, `flat-100k` | 1 000 / 10 000 / 100 000 small files, mixed extensions |
| `images` | 2 000 JPEGs, 1600×1200 (50 pictures, 40 copies each, every file its own thumbnail) |
| `ue\MyGame` | An Unreal-like project: Content, Source, Config, Plugins, Saved, Binaries, and deep `Intermediate` / `DerivedDataCache` trees (~2 500 folders, ~28 000 files) |

## What is measured

**FileManager, scripted.** One warm-up run (not counted), then 5 runs, each a fresh process
with a fresh settings folder (`APPDATA` points to the scratch folder). With `FM_BENCH=<file>`
set, the app appends JSON lines there (`src/bench.rs`, `src/app/bench.rs`; nothing runs when
it is unset):

- `listed`: a folder's listing is in the table and the frame that shows it has been painted
  (a marker painted last in the window). `list_ms` is from the request to the sorted rows,
  `paint_ms` to the end of that frame, `t_ms` since the OS created the process.
- `mem`: the process's own working set and private bytes (`GetProcessMemoryInfo`).

`FM_BENCH_STEPS` names a steps file: `open <path>` (waits for its frame), `view xl`,
`scroll` (runs the view to its end, three rows a frame), `wait <ms>`, `mem <label>`, `quit`.
`run.ps1` writes the session: start in `start`, open 1k, 10k, 100k, 50 folders of the
Unreal project one after the other, then the images in XL icons scrolled to the end.

**Every app, from outside.** Each app is started as a fresh process on a folder and left until
its private bytes stop growing (under 2% over 3 s, 30 s at most); `Get-Process` then reads
its private bytes and working set (all processes it started), and its window is closed (then
killed if it is still there). One launch per folder before the counted ones warms the caches.
Explorer runs with `/separate`, a process of its own, so the shell's process is not counted
(or touched). File Pilot finds its settings through the known-folder API, not `APPDATA`, so it
runs with the settings installed on the machine; `run.ps1` copies its settings files (last
session) away before and puts them back after. OneCommander (3.108) is left out: given a folder
on its command line it opens a window with no folder pane, so its memory would say nothing. An app
already running is skipped (a launch would hand the folder to the running instance).

The other apps expose nothing that says when a listing has been painted, so they are
compared on memory only; timings are FileManager's alone.

## Caveats

- Times are with a warm OS file cache (the fixtures were just written or read in the warm-up).
- Thumbnails come from the shell's thumbnail cache, also warm after the first run.
- Memory read from inside (scripted session) and from outside (comparison) are the same
  counters, but the sessions differ: don't mix the two tables.
- Default views differ between apps (Explorer may pick a picture layout for `images`).
- Navigation through 50 small folders is bimodal across runs (about 17 or 53 ms per folder):
  the listing is ready in 10 to 13 ms either way, the difference is in when the frame is
  painted. Not explained yet.
- File Pilot's private bytes are high from the start (~570 MB with a 10-file folder open);
  that's what its counters report and may be memory reserved up front, not a leak.
