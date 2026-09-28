# FileManager

A lightweight, fast file manager for Windows, written in Rust.

The goal is File Pilot-level speed with **bounded, predictable memory use**, even on huge trees such as Unreal Engine projects. There are no recursive folder sizes and no recursive watchers. Folder sizes will only be computed on demand, they will be cancellable, and they will skip heavy folders (`Intermediate`, `DerivedDataCache`, `.git`, `node_modules`, …) by default.

The UI is built with [gpui-kit](https://crates.io/crates/gpui-kit), which packages Zed's GPUI together with Longbridge's `gpui-component`.

> **Status:** early work in progress.

## Features

- Sidebar with known folders and drives
- Tabs with back/forward history
- Address bar with breadcrumb, plus editable path mode
- Instant filter and sortable columns (natural sort)
- Native Windows shell context menu
- Folder listing on background threads; stale results are discarded

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `Enter` | Open selected item |
| `Backspace` / `Alt+Up` | Parent folder |
| `Alt+Left` / `Alt+Right` | Back / Forward |
| `F5` | Refresh |
| `Ctrl+T` / `Ctrl+W` | New tab / Close tab |
| `Ctrl+Tab` | Next tab |
| `Ctrl+F` | Focus filter |
| `Ctrl+L` | Edit path |
| `Ctrl+H` | Toggle hidden files |

## Building

Requires Windows and a recent stable Rust toolchain (edition 2024).

```sh
cargo run -- <path>       # start in <path> (default: user profile)
cargo test                # unit tests
cargo build --release     # optimized build, no console window
```

## Roadmap

- [ ] Folder watcher (`ReadDirectoryChangesW`) on the visible folder only
- [ ] Multi-selection, rename (F2), copy/move/delete via `IFileOperation`, clipboard
- [ ] Drag & drop to Explorer
- [ ] Real shell icons with a bounded cache
- [ ] Persistent bookmarks and tabs
- [ ] Search through the Everything SDK
