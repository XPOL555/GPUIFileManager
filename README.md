<p align="center"><img src="assets/icon.png" width="96" alt="FileManager icon"></p>

# FileManager

A lightweight, fast file manager for Windows, written in Rust.

The goal is File Pilot-level speed with **bounded, predictable memory use**, even on huge trees such as Unreal Engine projects. There are no recursive folder sizes and no recursive watchers. Folder sizes will only be computed on demand, they will be cancellable, and they will skip heavy folders (`Intermediate`, `DerivedDataCache`, `.git`, `node_modules`, …) by default.

The UI is built with [gpui-kit](https://crates.io/crates/gpui-kit), which packages Zed's GPUI together with Longbridge's `gpui-component`.

> **Status:** early work in progress.

## Features

- Sidebar with known folders and drives: click to open in the current tab, double-click (or middle-click) to open in a new tab
- Tabs with back/forward history. Drag to reorder, drag out of the window to detach into a new window, drag onto another window to move there
- Address bar with breadcrumb. Click its empty space to type a path, with folder autocompletion (`↑`/`↓` to pick, `Tab` to complete, `Enter` to go)
- Instant filter and sortable columns (natural sort)
- The app's own context menu (open, open in new tab/window, copy path, properties). "Show more options", or `Shift`+right-click, opens Explorer's native menu
- Folder listing on background threads; stale results are discarded
- Custom title bar with a settings dialog (☰ or `Ctrl+,`)
- English and Italian UI, switchable at runtime
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
- [ ] Real shell icons with a bounded cache
- [ ] Persistent bookmarks and tabs
- [ ] Search through the Everything SDK

## License

[MIT](LICENSE)
