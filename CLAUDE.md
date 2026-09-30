# FileManager

A lightweight Windows file manager in Rust. The goal is File Pilot-level speed with bounded, predictable memory use. The UI uses **gpui-kit 0.7**: Zed's GPUI, published as `gpui-pre-*`, plus `gpui-component` from Longbridge.

## Commands
- `cargo run -- <path>`: start in `<path>` (default: user profile)
- `cargo test`: unit tests (sorting, formatting, path completion, tree rows, selection, drop rules, file names, settings, themes, Cover Flow geometry). Some run the real shell in `%TEMP%`: copy / move / rename, a file sent to the Recycle Bin and restored, Explorer's menus read (`FM_DUMP_MENU=1` prints them)
- `cargo build --release`: LTO build without a console window
- `cargo run --manifest-path tools/icon/Cargo.toml`: regenerate `assets/app.ico` and `assets/icon.png` from `assets/icon.svg` (standalone crate, not part of the app build)

## Layout
- `src/main.rs`: bootstrap (assets, settings, theme, icon workers, hook channel, first window, quit on last window closed)
- `src/app/mod.rs`: `FileManager` view (window, navigation, actions and keybindings in contexts `FileManager` and `FileView`, settings dialog, title bar), `Tab`, `open_window` (installs the window's hooks and drop target), `OpenWindows`, `init` (delivers `HookEvent`s to their window's view)
- `src/app/sidebar.rs`: favorites (in `Settings`, reorder by dragging) and drives as accordion sections; resize handle; collapse animation (a `spring`)
- `src/app/tabs.rs`: tab strip; a dragged tab moves along the strip live, lands where the cursor is on another window (of this instance, or of another one through `hooks`), or detaches into a new window
- `src/app/files.rs`: the file view (focus, keys, background clicks, drop zone of the listed folder), selection with mouse, keyboard and rubber band (the same in the table and the grid), rename in place, delete / cut / copy / paste / new folder, file drag start, drops, `reload` (lists again keeping selection, filter and scroll) and the debounced reload on change notifications
- `src/app/address_bar.rs`: breadcrumb, editable path with folder autocompletion, filter
- `src/app/views.rs`: `ViewMode` (details, tree, list, M/L/XL icons, Cover Flow), icon grid (`uniform_list` of rows of tiles), Ctrl + wheel, preview pane, status bar and info bar with the view picker
- `src/app/cover_flow.rs`: Cover Flow over the details table: covers in perspective drawn as thin vertical strips (`paint_image` of a part of the image; strips hidden behind the next cover toward the middle are skipped, which keeps a frame at ~1.5 ms), reflections made once per image, glide toward the cursor (`request_animation_frame`), drag / wheel / bar to run through them, the media filter buttons, the handle across the edge with the list (`Settings::flow_height`). Strips are counted (`strips`), never added up: float sums may stop moving and hang the app
- `src/app/bin.rs`: the Recycle Bin as a place (restore, delete for good with a confirmation, empty), its empty / full icon
- `src/app/folder_prefs.rs`: sorting (column header click, per tab), view and sorting kept per folder (`Settings::folders`, header / view button right click), the one-time snackbar offering it
- `src/app/menu.rs`: the app's context menus (the selection, background, sidebar items, the Recycle Bin): our entries, slots where Explorer's entries go as they load, and Explorer's native menu ("Show more options")
- `src/context_menu.rs`: the menu component: Windows 11 look (bigger rows, an icon row), submenus with a hover delay, keyboard, a slide as it opens (up when it opens above the cursor), clicks outside close it; `Slot`s filled later
- `src/drag_preview.rs`: floating pop-up window that follows a dragged tab outside the app's windows
- `src/icons.rs`: native icons and thumbnails: requests from views, LIFO job queues, worker threads, byte-budgeted LRU caches of `RenderImage`
- `src/shell_images.rs`: Win32 side of icons/thumbnails (system image list, `IShellItemImageFactory`), BGRA pixels
- `src/settings.rs`: `Settings` global, JSON in `%APPDATA%\FileManager\settings.json`; `Settings::update` saves in the background and redraws
- `src/theme.rs`: dark and light (gpui-component defaults) and dimmed (built here, 5 accents), applied with `Theme::update` + `apply_config`
- `src/i18n.rs`: UI strings (English, Italian). The language lives in `Settings`; `set_language` switches it live
- `src/assets.rs`: asset source = gpui-kit's default icons + the extra Lucide icons listed in `icon_assets!`, and the full-color logo
- `src/table.rs`: `FileTable`, a `TableDelegate` that owns the folder snapshot, the listings of expanded tree folders, the filtered and sorted rows (hidden files and the media filter follow `Settings`), and the selection (keys that survive sorting and filtering; cursor and Shift anchor). It renders its own header cells (`render_th`): the table's sort cycle, column selection and column moving are off. Rows draw the selection and send their mouse input to the owner (`RowEvent`); the table's own row selection is off. In the Recycle Bin the type and date columns show where and when items were deleted
- `src/file_ops.rs`: copy / move / delete / rename through `IFileOperation`, each on a thread of its own (the shell's progress, conflict and confirmation dialogs; `main` doesn't quit while one runs); files on the clipboard (`CF_HDROP` + `Preferred DropEffect`, as Explorer); `CutFiles` global (faded until pasted). Its tests really copy, move and rename in `%TEMP%` through the shell
- `src/dnd.rs`: file drag and drop: the drag source (the shell's data object, `SHDoDragDrop`), each window's drop target (replaces GPUI's), drop zones recorded while painting, Explorer's copy / move rules (`drop_kind`), the text under the drag image
- `src/hooks.rs`: Win32 subclass of every window: a property marks the app's windows, tabs from other instances (`WM_COPYDATA`), change notifications for the listed folder (`SHChangeNotifyRegister`, not recursive), the start of file drags. Everything becomes a `HookEvent` on a channel
- `src/fs.rs`: `list()` (std `read_dir`, no recursion), `Entry` (display names without hidden extensions), the Recycle Bin's path (`RECYCLE_BIN`, a shell parsing name that tabs hold like a folder), photos / videos and `MediaFilter`, natural sort, path completion, size and date formatting
- `src/shell.rs`: Win32/COM (drives, known folders, `ShellExecuteW`, native `IContextMenu`, shortcut targets, properties, window placement, dark native menus)
- `src/shell_menu.rs`: Explorer's context menu read entry by entry (labels, verbs, icons, submenus) on a thread of its own that keeps it until the app's menu closes and invokes the entry picked there; icons and entries that come late are sent after
- `src/recycle.rs`: the Recycle Bin read from the disk (`X:\$Recycle.Bin\<SID>\$I…` / `$R…`), empty / full state, emptying
- `installer/`: `filemanager.wxs` (WiX 5 MSI), `filemanager.nsi` (NSIS setup), both per-machine; `build.ps1` builds them into `dist/` (`-Format msi|nsis|all`, `-SkipBuild`). The MSI `UpgradeCode` must never change
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
- The app's menus are `context_menu::ContextMenu` (not gpui-component's `PopupMenu`, except the title bar's). Explorer's entries come from `shell_menu`: its thread owns the `IContextMenu`, so commands are invoked there (`ShellMenu::invoke`), never on the UI thread. The first menu a process builds lacks the packaged apps' entries (Terminal, VS Code…): `main` warms one up in the background at startup.
- Menu item bitmaps (`HBITMAP`) are 32-bit handles sign-extended: they may look negative. Only -1 (`HBMMENU_CALLBACK`, drawn through `WM_MEASUREITEM`/`WM_DRAWITEM`) and 1 to 11 are special.
- Single-line inputs let Enter, Backspace at their start and Left / Right at their ends bubble up (`cx.propagate()`), and GPUI then tries the next binding of the key. So the file view's keys (`enter`, `backspace`) are bound in `FileView` only, and the file view swallows those input actions coming from the rename editor.
- While a GPUI foreground task runs (even one stuck in a modal loop), no other task runs: GPUI doesn't wake itself again until it returns. So the file drag loop (`dnd::drag_files`) starts from the window procedure (`hooks::start_file_drag` posts a message), where the drop targets of the app's own windows can still get their `HookEvent`s delivered.
- Win32 hooks (subclass proc, drop target) never touch GPUI: they only `hooks::send` events, which the task from `app::init` delivers to the window's view.
- A drop never answers `DROPEFFECT_MOVE`: the app moves the files itself (Explorer's "optimized move"), so it answers `DROPEFFECT_NONE` and the source must not delete anything.
- `SendMessageTimeoutW` to another instance from inside a GPUI update uses `SMTO_BLOCK`, so nothing re-enters this thread meanwhile.
- The selection lives in `FileTable`, the same for the table and the grid: never use `TableState`'s selection (`row_selectable(false)`). The file view has its own focus handle (`files_focus`, context `FileView`); the table must not keep the focus, its `DataTable` key bindings would shadow ours.
- Items (rows, tiles) stop their mouse-downs: the file view's own handlers then only see presses on the background. Elements inside items (tree expander, rename editor) stop theirs too.
- A drop zone is recorded with `.on_prepaint(dnd::zone(…))`; zones are cleared at the start of every `FileManager::render`, and the innermost (highest priority) zone under the cursor wins.
- Every user-visible string goes through `i18n::t(cx)`, with both languages filled in. State that caches a string (input placeholders) must observe `Settings`.
- The Recycle Bin is a tab path like any folder (`fs::RECYCLE_BIN`): use `fs::parent`, not `Path::parent`, and `list_folder`, not `fs::list`, for what a tab shows. Its items' `path` is the bin's `$R…` file and `origin` where they were: they never go to the clipboard, a drag or a rename; restores and permanent deletes go through `FileOp::Restore` / `Purge`, which drop the `$I…` files.
- Drop zones have a `dnd::Target`: a folder (copy / move), the favorites heading (folders become favorites) or the Recycle Bin (delete to it).
- New settings: add a field to `Settings` (keep `#[serde(default)]` so old files still load) and change it only through `Settings::update`.
- The title bar is a Windows `HTCAPTION` drag area: an unhandled left mouse-down there starts the native move loop and the click never arrives. Wrap interactive title bar elements in a div that calls `cx.stop_propagation()` on mouse-down.
- Tab drag between windows: GPUI drags live inside one window, but Windows keeps the mouse captured by the source window, so moves and the release outside it still arrive there (`on_drag_move`, `on_mouse_up_out`). The window under the cursor comes from `WindowFromPoint`, matched against `OpenWindows`, else against the property `hooks` puts on the windows of other instances (they get the tab as JSON through `WM_COPYDATA`; bump `hooks::PROTOCOL` when `Tab` changes incompatibly). The drag preview window is disabled so it is looked through. Every window must be opened through `app::open_window` so it is registered and hooked.
- Filesystem I/O goes on `cx.background_spawn`. Use `load_generation` / `suggest_generation` to discard stale results.
- Testing by hand while the user may be running the app: build with `CARGO_TARGET_DIR=target/claude-test` and start that exe with `APPDATA` pointed at a scratch folder, so their running copy and their settings are left alone.
- Commits: never add `Co-Authored-By: Claude …` or other AI attribution lines to commit messages or PRs. The author is Paolo only.

## Release
- `.github/workflows/release.yml` builds on `windows-latest` only, and only on `v*` tags. It publishes a GitHub Release with `filemanager.exe` plus a zip.
- The same job also builds the MSI and the NSIS setup with `installer/build.ps1` (NSIS from choco, WiX as a dotnet tool) and attaches them.
- To release: bump `version` in `Cargo.toml`, commit, `git tag vX.Y.Z`, `git push origin main vX.Y.Z`.

## Status / TODO
- [ ] Verify the native context menu by hand (including the "Send to" / "Open with" submenus via the subclass proc)
- [x] Watcher on the visible folder only (`SHChangeNotifyRegister`, not recursive)
- [x] Multi-selection, rename (F2), copy/move/delete via `IFileOperation`, clipboard (CF_HDROP)
- [x] Drag & drop out to Explorer and in from it, onto folders, tabs, favorites, drives and the breadcrumb
- [x] Tabs between separate instances, live tab reordering
- [ ] Verify by hand: tab drag between two instances, file drag and drop both ways, rename, clipboard with Explorer, delete confirmations
- [x] Marquee (rubber band) selection, from the view's background (below the rows in the table)
- [x] Windows 11 style context menu with Explorer's entries, dark native menu
- [x] Recycle Bin (browse, restore, delete for good, empty, drop to delete), favorites by dropping folders on their heading
- [x] Cover Flow view with photo / video filter; shortcuts shown as shortcuts; settings for hidden files and extensions
- [ ] Verify by hand: the new menu (keyboard, submenus, Explorer's entries run), Cover Flow (drag, wheel, bar, the resize handle), the Recycle Bin (restore, delete for good, empty)
- [ ] Persistent tabs (in `Settings`)
- [ ] Search through the Everything SDK
