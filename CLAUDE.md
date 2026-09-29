# FileManager

A lightweight Windows file manager in Rust. The goal is File Pilot-level speed with bounded, predictable memory use. The UI uses **gpui-kit 0.7**: Zed's GPUI, published as `gpui-pre-*`, plus `gpui-component` from Longbridge.

## Commands
- `cargo run -- <path>`: start in `<path>` (default: user profile)
- `cargo test`: unit tests (sorting, formatting, path completion, tree rows, settings, themes)
- `cargo build --release`: LTO build without a console window
- `cargo run --manifest-path tools/icon/Cargo.toml`: regenerate `assets/app.ico` and `assets/icon.png` from `assets/icon.svg` (standalone crate, not part of the app build)

## Layout
- `src/main.rs`: bootstrap (assets, settings, theme, icon workers, first window, quit on last window closed)
- `src/app/mod.rs`: `FileManager` view (window, navigation, actions and keybindings in contexts `FileManager` and `FileGrid`, settings dialog, title bar), `open_window`, `OpenWindows`
- `src/app/sidebar.rs`: favorites (in `Settings`, reorder by dragging) and drives as accordion sections; resize handle; collapse animation (a `spring`)
- `src/app/tabs.rs`: tab strip, drag to reorder / detach / move between windows
- `src/app/address_bar.rs`: breadcrumb, editable path with folder autocompletion, filter
- `src/app/views.rs`: `ViewMode` (details, tree, list, M/L/XL icons), icon grid (`uniform_list` of rows of tiles), Ctrl + wheel, preview pane, status bar and info bar with the view picker
- `src/app/folder_prefs.rs`: sorting (column header click, per tab), view and sorting kept per folder (`Settings::folders`, header / view button right click), the one-time snackbar offering it
- `src/app/menu.rs`: the app's context menu (files, background, sidebar items) and Explorer's native one
- `src/drag_preview.rs`: floating pop-up window that follows a dragged tab outside the app's windows
- `src/icons.rs`: native icons and thumbnails: requests from views, LIFO job queues, worker threads, byte-budgeted LRU caches of `RenderImage`
- `src/shell_images.rs`: Win32 side of icons/thumbnails (system image list, `IShellItemImageFactory`), BGRA pixels
- `src/settings.rs`: `Settings` global, JSON in `%APPDATA%\FileManager\settings.json`; `Settings::update` saves in the background and redraws
- `src/theme.rs`: dark and light (gpui-component defaults) and dimmed (built here, 5 accents), applied with `Theme::update` + `apply_config`
- `src/i18n.rs`: UI strings (English, Italian). The language lives in `Settings`; `set_language` switches it live
- `src/assets.rs`: asset source = gpui-kit's default icons + the extra Lucide icons listed in `icon_assets!`, and the full-color logo
- `src/table.rs`: `FileTable`, a `TableDelegate` that owns the folder snapshot, the listings of expanded tree folders, and the filtered and sorted rows. It renders its own header cells (`render_th`): the table's sort cycle, column selection and column moving are off
- `src/fs.rs`: `list()` (std `read_dir`, no recursion), natural sort, path completion, size and date formatting
- `src/shell.rs`: Win32/COM (drives, known folders, `ShellExecuteW`, native `IContextMenu`, properties, window placement)
- `build.rs` + `assets/app.rc`: embed `assets/app.ico` as icon resource 1 (Explorer shows it, GPUI loads it for every window)

## Rules
- **Memory must stay bounded.** No recursive folder sizes and no recursive watchers. Folder sizes will only ever be computed on demand, cancellable, and with default exclusions (`Intermediate`, `DerivedDataCache`, `.git`, `node_modules`, …). This is the whole reason this project exists: File Pilot grows to GBs on UE projects. The tree view keeps only expanded folders; icon and thumbnail caches have byte budgets.
- Import everything through `gpui_kit::*` and `gpui_kit::component::*`. Never add `gpui` directly: the versions must match the ones gpui-kit pins. (`image` and `futures` are direct dependencies only because GPUI's API uses their types; keep them on the versions gpui-kit resolves.)
- Its API changes often. Check the sources in `~/.cargo/registry/src/*/gpui-component-0.7.0` and `gpui-base-0.7.0` before assuming a signature. `.when()` needs `prelude::FluentBuilder`, `.small()` needs `Sizable`, `.disabled()` needs `Disableable`, `.selected()` needs `Selectable`.
- In `src/app/*` submodules use `super::*` only, not `gpui_kit::*` as well: two globs make `Path` (std vs GPUI) and `open_window` ambiguous.
- Test modules must not `use super::*` when the parent imports `gpui_kit::*`: the glob brings GPUI's `test` macro, which shadows `#[test]`. Import the types explicitly.
- Icons: `gpui_kit::assets::Assets` only ships the icons components use. A Lucide icon that renders blank is missing: add it to `icon_assets!` in `src/assets.rs`. Don't switch to `AllAssets` (1800 icons).
- `RenderImage` wants BGRA with straight alpha. Evicted images must also be freed from the GPU atlas (`cx.drop_image`), as `icons` does.
- Mouse listeners in the capture phase run in paint order and the table's scroller stops the wheel there: a capture listener meant to see wheel events over a view (Ctrl + wheel) must be painted before that view.
- `shell::show_context_menu` runs a nested Win32 modal loop. Call it only from a `cx.spawn` task and never while holding a GPUI borrow (`update`/`read`), otherwise GPUI's wndproc re-enters and panics.
- Every user-visible string goes through `i18n::t(cx)`, with both languages filled in. State that caches a string (input placeholders) must observe `Settings`.
- New settings: add a field to `Settings` (keep `#[serde(default)]` so old files still load) and change it only through `Settings::update`.
- The title bar is a Windows `HTCAPTION` drag area: an unhandled left mouse-down there starts the native move loop and the click never arrives. Wrap interactive title bar elements in a div that calls `cx.stop_propagation()` on mouse-down.
- Tab drag between windows: GPUI drags live inside one window, but Windows keeps the mouse captured by the source window, so moves and the release outside it still arrive there (`on_drag_move`, `on_mouse_up_out`). The window under the cursor comes from `WindowFromPoint`, matched against `OpenWindows`; the drag preview window is disabled so it is looked through. Every window must be opened through `app::open_window` so it is registered.
- Filesystem I/O goes on `cx.background_spawn`. Use `load_generation` / `suggest_generation` to discard stale results.
- Testing by hand while the user may be running the app: build with `CARGO_TARGET_DIR=target/claude-test` and start that exe with `APPDATA` pointed at a scratch folder, so their running copy and their settings are left alone.
- Commits: never add `Co-Authored-By: Claude …` or other AI attribution lines to commit messages or PRs. The author is Paolo only.

## Release
- `.github/workflows/release.yml` builds on `windows-latest` only, and only on `v*` tags. It publishes a GitHub Release with `filemanager.exe` plus a zip.
- To release: bump `version` in `Cargo.toml`, commit, `git tag vX.Y.Z`, `git push origin main vX.Y.Z`.

## Status / TODO
- [ ] Verify the native context menu by hand (including the "Send to" / "Open with" submenus via the subclass proc)
- [ ] Watcher (`ReadDirectoryChangesW`) on the visible folder only
- [ ] Multi-selection, rename (F2), copy/move/delete via `IFileOperation`, clipboard (CF_HDROP)
- [ ] Drag & drop out to Explorer (OLE `DoDragDrop`), and files onto favorites
- [ ] Persistent tabs (in `Settings`)
- [ ] Search through the Everything SDK
