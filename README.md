<p align="center"><img src="assets/icon.png" width="96" alt="FileManager icon"></p>

# FileManager

A lightweight, fast file manager for Windows, written in Rust.

The goal is File Pilot-level speed with **bounded, predictable memory use**, even on huge trees such as Unreal Engine projects. There are no recursive folder sizes and no recursive watchers. Folder sizes will only be computed on demand, they will be cancellable, and they will skip heavy folders (`Intermediate`, `DerivedDataCache`, `.git`, `node_modules`, …) by default.

The UI is built with [gpui-kit](https://crates.io/crates/gpui-kit), which packages Zed's GPUI together with Longbridge's `gpui-component`.

> **Status:** early work in progress.

## Features

- View modes like File Pilot: details, tree, list, and medium, large and extra large icons. Switch with `Ctrl` + mouse wheel or from the info bar at the bottom right
- Native Windows icons for files and folders, and thumbnails of images and videos, loaded in the background with a bounded cache
- Preview pane (`Alt+P`) with a large thumbnail and the file's details
- Tree view: expand folders in place; only what is expanded is kept in memory
- Sidebar with favorites and drives as collapsible sections. Favorites can be added from the context menu, removed, and reordered by dragging. Click to open in the current tab, double-click (or middle-click) to open in a new tab
- Resizable sidebar that collapses with an animation (`Ctrl+B` or the title bar button)
- Tabs with back/forward history. Drag to reorder, drag out of the window to detach into a new window, drag onto another window to move there. A preview follows the cursor and says what releasing will do
- Address bar with breadcrumb. Click its empty space to type a path, with folder autocompletion (`↑`/`↓` to pick, `Tab` to complete, `Enter` to go)
- Instant filter and sortable columns (natural sort)
- The app's own context menu (open, open in new tab/window, add to favorites, copy path, properties). "Show more options", or `Shift`+right-click, opens Explorer's native menu
- Folder listing on background threads; stale results are discarded
- Three themes (dark, dimmed with five accent colors, light), English and Italian UI, all switchable at runtime from the settings (☰ or `Ctrl+,`)
- Settings saved in `%APPDATA%\FileManager\settings.json`

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `Enter` | Open selected item |
| `Backspace` / `Alt+Up` | Parent folder |
| `Alt+Left` / `Alt+Right` | Back / Forward |
| `F5` | Refresh |
| `Ctrl+T` / `Ctrl+W` | New tab / Close tab |
| `Ctrl+Tab` | Next tab |
| `Ctrl+N` | New window |
| `Ctrl+F` | Focus filter |
| `Ctrl+L` | Edit path |
| `Ctrl+H` | Toggle hidden files |
| `Ctrl` + mouse wheel | Switch view mode |
| `Ctrl+B` | Show or hide the sidebar |
| `Alt+P` | Preview pane |
| `Ctrl+,` | Settings |

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

- [ ] Folder watcher (`ReadDirectoryChangesW`) on the visible folder only
- [ ] Multi-selection, rename (F2), copy/move/delete via `IFileOperation`, clipboard
- [ ] Drag & drop to Explorer
- [ ] Persistent tabs
- [ ] Search through the Everything SDK

## License

[MIT](LICENSE)
