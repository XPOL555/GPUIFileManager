# FileManager

A lightweight Windows file manager in Rust. The goal is File Pilot-level speed with bounded, predictable memory use. The UI uses **gpui-kit 0.7**: Zed's GPUI, published as `gpui-pre-*`, plus `gpui-component` from Longbridge.

## Commands
- `cargo run -- <path>`: start in `<path>` (default: user profile)
- `cargo test`: unit tests (sorting, formatting)
- `cargo build --release`: LTO build without a console window

## Layout
- `src/main.rs`: bootstrap (assets, dark theme, window, quit on last window closed)
- `src/app.rs`: `FileManager` view (sidebar, tabs, address bar/breadcrumb, filter, status bar), actions and keybindings (context `FileManager`)
- `src/table.rs`: `FileTable`, a `TableDelegate` that owns the folder snapshot plus a filtered and sorted `rows` index
- `src/fs.rs`: `list()` (std `read_dir`, no recursion), natural sort, size and date formatting
- `src/shell.rs`: Win32/COM (drives, known folders, `ShellExecuteW`, native `IContextMenu`)

## Rules
- **Memory must stay bounded.** No recursive folder sizes and no recursive watchers. Folder sizes will only ever be computed on demand, cancellable, and with default exclusions (`Intermediate`, `DerivedDataCache`, `.git`, `node_modules`, …). This is the whole reason this project exists: File Pilot grows to GBs on UE projects.
- Import everything through `gpui_kit::*` and `gpui_kit::component::*`. Never add `gpui` directly: the versions must match the ones gpui-kit pins.
- Its API changes often. Check the sources in `~/.cargo/registry/src/*/gpui-component-0.7.0` and `gpui-base-0.7.0` before assuming a signature. `.when()` needs `prelude::FluentBuilder`, `.small()` needs `Sizable`, `.disabled()` needs `Disableable`.
- `shell::show_context_menu` runs a nested Win32 modal loop. Call it only from a `cx.spawn` task and never while holding a GPUI borrow (`update`/`read`), otherwise GPUI's wndproc re-enters and panics.
- Filesystem I/O goes on `cx.background_spawn`. Use `load_generation` to discard stale results.
- Commits: never add `Co-Authored-By: Claude …` or other AI attribution lines to commit messages or PRs. The author is Paolo only.

## Release
- `.github/workflows/release.yml` builds on `windows-latest` only, and only on `v*` tags. It publishes a GitHub Release with `filemanager.exe` plus a zip.
- To release: bump `version` in `Cargo.toml`, commit, `git tag vX.Y.Z`, `git push origin main vX.Y.Z`.

## Status / TODO
- [ ] Verify the native context menu by hand (including the "Send to" / "Open with" submenus via the subclass proc)
- [ ] Watcher (`ReadDirectoryChangesW`) on the visible folder only
- [ ] Multi-selection, rename (F2), copy/move/delete via `IFileOperation`, clipboard (CF_HDROP)
- [ ] Drag & drop out to Explorer (OLE `DoDragDrop`)
- [ ] Real shell icons (`SHGetFileInfoW`) with a bounded cache
- [ ] Persistent bookmarks and tabs (JSON in `%APPDATA%`)
- [ ] Search through the Everything SDK
