# FileManager

A lightweight Windows file manager in Rust. The goal is File Pilot-level speed with bounded, predictable memory use. The UI uses **gpui-kit 0.7**: Zed's GPUI, published as `gpui-pre-*`, plus `gpui-component` from Longbridge.

## Commands
- `cargo run -- <path>`: start in `<path>` (default: user profile)
- `cargo test`: unit tests (sorting, formatting, path completion, settings)
- `cargo build --release`: LTO build without a console window
- `cargo run --manifest-path tools/icon/Cargo.toml`: regenerate `assets/app.ico` and `assets/icon.png` from `assets/icon.svg` (standalone crate, not part of the app build)

## Layout
- `src/main.rs`: bootstrap (assets, settings, dark theme, first window, quit on last window closed)
- `src/app.rs`: `FileManager` view (custom title bar with the settings dialog, sidebar, tabs, address bar with breadcrumb and path autocompletion, filter, status bar, the app's context menu), actions and keybindings (context `FileManager`), `open_window`, cross-window tab drag
- `src/settings.rs`: `Settings` global, JSON in `%APPDATA%\FileManager\settings.json`; `Settings::update` saves in the background and redraws
- `src/i18n.rs`: UI strings (English, Italian). The language lives in `Settings`; `set_language` switches it live
- `src/assets.rs`: asset source = gpui-kit's default icons + the extra Lucide icons listed in `icon_assets!`, and the full-color logo
- `src/table.rs`: `FileTable`, a `TableDelegate` that owns the folder snapshot plus a filtered and sorted `rows` index
- `src/fs.rs`: `list()` (std `read_dir`, no recursion), natural sort, path completion, size and date formatting
- `src/shell.rs`: Win32/COM (drives, known folders, `ShellExecuteW`, native `IContextMenu`, properties, window placement)
- `build.rs` + `assets/app.rc`: embed `assets/app.ico` as icon resource 1 (Explorer shows it, GPUI loads it for every window)

## Rules
- **Memory must stay bounded.** No recursive folder sizes and no recursive watchers. Folder sizes will only ever be computed on demand, cancellable, and with default exclusions (`Intermediate`, `DerivedDataCache`, `.git`, `node_modules`, …). This is the whole reason this project exists: File Pilot grows to GBs on UE projects.
- Import everything through `gpui_kit::*` and `gpui_kit::component::*`. Never add `gpui` directly: the versions must match the ones gpui-kit pins.
- Its API changes often. Check the sources in `~/.cargo/registry/src/*/gpui-component-0.7.0` and `gpui-base-0.7.0` before assuming a signature. `.when()` needs `prelude::FluentBuilder`, `.small()` needs `Sizable`, `.disabled()` needs `Disableable`.
- Test modules must not `use super::*` when the parent imports `gpui_kit::*`: the glob brings GPUI's `test` macro, which shadows `#[test]`. Import the types explicitly.
- Icons: `gpui_kit::assets::Assets` only ships the icons components use. A Lucide icon that renders blank is missing: add it to `icon_assets!` in `src/assets.rs`. Don't switch to `AllAssets` (1800 icons).
- `shell::show_context_menu` runs a nested Win32 modal loop. Call it only from a `cx.spawn` task and never while holding a GPUI borrow (`update`/`read`), otherwise GPUI's wndproc re-enters and panics.
- Every user-visible string goes through `i18n::t(cx)`, with both languages filled in. State that caches a string (input placeholders) must observe `Settings`.
- New settings: add a field to `Settings` (keep `#[serde(default)]` so old files still load) and change it only through `Settings::update`.
- The title bar is a Windows `HTCAPTION` drag area: an unhandled left mouse-down there starts the native move loop and the click never arrives. Wrap interactive title bar elements in a div that calls `cx.stop_propagation()` on mouse-down.
- Tab drag between windows: GPUI drags live inside one window, but Windows keeps the mouse captured by the source window, so a release outside it still arrives there (`on_mouse_up_out`). `release_tab_outside` then finds the window under the cursor (`WindowFromPoint`, matched against `OpenWindows`). Every window must be opened through `app::open_window` so it is registered.
- Filesystem I/O goes on `cx.background_spawn`. Use `load_generation` / `suggest_generation` to discard stale results.
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
- [ ] Persistent bookmarks and tabs (in `Settings`)
- [ ] Search through the Everything SDK
