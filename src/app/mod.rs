//! Main window. This module holds the window, navigation and actions; the areas of
//! the window live in submodules: `sidebar` (favorites, drives), `tabs` (tab strip,
//! dragging tabs between windows and instances), `address_bar`, `views` (view modes,
//! icon grid, preview pane, info bar), `files` (selection, file operations, file drag
//! and drop), `folder_prefs` (sorting, view and sorting kept per folder), `menu`
//! (context menus) and `about` (About dialog, update check).

mod about;
mod address_bar;
mod bin;
mod cover_flow;
mod files;
mod folder_prefs;
mod menu;
mod sidebar;
mod tabs;
mod views;

pub use views::ViewMode;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{
    ActiveTheme as _, ElementExt as _, Icon, Selectable as _, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::dnd::{SharedZones, SpotKey};
use crate::drag_preview::DragPreviewWindow;
use crate::hooks::{self, HookEvent};
use crate::i18n::{self, Language};
use crate::settings::Settings;
use crate::table::{FileTable, HeaderClick, RowEvent};
use crate::theme::{self, Accent, ThemeChoice};
use crate::{fs, shell};

type LucideIcon = gpui_kit::assets::IconName;

actions!(
    file_manager,
    [
        OpenSelected,
        GoBack,
        GoForward,
        GoUp,
        Refresh,
        NewTab,
        NewWindow,
        CloseTab,
        NextTab,
        FocusFilter,
        EditPath,
        ToggleHidden,
        OpenSettings,
        ToggleSidebar,
        TogglePreview,
        NewFolder,
        CursorUp,
        CursorDown,
        CursorLeft,
        CursorRight,
        CursorHome,
        CursorEnd,
        CursorPageUp,
        CursorPageDown,
        SelectAll,
        ToggleCursorItem,
        RenameSelected,
        DeleteSelected,
        DeletePermanently,
        CopySelected,
        CutSelected,
        PasteFiles,
        ShowProperties
    ]
);

const CONTEXT: &str = "FileManager";
/// The file view (table or grid), when it has the keyboard.
const FILES_CONTEXT: &str = "FileView";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        // Only in the file view: inputs (filter, path) let Enter and Backspace at their
        // start bubble up, which must not open the selection or leave the folder.
        KeyBinding::new("enter", OpenSelected, Some(FILES_CONTEXT)),
        KeyBinding::new("backspace", GoUp, Some(FILES_CONTEXT)),
        KeyBinding::new("alt-up", GoUp, Some(CONTEXT)),
        KeyBinding::new("alt-left", GoBack, Some(CONTEXT)),
        KeyBinding::new("alt-right", GoForward, Some(CONTEXT)),
        KeyBinding::new("f5", Refresh, Some(CONTEXT)),
        KeyBinding::new("ctrl-t", NewTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-n", NewWindow, Some(CONTEXT)),
        KeyBinding::new("ctrl-w", CloseTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-f", FocusFilter, Some(CONTEXT)),
        KeyBinding::new("ctrl-l", EditPath, Some(CONTEXT)),
        KeyBinding::new("ctrl-h", ToggleHidden, Some(CONTEXT)),
        KeyBinding::new("ctrl-,", OpenSettings, Some(CONTEXT)),
        KeyBinding::new("ctrl-b", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("alt-p", TogglePreview, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-n", NewFolder, Some(CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(FILES_CONTEXT)),
        KeyBinding::new("ctrl-space", ToggleCursorItem, Some(FILES_CONTEXT)),
        KeyBinding::new("f2", RenameSelected, Some(FILES_CONTEXT)),
        KeyBinding::new("delete", DeleteSelected, Some(FILES_CONTEXT)),
        KeyBinding::new("shift-delete", DeletePermanently, Some(FILES_CONTEXT)),
        KeyBinding::new("ctrl-c", CopySelected, Some(FILES_CONTEXT)),
        KeyBinding::new("ctrl-x", CutSelected, Some(FILES_CONTEXT)),
        KeyBinding::new("ctrl-v", PasteFiles, Some(FILES_CONTEXT)),
        KeyBinding::new("alt-enter", ShowProperties, Some(FILES_CONTEXT)),
    ]);
    // Moves: with Shift they extend the selection, with Ctrl they only move the cursor.
    let mut moves = Vec::new();
    moves.extend(move_keys("up", CursorUp));
    moves.extend(move_keys("down", CursorDown));
    moves.extend(move_keys("left", CursorLeft));
    moves.extend(move_keys("right", CursorRight));
    moves.extend(move_keys("home", CursorHome));
    moves.extend(move_keys("end", CursorEnd));
    moves.extend(move_keys("pageup", CursorPageUp));
    moves.extend(move_keys("pagedown", CursorPageDown));
    cx.bind_keys(moves);
}

fn move_keys<A: Action + Clone>(key: &str, action: A) -> [KeyBinding; 4] {
    ["", "shift-", "ctrl-", "ctrl-shift-"]
        .map(|modifiers| KeyBinding::new(&format!("{modifiers}{key}"), action.clone(), Some(FILES_CONTEXT)))
}

static NEXT_TAB_ID: AtomicU64 = AtomicU64::new(1);

fn next_tab_id() -> u64 {
    NEXT_TAB_ID.fetch_add(1, Ordering::Relaxed)
}

/// A tab; it moves between windows and instances as it is (history, view, sorting).
#[derive(Clone, Serialize, Deserialize)]
pub struct Tab {
    /// Tells tabs apart while they are dragged around; not kept across instances.
    #[serde(skip, default = "next_tab_id")]
    id: u64,
    path: PathBuf,
    #[serde(default)]
    back: Vec<PathBuf>,
    #[serde(default)]
    forward: Vec<PathBuf>,
    /// View and sorting shown: the ones kept for the folder, else the picked ones.
    #[serde(default)]
    view: ViewMode,
    #[serde(default)]
    sort: fs::Sort,
    /// The last view and sorting picked by hand in this tab.
    #[serde(default)]
    picked_view: ViewMode,
    #[serde(default)]
    picked_sort: fs::Sort,
}

impl Tab {
    /// A tab on `path`, starting from the view and sorting last picked in any tab.
    pub fn new(path: PathBuf, cx: &App) -> Self {
        let settings = Settings::get(cx);
        let (view, sort) = (settings.view_mode, settings.sort);
        let mut tab = Self {
            id: next_tab_id(),
            path,
            back: Vec::new(),
            forward: Vec::new(),
            view,
            sort,
            picked_view: view,
            picked_sort: sort,
        };
        tab.use_folder_prefs(cx);
        tab
    }

    /// On entering a folder: its kept view and sorting, or the picked ones.
    fn use_folder_prefs(&mut self, cx: &App) {
        let prefs = Settings::get(cx).folder(&self.path);
        self.view = prefs.view.unwrap_or(self.picked_view);
        self.sort = prefs.sort.unwrap_or(self.picked_sort);
    }

    fn title(&self, cx: &App) -> SharedString {
        folder_name(&self.path, cx).into()
    }
}

/// A folder's name as the app shows it: the last part of its path, the drive, or the
/// Recycle Bin.
fn folder_name(path: &Path, cx: &App) -> String {
    if fs::is_recycle_bin(path) {
        return i18n::t(cx).recycle_bin.to_string();
    }
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Lists a folder, or the Recycle Bin. Blocks: run it on a background thread.
fn list_folder(path: &Path) -> std::io::Result<Vec<fs::Entry>> {
    if fs::is_recycle_bin(path) { Ok(crate::recycle::list()) } else { fs::list(path) }
}

/// Every open window, so a tab released outside its own window can find the window
/// under the cursor, and hook events their view.
#[derive(Default)]
struct OpenWindows(Vec<OpenWindow>);

struct OpenWindow {
    view: WeakEntity<FileManager>,
    handle: AnyWindowHandle,
    hwnd: isize,
}

impl Global for OpenWindows {}

/// Starts delivering what the windows' Win32 hooks receive (tabs from other instances,
/// folder changes, file drops) to their views. Call once, before opening windows.
pub fn init(cx: &mut App) {
    let mut events = hooks::init();
    cx.spawn(async move |cx| {
        while let Some((hwnd, event)) = events.next().await {
            cx.update(|cx| deliver(hwnd, event, cx));
        }
    })
    .detach();
}

fn deliver(hwnd: isize, event: HookEvent, cx: &mut App) {
    let Some((view, handle)) = cx
        .try_global::<OpenWindows>()
        .and_then(|windows| windows.0.iter().find(|w| w.hwnd == hwnd))
        .map(|w| (w.view.clone(), w.handle))
    else {
        return;
    };
    handle
        .update(cx, |_, window, cx| {
            view.update(cx, |fm, cx| fm.on_hook(event, window, cx)).ok();
        })
        .ok();
}

/// Places a window so the cursor sits over its first tab, as if it was being held by it.
fn place_under_cursor(hwnd: isize, cursor: shell::ScreenPoint, scale: f32) {
    let grab = ((300. * scale) as i32, (52. * scale) as i32);
    shell::move_window(hwnd, (cursor.0 - grab.0, cursor.1 - grab.1), cursor);
}

/// Opens a window holding `tabs`, placed under `cursor` (screen pixels) when given.
pub fn open_window(tabs: Vec<Tab>, cursor: Option<shell::ScreenPoint>, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    // The app draws its own title bar (`component::TitleBar`), with the menus on the left.
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("FileManager".into()), ..TitleBar::title_bar_options() }),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| FileManager::new(tabs, window, cx)));
    let (handle, view) = match opened {
        Ok(opened) => opened,
        Err(err) => return eprintln!("open window: {err:?}"),
    };
    let hwnd = handle
        .update(cx, |_, window, cx| {
            let hwnd = shell::hwnd(window);
            if let Some(hwnd) = hwnd {
                if let Some(cursor) = cursor {
                    place_under_cursor(hwnd, cursor, window.scale_factor());
                }
                hooks::install(hwnd);
                crate::dnd::register_drop_target(hwnd, view.read(cx).drop_zones.clone());
                view.update(cx, |fm, _| fm.bin_watch = hooks::watch_bin(hwnd));
            }
            view.update(cx, |fm, cx| fm.focus_view(window, cx));
            hwnd
        })
        .ok()
        .flatten()
        .unwrap_or(0);
    let windows = &mut cx.default_global::<OpenWindows>().0;
    windows.retain(|w| w.view.upgrade().is_some());
    windows.push(OpenWindow { view: view.downgrade(), handle, hwnd });
}

fn drive_label(d: &shell::Drive) -> String {
    let root = d.root.to_string_lossy();
    if d.label.is_empty() { root.into_owned() } else { format!("{} ({})", d.label, &root[..2]) }
}

pub struct FileManager {
    focus: FocusHandle,
    hwnd: isize,
    logo: Arc<Image>,
    tabs: Vec<Tab>,
    active: usize,
    table: Entity<TableState<FileTable>>,
    filter: Entity<InputState>,
    error: Option<SharedString>,
    places: Vec<(shell::KnownFolder, PathBuf)>,
    drives: Vec<shell::Drive>,
    /// Bumped on each navigation so late results from a previous listing are dropped.
    load_generation: u64,
    context_menu: Option<menu::OpenMenu>,
    snackbar: Option<folder_prefs::Snackbar>,

    // Address bar (see `address_bar`).
    path_input: Entity<InputState>,
    editing_path: bool,
    suggestions: Vec<address_bar::Suggestion>,
    suggestion_ix: Option<usize>,
    suggest_generation: u64,

    // Sidebar (see `sidebar`).
    sidebar_width: f32,
    /// Width on screen in the last frame, collapse animation included.
    sidebar_visible: f32,
    sidebar_collapsed: bool,
    resizing_sidebar: bool,
    sidebar_click: Option<sidebar::SidebarClick>,

    // Views (see `views`, `cover_flow`).
    grid_scroll: UniformListScrollHandle,
    flow: cover_flow::CoverFlow,
    /// Where the table is on screen, from the last frame (under the covers in Cover Flow).
    table_bounds: Rc<Cell<Bounds<Pixels>>>,
    grid_columns: usize,
    zoom_pixels: f32,
    view_slider: Entity<SliderState>,
    preview_pane: bool,

    // Files (see `files`).
    /// The file view (table or grid) and its keyboard context.
    files_focus: FocusHandle,
    /// Where the file view is on screen, from the last frame.
    files_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Where an item was pressed, until the mouse moves far enough to drag it.
    press: Rc<Cell<Option<Point<Pixels>>>>,
    rename: Option<files::Rename>,
    marquee: Option<files::Marquee>,
    /// Whether a rubber band is being dragged, for the raw mouse listener.
    marquee_active: Rc<Cell<bool>>,
    /// Where files can be dropped in this window, recorded while painting.
    drop_zones: SharedZones,
    /// The zone a file drag hovers.
    drop_hover: Option<SpotKey>,
    /// Change notifications for the listed folder.
    watch: Option<hooks::Watch>,
    /// Change notifications for the Recycle Bin (its icon, its listing).
    bin_watch: Option<hooks::Watch>,
    /// A drag holding folders is over the window: the favorites heading takes them.
    dragging_folders: bool,
    /// A reload after changes in the folder, waiting for more changes to settle.
    reload_timer: Option<Task<()>>,
    error_timer: Option<Task<()>>,

    // Tab dragging (see `tabs`).
    tab_strip: Rc<Cell<Bounds<Pixels>>>,
    /// Bounds of each tab in the last frame.
    tab_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    drag_preview: Option<DragPreviewWindow>,
    drag_target: Option<tabs::DragTarget>,
    /// A tab from another window hovers this one: where it would land.
    incoming_tab: Option<tabs::IncomingTab>,

    _subscriptions: Vec<Subscription>,
}

impl FileManager {
    fn new(tabs: Vec<Tab>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        assert!(!tabs.is_empty(), "a window needs at least one tab");
        let settings = Settings::get(cx).clone();
        let drop_zones = SharedZones::default();
        // Rows are selected by `files` (the same way as in the grid), not by the table;
        // headers sort through `sort_by`, not the table's own sort cycle, and a click
        // on one must not select (highlight) its column.
        let table = cx.new(|cx| {
            TableState::new(FileTable::new(), window, cx)
                .row_selectable(false)
                .col_selectable(false)
                .col_movable(false)
                .sortable(false)
        });
        let this = cx.entity().downgrade();
        table.update(cx, |t, _| {
            let d = t.delegate_mut();
            d.set_show_hidden(settings.show_hidden);
            d.set_zones(drop_zones.clone());
            let toggle = this.clone();
            d.set_on_toggle(Rc::new(move |path, expand, window, cx| {
                toggle.update(cx, |fm, cx| fm.toggle_folder(path, expand, window, cx)).ok();
            }));
            let row = this.clone();
            d.set_on_row(Rc::new(move |event, window, cx| {
                row.update(cx, |fm, cx| fm.on_row(event, window, cx)).ok();
            }));
            d.set_on_header(Rc::new(move |click, window, cx| {
                this.update(cx, |fm, cx| match click {
                    HeaderClick::Sort(key) => fm.sort_by(key, cx),
                    HeaderClick::Menu(position) => fm.show_keep_menu(folder_prefs::Pref::Sort, position, window, cx),
                })
                .ok();
            }));
        });
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t(cx).filter));
        let path_input = cx.new(|cx| InputState::new(window, cx));
        let mode = tabs[0].view;
        let view_slider =
            cx.new(|_| SliderState::new().min(0.).max((ViewMode::ALL.len() - 1) as f32).step(1.).default_value(mode.slider_value()));

        let table_focus = table.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
            // The keyboard belongs to the file view: the table's own bindings (arrows,
            // Home, End…) would shadow its actions.
            cx.on_focus(&table_focus, window, |this, window, cx| this.focus_view(window, cx)),
            cx.subscribe_in(&filter, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let q = input.read(cx).value();
                    this.table.update(cx, |t, cx| {
                        t.delegate_mut().set_filter(&q);
                        t.refresh(cx);
                    });
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if this.table.read(cx).delegate().len() > 0 {
                        this.update_selection(cx, |d| d.select_only(0));
                        this.scroll_to_row(0, cx);
                    }
                    this.focus_view(window, cx);
                }
                _ => {}
            }),
            cx.subscribe_in(&path_input, window, Self::on_path_input_event),
            cx.subscribe_in(&view_slider, window, |this, _, ev: &SliderEvent, window, cx| {
                if let SliderEvent::Change(value) = ev {
                    this.set_view_mode(ViewMode::from_slider(value.end()), window, cx);
                }
            }),
            cx.observe_global_in::<Settings>(window, |this, window, cx| {
                let placeholder = i18n::t(cx).filter;
                this.filter.update(cx, |f, cx| f.set_placeholder(placeholder, window, cx));
                this.apply_list_settings(cx);
            }),
        ];

        // The Recycle Bin's icon: empty or full.
        bin::check_bin(cx);
        let mut this = Self {
            focus: cx.focus_handle(),
            hwnd: shell::hwnd(window).unwrap_or(0),
            logo: crate::assets::logo(),
            tabs,
            active: 0,
            table,
            filter,
            error: None,
            places: shell::known_folders(),
            drives: shell::drives(),
            load_generation: 0,
            context_menu: None,
            snackbar: None,
            path_input,
            editing_path: false,
            suggestions: Vec::new(),
            suggestion_ix: None,
            suggest_generation: 0,
            sidebar_width: settings.sidebar_width,
            sidebar_visible: if settings.sidebar_collapsed { 0. } else { settings.sidebar_width },
            sidebar_collapsed: settings.sidebar_collapsed,
            resizing_sidebar: false,
            sidebar_click: None,
            grid_scroll: UniformListScrollHandle::new(),
            flow: Default::default(),
            table_bounds: Rc::default(),
            grid_columns: 1,
            zoom_pixels: 0.,
            view_slider,
            preview_pane: settings.preview_pane,
            files_focus: cx.focus_handle(),
            files_bounds: Rc::default(),
            press: Rc::default(),
            rename: None,
            marquee: None,
            marquee_active: Rc::default(),
            drop_zones,
            drop_hover: None,
            watch: None,
            bin_watch: None,
            dragging_folders: false,
            reload_timer: None,
            error_timer: None,
            tab_strip: Rc::default(),
            tab_bounds: Rc::default(),
            drag_preview: None,
            drag_target: None,
            incoming_tab: None,
            _subscriptions: subscriptions,
        };
        this.load(None, window, cx);
        this
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn view(&self) -> ViewMode {
        self.tab().view
    }

    /// Keyboard focus to the file view.
    fn focus_view(&self, window: &mut Window, cx: &mut App) {
        self.files_focus.focus(window, cx);
    }

    /// What a Win32 hook of this window received.
    fn on_hook(&mut self, event: HookEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            HookEvent::TabHover { title, cursor } => self.show_incoming(title.into(), cursor, window, cx),
            HookEvent::TabLeave => self.clear_incoming(cx),
            HookEvent::TabDrop { tab, cursor } => self.receive_tab(tab, cursor, window, cx),
            HookEvent::FolderChanged => self.folder_changed(window, cx),
            HookEvent::DropHover(spot) => self.set_drop_hover(spot, cx),
            HookEvent::DropFiles { paths, target, kind } => self.drop_files(paths, target, kind, window, cx),
            HookEvent::DraggingFolders(dragging) => {
                self.dragging_folders = dragging;
                cx.notify();
            }
            HookEvent::BinChanged => self.bin_changed(window, cx),
            HookEvent::FileDragEnded { dropped } => {
                // Moved away, or into a subfolder: the change notification may be late.
                if dropped {
                    self.reload(files::Reselect::Keep, window, cx).detach();
                }
            }
        }
    }

    // ---- navigation -------------------------------------------------------

    /// Navigates the active tab. `record` pushes the current folder on the back stack.
    fn navigate(&mut self, path: PathBuf, record: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_to(path, record, None, window, cx);
    }

    /// `navigate`, selecting the item named `select` once listed.
    fn navigate_to(
        &mut self,
        path: PathBuf,
        record: bool,
        select: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path == self.tab().path {
            return self.load(select, window, cx);
        }
        let (view, had_focus) = (self.view(), self.files_focus.contains_focused(window, cx));
        let tab = &mut self.tabs[self.active];
        let previous = std::mem::replace(&mut tab.path, path);
        if record {
            tab.back.push(previous.clone());
            tab.forward.clear();
        }
        // The folder may have a view and a sorting of its own.
        tab.use_folder_prefs(cx);
        // The snackbar was about the folder being left.
        self.snackbar = None;
        // Coming back up from a child: reselect the folder we came from.
        let select = select.or_else(|| {
            previous.parent().filter(|p| *p == self.tab().path).and(previous.file_name()).map(|n| n.to_string_lossy().into_owned())
        });
        self.load(select, window, cx);
        if self.view() != view {
            self.sync_view_slider(window, cx);
            if had_focus {
                self.focus_view(window, cx);
            }
        }
    }

    /// Lists the active tab's folder on a background thread, and watches it for changes.
    fn load(&mut self, select: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.tab().path.clone();
        let (tree, sort) = (self.view() == ViewMode::Tree, self.tab().sort);
        let settings = Settings::get(cx);
        let show_hidden = settings.show_hidden;
        let media = if self.view() == ViewMode::CoverFlow { settings.media_filter } else { fs::MediaFilter::All };
        self.load_generation += 1;
        let generation = self.load_generation;
        self.error = None;
        self.rename = None;
        self.reload_timer = None;
        // The Recycle Bin is always watched (`bin_watch`).
        self.watch = if fs::is_recycle_bin(&path) { None } else { hooks::watch(self.hwnd, &path) };
        self.grid_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.filter.update(cx, |f, cx| f.set_value("", window, cx));
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.loading = true;
            d.renaming = None;
            d.set_tree(tree);
            d.set_sort(sort);
            d.set_show_hidden(show_hidden);
            d.set_media_filter(media);
            d.set_folder(path.clone());
            d.set_filter("");
            cx.notify();
        });

        let listing = cx.background_spawn(async move { list_folder(&path) });
        cx.spawn(async move |this, cx| {
            let result = listing.await;
            this.update(cx, |this, cx| {
                if this.load_generation != generation {
                    return;
                }
                let entries = match result {
                    Ok(e) => e,
                    Err(err) => {
                        this.error = Some(err.to_string().into());
                        Vec::new()
                    }
                };
                let row = this.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    d.loading = false;
                    d.set_entries(entries);
                    let row = select.as_deref().and_then(|n| d.row_of(n));
                    if let Some(row) = row {
                        d.select_only(row);
                    }
                    t.refresh(cx);
                    row
                });
                if let Some(row) = row {
                    this.scroll_to_row(row, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Tree view: lists `path` on a background thread and shows it under its folder,
    /// or drops its listing when collapsing.
    fn toggle_folder(&mut self, path: PathBuf, expand: bool, _: &mut Window, cx: &mut Context<Self>) {
        if !expand {
            self.table.update(cx, |t, cx| {
                t.delegate_mut().collapse(&path);
                t.refresh(cx);
            });
            return;
        }
        self.table.update(cx, |t, cx| {
            t.delegate_mut().mark_expanding(path.clone());
            cx.notify();
        });
        let generation = self.load_generation;
        let listing = cx.background_spawn({
            let path = path.clone();
            async move { fs::list(&path).unwrap_or_default() }
        });
        cx.spawn(async move |this, cx| {
            let entries = listing.await;
            this.update(cx, |this, cx| {
                if this.load_generation == generation {
                    this.table.update(cx, |t, cx| {
                        t.delegate_mut().set_children(path, entries);
                        t.refresh(cx);
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    /// Folders open in the tab, and so do shortcuts to folders; files open in their app.
    fn open_entry(&mut self, entry: fs::Entry, window: &mut Window, cx: &mut Context<Self>) {
        if entry.origin.is_some() {
            // In the Recycle Bin: a file can still be looked at, a folder not entered.
            if !entry.is_dir {
                shell::open(&entry.path);
            }
        } else if entry.is_dir {
            self.navigate(entry.path, true, window, cx);
        } else if let Some(folder) = entry.is_shortcut().then(|| shell::shortcut_target(&entry.path)).flatten().filter(|t| t.is_dir()) {
            self.navigate(folder, true, window, cx);
        } else {
            shell::open(&entry.path);
        }
    }

    /// Shows `target` in its folder, selected ("Open file location").
    fn reveal(&mut self, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(folder), Some(name)) = (target.parent(), target.file_name()) else { return };
        let name = name.to_string_lossy().into_owned();
        self.navigate_to(folder.to_path_buf(), true, Some(name), window, cx);
    }

    fn on_table_event(
        &mut self,
        table: &Entity<TableState<FileTable>>,
        ev: &TableEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            // `TableState::refresh` resets the widths to the delegate's columns.
            TableEvent::ColumnWidthsChanged(widths) => {
                let widths = widths.clone();
                table.update(cx, |t, _| t.delegate_mut().set_widths(&widths));
            }
            _ => cx.notify(),
        }
    }

    // ---- actions ----------------------------------------------------------

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[self.active];
        if let Some(p) = tab.back.pop() {
            tab.forward.push(tab.path.clone());
            self.navigate(p, false, window, cx);
        }
    }

    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[self.active];
        if let Some(p) = tab.forward.pop() {
            tab.back.push(tab.path.clone());
            self.navigate(p, false, window, cx);
        }
    }

    fn go_up(&mut self, _: &GoUp, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = fs::parent(&self.tab().path).map(Path::to_path_buf) {
            self.navigate(parent, true, window, cx);
        }
    }

    /// Lists the folder again, keeping the selection, the filter and the scroll position.
    fn refresh(&mut self, _: &Refresh, window: &mut Window, cx: &mut Context<Self>) {
        self.drives = shell::drives();
        self.reload(files::Reselect::Keep, window, cx).detach();
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(self.tab().path.clone(), window, cx);
    }

    fn new_window(&mut self, _: &NewWindow, _: &mut Window, cx: &mut Context<Self>) {
        let tab = Tab::new(self.tab().path.clone(), cx);
        cx.defer(move |cx| open_window(vec![tab], None, cx));
    }

    /// Opens `path` in a new tab and activates it.
    fn open_tab(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_tab(Tab::new(path, cx), window, cx);
    }

    fn insert_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_tab_at(tab, self.tabs.len(), window, cx);
    }

    /// Inserts `tab` at `index` and activates it.
    fn insert_tab_at(&mut self, tab: Tab, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let index = index.min(self.tabs.len());
        self.tabs.insert(index, tab);
        if index <= self.active {
            self.active += 1;
        }
        self.switch_tab(index, window, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_tab((self.active + 1) % self.tabs.len(), window, cx);
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 || ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        let active = if self.active > ix || self.active >= self.tabs.len() {
            self.active.saturating_sub(1)
        } else {
            self.active
        };
        self.active = usize::MAX; // force reload in switch_tab
        self.switch_tab(active, window, cx);
    }

    fn switch_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix == self.active {
            return;
        }
        self.active = ix;
        self.snackbar = None;
        self.sync_view_slider(window, cx);
        self.load(None, window, cx);
        self.focus_view(window, cx);
    }

    fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.read(cx).focus_handle(cx).focus(window, cx);
    }

    /// Ctrl + H: hidden files, in every window (a setting).
    fn toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        Settings::update(cx, |s| s.show_hidden = !s.show_hidden);
    }

    /// What the listing shows follows the settings: hidden files, and the media filter
    /// of the cover flow view.
    fn apply_list_settings(&mut self, cx: &mut Context<Self>) {
        let settings = Settings::get(cx);
        let show_hidden = settings.show_hidden;
        let media = if self.view() == ViewMode::CoverFlow { settings.media_filter } else { fs::MediaFilter::All };
        let changed = {
            let d = self.table.read(cx).delegate();
            d.shows_hidden() != show_hidden || d.media_filter() != media
        };
        if changed {
            self.table.update(cx, |t, cx| {
                let d = t.delegate_mut();
                d.set_show_hidden(show_hidden);
                d.set_media_filter(media);
                t.refresh(cx);
            });
            self.scroll_to_cursor(cx);
            cx.notify();
        }
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        // The builder runs on every render, so the dialog follows changes live.
        window.open_dialog(cx, |dialog, _, cx| {
            let s = i18n::t(cx);
            let settings = Settings::get(cx).clone();
            let section = |label: &'static str, cx: &App| {
                div().pt_2().text_sm().text_color(cx.theme().muted_foreground).child(label)
            };
            dialog
                .title(s.settings)
                .w(px(460.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(section(s.language, cx))
                        .child(
                            RadioGroup::horizontal("language")
                                .children(Language::ALL.map(Language::native_name))
                                .selected_index(Language::ALL.iter().position(|&l| l == settings.language))
                                .on_click(|ix, _, cx| i18n::set_language(Language::ALL[*ix], cx)),
                        )
                        .child(section(s.theme, cx))
                        .child(h_flex().gap_2().children(ThemeChoice::ALL.map(|choice| {
                            Button::new(("theme", choice as usize))
                                .small()
                                .outline()
                                .label(s.theme_name(choice))
                                .selected(settings.theme == choice)
                                .on_click(move |_, _, cx| {
                                    Settings::update(cx, |s| s.theme = choice);
                                    theme::apply(cx);
                                })
                        })))
                        .when(settings.theme == ThemeChoice::Dimmed, |d| {
                            d.child(section(s.accent, cx)).child(h_flex().gap_3().children(Accent::ALL.map(|accent| {
                                let selected = settings.accent == accent;
                                div()
                                    .id(("accent", accent as usize))
                                    .size(px(26.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(if selected { cx.theme().foreground } else { transparent_black() })
                                    .p(px(3.))
                                    .cursor_pointer()
                                    .child(div().size_full().rounded_full().bg(rgb(accent.rgb())))
                                    .tooltip({
                                        let name = s.accent_name(accent);
                                        move |window, cx| gpui_kit::component::tooltip::Tooltip::new(name).build(window, cx)
                                    })
                                    .on_click(move |_, _, cx| {
                                        Settings::update(cx, |s| s.accent = accent);
                                        theme::apply(cx);
                                    })
                            })))
                        })
                        .child(section(s.files_section, cx))
                        .child(
                            Switch::new("show-hidden")
                                .label(s.show_hidden)
                                .checked(settings.show_hidden)
                                .on_click(|checked, _, cx| Settings::update(cx, |s| s.show_hidden = *checked)),
                        )
                        .child(
                            Switch::new("show-extensions")
                                .label(s.show_extensions)
                                .checked(settings.show_extensions)
                                .on_click(|checked, _, cx| Settings::update(cx, |s| s.show_extensions = *checked)),
                        ),
                )
        });
    }

    // ---- rendering --------------------------------------------------------

    /// Custom window title bar (the native one is hidden, see `open_window`).
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        // On Windows the bar is an HTCAPTION area: an unhandled mouse-down enters the
        // native move loop, which swallows the mouse-up and the click.
        let button =
            |button: AnyElement| div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(button);
        let this = cx.entity().downgrade();
        TitleBar::new().pl_1().child(
            h_flex()
                .gap_0p5()
                .child(button(
                    Button::new("app-menu")
                        .ghost()
                        .small()
                        .icon(Icon::new(LucideIcon::Menu))
                        .tooltip(s.menu)
                        .dropdown_menu(move |menu, _, cx| {
                            let s = i18n::t(cx);
                            menu.min_w(px(180.))
                                .item(menu::menu_item(&this, s.settings, LucideIcon::Settings, |fm, window, cx| {
                                    fm.open_settings(&OpenSettings, window, cx)
                                }))
                                .item(menu::menu_item(&this, s.about, LucideIcon::Info, |fm, window, cx| {
                                    fm.open_about(window, cx)
                                }))
                        })
                        .into_any_element(),
                ))
                .child(button(
                    Button::new("toggle-sidebar")
                        .ghost()
                        .small()
                        .icon(Icon::new(LucideIcon::PanelLeft))
                        .tooltip(s.toggle_sidebar)
                        .selected(!self.sidebar_collapsed)
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(&ToggleSidebar, window, cx)))
                        .into_any_element(),
                ))
                .child(img(self.logo.clone()).size_4().ml_1p5())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("FileManager")),
        )
    }

    fn render_table(&self) -> impl IntoElement {
        let bounds = self.table_bounds.clone();
        div()
            .size_full()
            .on_prepaint(move |b, _, _| bounds.set(b))
            .child(DataTable::new(&self.table).bordered(false).small())
    }
}

impl Focusable for FileManager {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FileManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Painting records the drop zones of this frame.
        self.drop_zones.borrow_mut().clear();
        files::forget_stale_cut(cx);
        // First: it measures the sidebar width the grid lays its columns out in.
        let sidebar = self.render_sidebar(window, cx).into_any_element();
        let mode = self.view();
        let content = if mode == ViewMode::CoverFlow {
            v_flex()
                .size_full()
                .child(self.render_cover_flow(window, cx))
                .child(div().relative().flex_1().min_h_0().child(self.render_table()).child(self.render_flow_handle(cx)))
                .into_any_element()
        } else if mode.is_table() {
            self.render_table().into_any_element()
        } else {
            self.render_grid(mode, window, cx)
        };
        v_flex()
            .id("file-manager")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_up))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::focus_filter))
            .on_action(cx.listener(Self::edit_path))
            .on_action(cx.listener(Self::toggle_hidden))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::new_folder))
            // Mouse back/forward buttons.
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|this, _, window, cx| this.go_back(&GoBack, window, cx)))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Forward), cx.listener(|this, _, window, cx| this.go_forward(&GoForward, window, cx)))
            .on_drag_move(cx.listener(Self::tab_drag_moved))
            .on_drag_move(cx.listener(Self::sidebar_resize_moved))
            .on_drag_move(cx.listener(Self::flow_resize_moved))
            // A tab drag over this window from another one keeps the mouse captured
            // there: a move arriving here means it is over.
            .on_mouse_move(cx.listener(|this, _, _, cx| this.clear_incoming(cx)))
            // End of a drag: a tab released outside the window moves or detaches; any
            // other release ends tab and sidebar drags.
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, window, cx| {
                this.release_tab_outside(window, cx);
                this.end_sidebar_resize(cx);
                this.end_flow_resize(cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.end_tab_drag(cx);
                this.end_sidebar_resize(cx);
                this.end_flow_resize(cx);
            }))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(self.render_title_bar(cx))
            .child(
                h_flex().flex_1().min_h_0().child(sidebar).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.render_tabs(cx))
                        .child(self.render_address_bar(cx))
                        .child(
                            h_flex()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    div()
                                        .relative()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        // Before the files: capture-phase listeners run in paint
                                        // order, and the table's scroller stops the wheel there.
                                        .child(self.zoom_listener(cx))
                                        .child(self.render_files(content, cx))
                                        .children(self.render_snackbar(cx)),
                                )
                                .when(self.preview_pane, |d| d.child(self.render_preview_pane(cx))),
                        )
                        .child(self.render_status(cx)),
                ),
            )
            .when_some(self.context_menu.as_ref(), |d, m| d.child(m.menu.clone()))
    }
}
