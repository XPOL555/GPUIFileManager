<p align="center"><img src="assets/icon.png" width="96" alt="FileManager icon"></p>

# FileManager

A lightweight, fast file manager for Windows, written in Rust.

The goal is File Pilot-level speed with **bounded, predictable memory use**, even on huge trees such as Unreal Engine projects. There are no recursive folder sizes and no recursive watchers. Folder sizes will only be computed on demand, they will be cancellable, and they will skip heavy folders (`Intermediate`, `DerivedDataCache`, `.git`, `node_modules`, …) by default.

The UI is built with [gpui-kit](https://crates.io/crates/gpui-kit), which packages Zed's GPUI together with Longbridge's `gpui-component`.

> **Status:** early work in progress.

## Features

- View modes like File Pilot: details, tree, list, and medium, large and extra large icons. Switch with `Ctrl` + mouse wheel or from the info bar at the bottom right
- Cover Flow, as in the old Finder: pictures and videos in perspective over the details list, with reflections. Drag across the covers (or use the wheel, or the bar under them) to run through them quickly; a filter shows all files, photos and videos, or photos only. Drag the edge between the covers and the list to give them more or less room (double-click it to go back to the default)
- Native Windows icons for files and folders, and thumbnails of images and videos, loaded in the background with a bounded cache
- Preview pane (`Alt+P`) with a large thumbnail and the file's details
- Tree view: expand folders in place; only what is expanded is kept in memory
- Sidebar with favorites and drives as collapsible sections, and the Recycle Bin below them. Favorites can be added from the context menu or by dropping folders on the favorites heading, removed, and reordered by dragging. Click to open in the current tab, double-click (or middle-click) to open in a new tab
- The Recycle Bin inside the app: where each item was deleted from and when, restore, delete for good, empty it. Files dropped on it go to the bin
- Shortcuts (`.lnk`) look like shortcuts: the arrow on the icon, no extension; one to a folder opens it in the tab, and "Open file location" shows its target
- Settings to show hidden files and file extensions
- Resizable sidebar that collapses with an animation (`Ctrl+B` or the title bar button)
- Tabs with back/forward history. Drag along the strip to reorder them, out of the window to detach into a new window, onto another window (also of another running instance) to move there, where you drop it. A preview follows the cursor and says what releasing will do
- Address bar with breadcrumb. Click its empty space to type a path, with folder autocompletion (`↑`/`↓` to pick, `Tab` to complete, `Enter` to go)
- Instant filter and sortable columns (natural sort)
- Multiple selection as in Explorer: `Ctrl`+click, `Shift`+click, `Shift`/`Ctrl` + arrows, `Ctrl+A`, and a rubber band dragged from the empty space, in every view
- Rename in place (`F2`), delete to the Recycle Bin (`Del`) or for good (`Shift+Del`), cut / copy / paste (`Ctrl+X` / `C` / `V`, interoperable with Explorer), new folder (`Ctrl+Shift+N`). Operations go through the Windows shell, with its progress, conflict and confirmation dialogs
- Drag and drop of files with Explorer and other apps, both ways, and between windows: drop on the file view, a folder, a tab, a favorite, a drive or a breadcrumb segment. Same rules as Explorer: moves within a drive, copies to another one, `Ctrl` copies, `Shift` moves
- The listed folder updates by itself when its content changes
- The app's own context menu, in Windows 11's style: a row of icons for cut, copy, rename and delete, then open, "Open with", and every entry Explorer's menu has (what apps add, "Send to", "New"…), with their icons. It slides open and works with the keyboard. "Show more options", or `Shift`+right-click, opens Explorer's own menu, dark or light with the app
- Folder listing on background threads; stale results are discarded
- Three themes (dark, dimmed with five accent colors, light), English and Italian UI, all switchable at runtime from the settings (☰ or `Ctrl+,`)
- Settings saved in `%APPDATA%\FileManager\settings.json`

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `Enter` | Open the selected items |
| `↑` `↓` `←` `→` `Home` `End` `PgUp` `PgDn` | Move; with `Shift` extend the selection, with `Ctrl` move without selecting |
| `Ctrl+Space` | Select or deselect the item at the cursor |
| `Ctrl+A` | Select all |
| `F2` | Rename |
| `Del` / `Shift+Del` | Delete to the Recycle Bin / for good |
| `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | Cut / Copy / Paste files |
| `Ctrl+Shift+N` | New folder |
| `Alt+Enter` | Properties |
| `Backspace` / `Alt+Up` | Parent folder |
| `Alt+Left` / `Alt+Right` | Back / Forward |
| `F5` | Refresh |
| `Ctrl+T` / `Ctrl+W` | New tab / Close tab |
| `Ctrl+Tab` | Next tab |
| `Ctrl+N` | New window |
| `Ctrl+F` | Focus filter |
| `Ctrl+L` | Edit path |
| `Ctrl+H` | Show or hide hidden files |
| `Ctrl` + mouse wheel | Switch view mode |
| `Ctrl+B` | Show or hide the sidebar |
| `Alt+P` | Preview pane |
| `Ctrl+,` | Settings |

## Installing

Each [release](https://github.com/XPOL555/GPUIFileManager/releases) ships:

- `filemanager-vX.Y.Z-windows-x64-setup.exe`: NSIS setup, per-machine (asks for admin), installs in `C:\Program Files\FileManager`
- `filemanager-vX.Y.Z-windows-x64.msi`: the same as an MSI, handy for silent or managed deployment
- `filemanager.exe` and a zip: portable, no installation

Both installers add an all-users Start menu entry and an entry in Settings > Apps, and installing a newer version replaces the old one. Use one of the two, not both. Settings in `%APPDATA%` are kept on uninstall.

```sh
filemanager-vX.Y.Z-windows-x64-setup.exe /S              # silent (add /D=C:\path to change the folder)
msiexec /i filemanager-vX.Y.Z-windows-x64.msi /qn        # silent
msiexec /x filemanager-vX.Y.Z-windows-x64.msi /qn        # uninstall
```

To build the installers locally you need [WiX 5](https://wixtoolset.org) (`dotnet tool install --global wix --version 5.0.0`) for the MSI and [NSIS 3](https://nsis.sourceforge.io) for the setup:

```sh
pwsh installer/build.ps1                  # builds the release exe, then every installer it has the tool for, into dist/
pwsh installer/build.ps1 -Format msi      # or nsis; add -SkipBuild to reuse target/release/filemanager.exe
```

## Building

Requires Windows and a recent stable Rust toolchain (edition 2024).

```sh
cargo run -- <path>       # start in <path> (default: user profile)
cargo test                # unit tests
cargo build --release     # optimized build, no console window
```

The app icon is drawn in `assets/icon.svg`. After editing it, regenerate `assets/app.ico` (embedded in the exe) and `assets/icon.png`:

```sh
cargo run --manifest-path tools/icon/Cargo.toml
```

## Roadmap

- [x] Folder watcher on the visible folder only
- [x] Multi-selection, rename (F2), copy/move/delete via `IFileOperation`, clipboard
- [x] Drag & drop with Explorer
- [x] Marquee selection
- [x] Recycle Bin, Cover Flow, Explorer's entries in the app's menu
- [ ] Persistent tabs
- [ ] Search through the Everything SDK

## License

[MIT](LICENSE)
