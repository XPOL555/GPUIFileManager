---
title: Performance
description: FileManager's memory next to Windows Explorer and File Pilot, and its own listing times — the numbers, the machine, the method, and what is not good yet.
navigation:
  icon: i-lucide-gauge
---

FileManager is built to stay light while you keep folders open and move between them. This page shows what the
benchmark measured on one machine, and nothing more: where FileManager is not good yet, it says so too.

::bench-machine
::

## In short

- **Memory, next to the others.** Opened fresh on a folder, FileManager's working set was **91–120 MB**,
  against **195–219 MB** for Windows Explorer and **386–425 MB** for File Pilot. Counted as private bytes the gap
  with Explorer is smaller (FileManager 8–32% lower); File Pilot stays more than four times higher.
- **Memory over a session.** After listing 1 000, 10 000 and 100 000 files and walking 50 folders, FileManager
  was back to **96 MB** of working set, the same as when idle: browsing did not pile up memory.
- **Not good yet.** Start up to the first painted listing takes **0.6 s**. Walking 50 small folders one after
  the other took **2.7 s** in most runs, though each listing was ready in 10–13 ms. A grid of 2 000 photos in
  extra large icons, scrolled to the end, takes **~230 MB** of working set.

## Memory next to Explorer and File Pilot

Each app started as a fresh process on one folder, left until its memory stopped growing, then read from
outside. Only memory is compared: the other apps have no hook that says when a listing is on screen.

The **working set** is the memory the process has in RAM right now; **private bytes** is the memory it has
committed for itself, in RAM or not. Both are what Task Manager and `Get-Process` report.

::tabs
  :::tabs-item{label="Working set"}
    ::::bench-chart{id="fresh-start-ws" title="A folder with 10 files" hide-notes}
    ::::

    ::::bench-chart{id="fresh-flat-100k-ws" title="A folder with 100 000 files" hide-notes}
    ::::

    ::::bench-chart{id="fresh-images-ws" title="A folder with 2 000 photos, default view" hide-notes}
    ::::
  :::

  :::tabs-item{label="Private bytes"}
    ::::bench-chart{id="fresh-start-private" title="A folder with 10 files" hide-notes}
    ::::

    ::::bench-chart{id="fresh-flat-100k-private" title="A folder with 100 000 files" hide-notes}
    ::::

    ::::bench-chart{id="fresh-images-private" title="A folder with 2 000 photos, default view" hide-notes}
    ::::
  :::
::

Explorer ran as a process of its own (`explorer /separate`), so what the Windows shell shares with it is not
counted. File Pilot's private bytes are high from the very start (~570 MB with 10 files open): that is what its
counters report, possibly memory reserved up front rather than used. OneCommander was tried too and left out:
opened on a folder from the command line, it showed a window with no folder pane.

## FileManager over a session

One scripted session per run: start in a folder with 10 files, open 1 000, 10 000 and 100 000 files, walk 50
folders of an Unreal-like project, then open 2 000 photos in extra large icons and scroll to the end. The app
read its own memory at each step. Not comparable with the charts above: a different session, read from inside.

::tabs
  :::tabs-item{label="Working set"}
    ::::bench-rows
    ---
    ids: memory-idle-ws memory-after-100k-ws memory-after-nav-ws memory-images-xl-scrolled-ws
    title: Memory at each step
    strip: "Memory (working set): "
    ---
    ::::
  :::

  :::tabs-item{label="Private bytes"}
    ::::bench-rows
    ---
    ids: memory-idle-private memory-after-100k-private memory-after-nav-private memory-images-xl-scrolled-private
    title: Memory at each step
    strip: "Memory (private bytes): "
    ---
    ::::
  :::
::

Listings and navigation leave nothing behind: after the 100 000 files and the 50 folders the working set is
where it started. Photos are another story: thumbnails in extra large icons take about 130 MB more once the whole
grid has been seen. They are held in a cache with a size limit, so that figure is a ceiling rather than a leak,
but the limit is generous.

## Times (FileManager only)

From the request to the end of painting the frame that shows the sorted rows, in a running window, with the
files recently read (the Windows file cache is warm).

::bench-rows
---
ids: list-1k list-10k list-100k
title: Open a folder
strip: "Open a folder with "
---
::

::bench-chart{id="startup" title="Start up, until the first listing is painted"}
::

::bench-chart{id="navigate-50" title="Walk 50 folders of an Unreal-like project, one after the other"}
::

The 50 folders are bimodal across runs: each step took about 53 ms in most of them and about 17 ms in the fastest.
The listing itself is ready in 10–13 ms every time; the rest is waiting for the next frame to be painted. The
cause is not found yet.

## Method

::bench-method
::

## Caveats

- **One machine**, a fast desktop with an NVMe SSD (above). Slower disks, a cold file cache or an antivirus
  scanning every folder will change the times; compare the bars with each other, not with your own stopwatch.
- **Warm caches.** Listings were timed with the folders recently read; thumbnails came from the shell's
  thumbnail cache, also warm.
- **Default views differ.** Each app opened the folders in its own default view (Explorer may pick a picture
  layout for the photos).
- **Many tabs and windows** open at once are not measured yet: every number here is one window, one folder at a
  time.
- **Versions.** FileManager 0.3.0 plus work in progress (the commit above). The other apps were the versions
  installed on the machine, with their own settings; their version numbers were not recorded.

## Reproduce it

The harness lives in the repository under
[`bench/`](https://github.com/XPOL555/GPUIFileManager/tree/main/bench): its
[`README.md`](https://github.com/XPOL555/GPUIFileManager/blob/main/bench/README.md) explains how to create the
test folders and run every scenario (about 10–15 minutes, Windows only).
