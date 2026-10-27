---
seo:
  title: FileManager — a Windows file manager that stays light with many folders open
  description: "A lightweight file manager for Windows, written in Rust on Zed's GPUI. Open as many folders, tabs and windows as you need: memory grows with what you have open, not with the size of your drives."
---

::u-page-hero{orientation="horizontal"}
#headline
  :::u-badge{icon="i-simple-icons-windows" color="neutral" variant="subtle" size="lg"}
  Windows 10 / 11 · open source (MIT)
  :::

#title
Open every folder you need, [keep your RAM]{.text-primary}

#description
FileManager is a lightweight file manager for Windows, written in Rust on Zed's GPUI. Tabs, windows, expanded trees: it is built to scale with how much you have open, without growing into gigabytes of RAM.

#links
  :::u-button
  ---
  size: xl
  to: https://github.com/XPOL555/GPUIFileManager/releases/latest
  target: _blank
  icon: i-lucide-download
  ---
  Download
  :::

  :::u-button
  ---
  color: neutral
  size: xl
  to: https://github.com/XPOL555/GPUIFileManager
  target: _blank
  variant: ghost
  icon: i-simple-icons-github
  ---
  Source
  :::

#default
  :::hero-scene
  :::
::

::u-page-section
#title
Why another file manager

#description
A file manager stays open all day, with a dozen tabs and a few windows. If every folder you open is indexed, cached and watched recursively, memory keeps climbing until it is measured in gigabytes. FileManager keeps for each open folder only what that folder needs, and caps everything shared.

  :::card-group
    ::::card{icon="i-lucide-folder-tree" title="Only what you see"}
    Each tab lists its folder, without recursion. The tree view keeps only the folders you expanded.
    ::::

    ::::card{icon="i-lucide-eye" title="One watcher, one folder"}
    Change notifications for the listed folders only, never a recursive watcher on a whole tree.
    ::::

    ::::card{icon="i-lucide-image" title="Budgeted caches"}
    Icons and thumbnails live in caches with a byte budget, shared by every tab and window: more folders open does not mean more cache.
    ::::

    ::::card{icon="i-lucide-calculator" title="Folder sizes only when you ask"}
    Off by default. Turned on, they cover only the folder you are in, stop when you leave it, skip `.git`, `node_modules`, `Intermediate`, `DerivedDataCache`, and keep one number per folder.
    ::::
  :::
::

::u-page-section
#title
A complete file manager

#features
  :::u-page-feature
  ---
  icon: i-lucide-layout-grid
  to: /features#views
  ---
  #title
  Every view

  #description
  Details, tree, list, medium / large / extra large icons and Cover Flow. Switch with `Ctrl` + wheel.
  :::

  :::u-page-feature
  ---
  icon: i-lucide-app-window
  to: /features#tabs-and-windows
  ---
  #title
  Tabs that travel

  #description
  Reorder them live, drag one out to a new window or onto another window, even of another instance.
  :::

  :::u-page-feature
  ---
  icon: i-lucide-mouse-pointer-click
  to: /features#explorer-integration
  ---
  #title
  Explorer's menus

  #description
  A Windows 11 style context menu with every entry Explorer's menu has, and "Show more options" for the native one.
  :::

  :::u-page-feature
  ---
  icon: i-lucide-copy
  to: /features#file-operations
  ---
  #title
  The shell's file operations

  #description
  Copy, move, rename and delete through Windows, with its progress and conflict dialogs. Clipboard and drag and drop with Explorer, both ways.
  :::

  :::u-page-feature
  ---
  icon: i-lucide-trash-2
  to: /features#recycle-bin
  ---
  #title
  Recycle Bin inside

  #description
  Where each item came from and when, restore, delete for good, empty. Drop files on it to delete them.
  :::

  :::u-page-feature
  ---
  icon: i-lucide-palette
  to: /features#themes-and-languages
  ---
  #title
  Dark, dimmed, light

  #description
  Three themes, five accents, English and Italian, all switchable at runtime.
  :::
::

::u-page-section
#title
Download

#description
Each release ships a setup, an MSI for managed or silent installs, and a portable `filemanager-portable.exe` (also zipped) that keeps its settings next to itself. Windows 10 or 11, 64-bit.

#links
  :::u-button
  ---
  size: xl
  to: https://github.com/XPOL555/GPUIFileManager/releases/latest
  target: _blank
  icon: i-lucide-download
  ---
  Setup, MSI and zip
  :::

  :::u-button
  ---
  color: neutral
  size: xl
  variant: outline
  to: /getting-started/install
  icon: i-lucide-book-open
  ---
  Install guide
  :::
::
