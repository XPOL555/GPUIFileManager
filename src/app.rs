//! Main window: title bar, sidebar (places + drives), tab strip, address bar, file table,
//! status bar, and the app's own context menu. Tabs can be dragged between windows.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::i18n::{self, Language};
use crate::settings::Settings;
use crate::table::FileTable;
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
        OpenSettings
    ]
);

const CONTEXT: &str = "FileManager";
/// Address-bar completions shown at once. Keeps the list short enough to never scroll.
const MAX_SUGGESTIONS: usize = 12;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", OpenSelected, Some(CONTEXT)),
        KeyBinding::new("backspace", GoUp, Some(CONTEXT)),
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
    ]);
}

#[derive(Clone)]
pub struct Tab {
    path: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl Tab {
    pub fn new(path: PathBuf) -> Self {
        Self { path, back: Vec::new(), forward: Vec::new() }
    }

    fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
            .into()
    }
}

// ---- windows and tab dragging ------------------------------------------------

/// Every open window, so a tab released outside its own window can find the window under the cursor.
#[derive(Default)]
struct OpenWindows(Vec<OpenWindow>);

struct OpenWindow {
    view: WeakEntity<FileManager>,
    handle: AnyWindowHandle,
    hwnd: isize,
}

impl Global for OpenWindows {}

/// Payload of a tab drag, also rendered as the drag preview.
#[derive(Clone)]
struct DraggedTab {
    source: EntityId,
    ix: usize,
    title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_3()
            .h(px(28.))
            .rounded(cx.theme().radius)
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_md()
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(Icon::new(IconName::Folder).small().text_color(cx.theme().warning))
            .child(self.title.clone())
    }
}

/// The tab being dragged, from drag start until the button is released. GPUI drops
/// its own drag state on a release that no drop target takes, which is exactly the
/// case (released outside the window) that has to detach the tab.
#[derive(Default)]
struct TabDragState(Option<DraggedTab>);

impl Global for TabDragState {}

/// Places a window so the cursor sits over its first tab, as if it was being held by it.
fn place_under_cursor(hwnd: isize, cursor: shell::ScreenPoint, scale: f32) {
    let grab = ((300. * scale) as i32, (52. * scale) as i32);
    shell::move_window(hwnd, (cursor.0 - grab.0, cursor.1 - grab.1), cursor);
}

/// Opens a window holding `tabs`, placed under `cursor` (screen pixels) when given.
pub fn open_window(tabs: Vec<Tab>, cursor: Option<shell::ScreenPoint>, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    // The app draws its own title bar (`component::TitleBar`), with the settings menu on the left.
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
            if let (Some(hwnd), Some(cursor)) = (hwnd, cursor) {
                place_under_cursor(hwnd, cursor, window.scale_factor());
            }
            view.read(cx).table.read(cx).focus_handle(cx).focus(window, cx);
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

// ---- view -------------------------------------------------------------------

struct Suggestion {
    label: SharedString,
    path: PathBuf,
    is_drive: bool,
}

struct OpenMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    _dismiss: Subscription,
}

/// First click of a possible sidebar double-click, kept so the second click can undo it.
struct SidebarClick {
    path: PathBuf,
    tab: usize,
    before: Tab,
}

pub struct FileManager {
    focus: FocusHandle,
    logo: std::sync::Arc<Image>,
    tabs: Vec<Tab>,
    active: usize,
    table: Entity<TableState<FileTable>>,
    filter: Entity<InputState>,
    path_input: Entity<InputState>,
    editing_path: bool,
    /// Address-bar completions for the text being typed.
    suggestions: Vec<Suggestion>,
    suggestion_ix: Option<usize>,
    suggest_generation: u64,
    context_menu: Option<OpenMenu>,
    sidebar_click: Option<SidebarClick>,
    error: Option<SharedString>,
    places: Vec<(shell::KnownFolder, PathBuf)>,
    drives: Vec<shell::Drive>,
    /// Bumped on each navigation so late results from a previous listing are dropped.
    load_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl FileManager {
    fn new(tabs: Vec<Tab>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        assert!(!tabs.is_empty(), "a window needs at least one tab");
        let table = cx.new(|cx| TableState::new(FileTable::new(), window, cx).row_selectable(true));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t(cx).filter));
        let path_input = cx.new(|cx| InputState::new(window, cx));

        let subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
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
                    this.table.update(cx, |t, cx| {
                        if t.delegate().summary() != (0, 0) {
                            t.set_selected_row(0, cx);
                        }
                    });
                    this.focus_table(window, cx);
                }
                _ => {}
            }),
            cx.subscribe_in(&path_input, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.update_suggestions(text, cx);
                }
                InputEvent::PressEnter { .. } => {
                    let target = match this.suggestion_ix.and_then(|ix| this.suggestions.get(ix)) {
                        Some(s) => s.path.clone(),
                        None => PathBuf::from(input.read(cx).value().trim()),
                    };
                    this.finish_edit_path(window, cx);
                    this.navigate(target, true, window, cx);
                }
                InputEvent::Blur => {
                    this.editing_path = false;
                    this.suggestions.clear();
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe_global_in::<Settings>(window, |this, window, cx| {
                let placeholder = i18n::t(cx).filter;
                this.filter.update(cx, |f, cx| f.set_placeholder(placeholder, window, cx));
            }),
        ];

        let mut this = Self {
            focus: cx.focus_handle(),
            logo: crate::assets::logo(),
            tabs,
            active: 0,
            table,
            filter,
            path_input,
            editing_path: false,
            suggestions: Vec::new(),
            suggestion_ix: None,
            suggest_generation: 0,
            context_menu: None,
            sidebar_click: None,
            error: None,
            places: shell::known_folders(),
            drives: shell::drives(),
            load_generation: 0,
            _subscriptions: subscriptions,
        };
        this.load(None, window, cx);
        this
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn focus_table(&self, window: &mut Window, cx: &mut App) {
        self.table.read(cx).focus_handle(cx).focus(window, cx);
    }

    // ---- navigation -------------------------------------------------------

    /// Navigates the active tab. `record` pushes the current folder on the back stack.
    fn navigate(&mut self, path: PathBuf, record: bool, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[self.active];
        if path == tab.path {
            return self.load(None, window, cx);
        }
        let previous = std::mem::replace(&mut tab.path, path);
        if record {
            tab.back.push(previous.clone());
            tab.forward.clear();
        }
        // Coming back up from a child: reselect the folder we came from.
        let select = previous
            .parent()
            .filter(|p| *p == self.tab().path)
            .and(previous.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        self.load(select, window, cx);
    }

    /// Lists the active tab's folder on a background thread.
    fn load(&mut self, select: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.tab().path.clone();
        self.load_generation += 1;
        let generation = self.load_generation;
        self.error = None;
        self.filter.update(cx, |f, cx| f.set_value("", window, cx));
        self.table.update(cx, |t, cx| {
            t.delegate_mut().loading = true;
            t.delegate_mut().set_filter("");
            cx.notify();
        });

        let listing = cx.background_spawn(async move { fs::list(&path) });
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
                this.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    d.loading = false;
                    d.set_entries(entries);
                    let row = select.as_deref().and_then(|n| t.delegate().row_of(n));
                    t.refresh(cx);
                    match row {
                        Some(row) => {
                            t.set_selected_row(row, cx);
                            t.scroll_to_row(row, cx);
                        }
                        None => t.clear_selection(cx),
                    }
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn selected_entry(&self, cx: &App) -> Option<fs::Entry> {
        let t = self.table.read(cx);
        t.selected_row().and_then(|r| t.delegate().entry(r).cloned())
    }

    fn open_entry(&mut self, entry: fs::Entry, window: &mut Window, cx: &mut Context<Self>) {
        if entry.is_dir {
            self.navigate(entry.path, true, window, cx);
        } else {
            shell::open(&entry.path);
        }
    }

    fn on_table_event(
        &mut self,
        table: &Entity<TableState<FileTable>>,
        ev: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            TableEvent::DoubleClickedRow(row) => {
                if let Some(e) = table.read(cx).delegate().entry(*row).cloned() {
                    self.open_entry(e, window, cx);
                }
            }
            // No `RightClickedRow` here: gpui-component also emits `RightClickedRow(None)`
            // from `set_selected_row` (every left click / arrow key) just to clear its
            // highlight, so it cannot tell "right click on empty space" apart.
            // See `on_table_right_mouse_down`.
            _ => {}
        }
    }

    /// Sidebar: a click navigates the active tab, a double click opens the folder in a
    /// new tab instead. The first click of a double click has already navigated, so the
    /// second one restores the tab as it was before.
    fn sidebar_click(&mut self, path: PathBuf, clicks: usize, window: &mut Window, cx: &mut Context<Self>) {
        match clicks {
            1 => {
                self.sidebar_click = Some(SidebarClick { path: path.clone(), tab: self.active, before: self.tab().clone() });
                self.navigate(path, true, window, cx);
            }
            2 => {
                if let Some(first) = self.sidebar_click.take().filter(|c| c.path == path && c.tab == self.active) {
                    self.tabs[first.tab] = first.before;
                }
                self.open_tab(path, window, cx);
            }
            _ => {}
        }
    }

    // ---- context menu -----------------------------------------------------

    /// Bubble phase of a right mouse-down anywhere over the table. The row's own
    /// handler (child, runs first) has already set `right_clicked_row`; the capture
    /// handler cleared it beforehand, so `None` means empty space → folder background menu.
    /// Shift+right-click skips the app menu and shows Explorer's.
    fn on_table_right_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let t = self.table.read(cx);
        let entry = t.right_clicked_row().and_then(|r| t.delegate().entry(r)).cloned();
        let at = shell::cursor_pos();
        if ev.modifiers.shift {
            let paths = entry.map(|e| vec![e.path]).unwrap_or_default();
            return self.open_native_menu(paths, at, window, cx);
        }
        self.open_menu(entry, ev.position, at, window, cx);
    }

    fn open_menu(
        &mut self,
        entry: Option<fs::Entry>,
        position: Point<Pixels>,
        at: shell::ScreenPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        let s = i18n::t(cx);
        let folder = self.tab().path.clone();
        let show_hidden = self.table.read(cx).delegate().show_hidden();
        let table_focus = self.table.read(cx).focus_handle(cx);

        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            let mut menu = menu.action_context(table_focus).min_w(px(220.));
            let target = entry.as_ref().map_or(&folder, |e| &e.path).clone();
            menu = match &entry {
                Some(e) => {
                    let open_icon = if e.is_dir { LucideIcon::FolderOpen } else { LucideIcon::ExternalLink };
                    let e = e.clone();
                    let is_dir = e.is_dir;
                    menu.item(menu_item(&this, s.open, open_icon, move |fm, window, cx| {
                        fm.open_entry(e.clone(), window, cx)
                    }))
                    .when(is_dir, |menu| open_elsewhere_items(menu, &this, s, &target))
                }
                None => menu
                    .item(menu_item(&this, s.refresh, LucideIcon::RefreshCw, |fm, window, cx| {
                        fm.refresh(&Refresh, window, cx)
                    }))
                    .item(
                        menu_item(&this, s.show_hidden, LucideIcon::Eye, |fm, window, cx| {
                            fm.toggle_hidden(&ToggleHidden, window, cx)
                        })
                        .checked(show_hidden),
                    )
                    .separator()
                    .map(|menu| open_elsewhere_items(menu, &this, s, &target)),
            };
            let copied = target.clone();
            let props = target.clone();
            let native_paths: Vec<PathBuf> = entry.iter().map(|e| e.path.clone()).collect();
            menu.separator()
                .item(menu_item(&this, s.copy_path, LucideIcon::Copy, move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copied.to_string_lossy().into_owned()))
                }))
                .item(menu_item(&this, s.properties, LucideIcon::Info, move |_, _, _| shell::show_properties(&props)))
                .separator()
                .item({
                    let this = this.clone();
                    PopupMenuItem::element(move |_, cx| {
                        div().text_xs().text_color(cx.theme().muted_foreground).child(s.more_options)
                    })
                    .icon(Icon::new(LucideIcon::Ellipsis).xsmall())
                    .on_click(move |_, window, cx| {
                        let paths = native_paths.clone();
                        this.update(cx, |fm, cx| fm.open_native_menu(paths, at, window, cx)).ok();
                    })
                })
        });

        let dismiss = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.context_menu = None;
            cx.notify();
        });
        menu.focus_handle(cx).focus(window, cx);
        self.context_menu = Some(OpenMenu { menu, position, _dismiss: dismiss });
        cx.notify();
    }

    /// Explorer's own menu. It runs a nested Win32 modal loop; running it inside an
    /// event handler would re-enter GPUI while `App` is borrowed, so it is deferred to a
    /// foreground task where no GPUI borrow is held. The folder is reloaded afterwards
    /// (the command may have renamed, deleted or created files).
    fn open_native_menu(
        &mut self,
        paths: Vec<PathBuf>,
        at: shell::ScreenPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = self.tab().path.clone();
        cx.spawn_in(window, async move |this, cx| {
            if let Err(err) = shell::show_context_menu(&folder, &paths, at) {
                eprintln!("context menu: {err:?}");
            }
            this.update_in(cx, |this, window, cx| this.refresh(&Refresh, window, cx)).ok();
        })
        .detach();
    }

    // ---- address bar ------------------------------------------------------

    fn edit_path(&mut self, _: &EditPath, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.tab().path.to_string_lossy().into_owned();
        self.editing_path = true;
        self.suggestions.clear();
        self.suggestion_ix = None;
        self.path_input.update(cx, |i, cx| {
            i.set_value(p, window, cx);
            i.select_all(window, cx);
        });
        self.path_input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn finish_edit_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_path = false;
        self.suggestions.clear();
        self.suggestion_ix = None;
        self.suggest_generation += 1;
        self.focus_table(window, cx);
        cx.notify();
    }

    /// Completes the last path segment: drives while no separator is typed, otherwise
    /// the subfolders of the typed parent, listed on a background thread.
    fn update_suggestions(&mut self, text: String, cx: &mut Context<Self>) {
        self.suggest_generation += 1;
        let generation = self.suggest_generation;
        self.suggestion_ix = None;
        if !self.editing_path || text == self.tab().path.to_string_lossy() {
            self.suggestions.clear();
            return cx.notify();
        }
        let (dir, prefix) = fs::split_for_completion(&text);
        let Some(dir) = dir else {
            let prefix = prefix.to_uppercase();
            self.suggestions = self
                .drives
                .iter()
                .filter(|d| d.root.to_string_lossy().to_uppercase().starts_with(&prefix))
                .map(|d| Suggestion { label: drive_label(d).into(), path: d.root.clone(), is_drive: true })
                .collect();
            return cx.notify();
        };
        let (dir, prefix) = (PathBuf::from(dir), prefix.to_string());
        let names = cx.background_spawn({
            let dir = dir.clone();
            async move { fs::complete_dirs(&dir, &prefix, MAX_SUGGESTIONS) }
        });
        cx.spawn(async move |this, cx| {
            let names = names.await;
            this.update(cx, |this, cx| {
                if this.suggest_generation != generation || !this.editing_path {
                    return;
                }
                this.suggestions = names
                    .into_iter()
                    .map(|name| Suggestion { path: dir.join(&name), label: name.into(), is_drive: false })
                    .collect();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn move_suggestion(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.suggestions.len() as isize;
        let next = match self.suggestion_ix {
            None if delta > 0 => 0,
            None => n - 1,
            Some(ix) => (ix as isize + delta).rem_euclid(n),
        };
        self.suggestion_ix = Some(next as usize);
        cx.notify();
    }

    /// Tab: completes the text with the highlighted (or first) suggestion and keeps
    /// completing inside it.
    fn accept_suggestion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.suggestions.get(self.suggestion_ix.unwrap_or(0)) else { return };
        let mut text = s.path.to_string_lossy().into_owned();
        if !text.ends_with(['\\', '/']) {
            text.push('\\');
        }
        // `replace_all` leaves the cursor at the end and emits `Change`, which refreshes the list.
        self.path_input.update(cx, |i, cx| i.replace_all(text, window, cx));
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
        if let Some(parent) = self.tab().path.parent().map(Path::to_path_buf) {
            self.navigate(parent, true, window, cx);
        }
    }

    fn refresh(&mut self, _: &Refresh, window: &mut Window, cx: &mut Context<Self>) {
        let keep = self.selected_entry(cx).map(|e| e.name.to_string());
        self.drives = shell::drives();
        self.load(keep, window, cx);
    }

    fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = self.selected_entry(cx) {
            self.open_entry(e, window, cx);
        }
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(self.tab().path.clone(), window, cx);
    }

    fn new_window(&mut self, _: &NewWindow, _: &mut Window, cx: &mut Context<Self>) {
        let tab = Tab::new(self.tab().path.clone());
        cx.defer(move |cx| open_window(vec![tab], None, cx));
    }

    /// Opens `path` in a new tab and activates it.
    fn open_tab(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_tab(Tab::new(path), window, cx);
    }

    fn insert_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.push(tab);
        self.switch_tab(self.tabs.len() - 1, window, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_tab((self.active + 1) % self.tabs.len(), window, cx);
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 {
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
        self.load(None, window, cx);
    }

    /// A tab dropped on this window's tab strip: reorder. `to` is the tab it was dropped
    /// on, `None` for the empty end of the strip.
    fn drop_tab(&mut self, drag: &DraggedTab, to: Option<usize>, cx: &mut Context<Self>) {
        cx.default_global::<TabDragState>().0 = None;
        // Drags from other windows never land here: the source window captures the
        // mouse, see `release_tab_outside`.
        let n = self.tabs.len();
        if drag.source != cx.entity_id() || drag.ix >= n {
            return;
        }
        let (from, to) = (drag.ix, to.unwrap_or(n - 1).min(n - 1));
        if from == to {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        let a = self.active;
        self.active = match () {
            _ if a == from => to,
            _ if from < a && to >= a => a - 1,
            _ if from > a && to <= a => a + 1,
            _ => a,
        };
        cx.notify();
    }

    /// The mouse was released outside this window during a tab drag (the window keeps
    /// the mouse captured while a button is down, so the release still arrives here).
    /// Over another of our windows the tab moves there; anywhere else it becomes a new
    /// window. Dragging the only tab moves the window itself.
    fn release_tab_outside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = cx.default_global::<TabDragState>().0.take() else { return };
        if drag.source != cx.entity_id() || drag.ix >= self.tabs.len() {
            return;
        }
        let cursor = shell::cursor_pos();
        let under = shell::root_window_at(cursor);
        let own = shell::hwnd(window).unwrap_or(0);
        let target = cx
            .default_global::<OpenWindows>()
            .0
            .iter()
            .find(|w| w.hwnd == under && under != own && w.view.upgrade().is_some())
            .map(|w| (w.view.clone(), w.handle));

        let tab = self.tabs[drag.ix].clone();
        match target {
            Some((view, handle)) => {
                if self.tabs.len() == 1 {
                    window.remove_window();
                } else {
                    self.close_tab_at(drag.ix, window, cx);
                }
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        view.update(cx, |fm, cx| fm.insert_tab(tab, window, cx)).ok();
                        window.activate_window();
                    });
                });
            }
            None if self.tabs.len() > 1 => {
                self.close_tab_at(drag.ix, window, cx);
                cx.defer(move |cx| open_window(vec![tab], Some(cursor), cx));
            }
            None => place_under_cursor(own, cursor, window.scale_factor()),
        }
    }

    fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            t.delegate_mut().toggle_hidden();
            t.refresh(cx);
        });
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        // The builder runs on every render, so the dialog follows language changes live.
        window.open_dialog(cx, |dialog, _, cx| {
            let s = i18n::t(cx);
            let current = i18n::language(cx);
            dialog.title(s.settings).w(px(420.)).child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(s.language))
                    .child(
                        RadioGroup::vertical("language")
                            .children(Language::ALL.map(Language::native_name))
                            .selected_index(Language::ALL.iter().position(|&l| l == current))
                            .on_click(|ix, _, cx| i18n::set_language(Language::ALL[*ix], cx)),
                    ),
            )
        });
    }

    // ---- rendering --------------------------------------------------------

    /// Custom window title bar (the native one is hidden, see `open_window`).
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().pl_1().child(
            h_flex()
                .gap_1()
                .child(
                    // On Windows the bar is an HTCAPTION area: an unhandled mouse-down
                    // enters the native move loop, which swallows the mouse-up and the click.
                    div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
                        Button::new("settings")
                            .ghost()
                            .small()
                            .icon(Icon::new(LucideIcon::Menu))
                            .on_click(cx.listener(|this, _, window, cx| this.open_settings(&OpenSettings, window, cx))),
                    ),
                )
                .child(img(self.logo.clone()).size_4().ml_1())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("FileManager")),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let current = self.tab().path.clone();
        let item = |id: SharedString, icon: IconName, label: String, path: PathBuf, cx: &mut Context<Self>| {
            let active = current == path;
            let middle = path.clone();
            h_flex()
                .id(id)
                .gap_2()
                .px_3()
                .py_1()
                .rounded(cx.theme().radius)
                .when(active, |d| d.bg(cx.theme().accent))
                .hover(|d| d.bg(cx.theme().accent.opacity(0.6)))
                .cursor_pointer()
                .child(Icon::new(icon).small().text_color(cx.theme().muted_foreground))
                .child(div().truncate().child(label))
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    this.sidebar_click(path.clone(), ev.click_count(), window, cx)
                }))
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| this.open_tab(middle.clone(), window, cx)),
                )
        };
        let heading = |s: &'static str, cx: &Context<Self>| {
            div().px_3().pt_3().pb_1().text_xs().text_color(cx.theme().muted_foreground).child(s)
        };
        let s = i18n::t(cx);

        let mut col = v_flex()
            .id("sidebar")
            .w(px(230.))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scroll()
            .p_1()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(heading(s.places, cx));
        for (folder, path) in self.places.clone() {
            let id = format!("place-{folder:?}").into();
            col = col.child(item(id, IconName::Folder, s.known_folder(folder).into(), path, cx));
        }
        col = col.child(heading(s.drives, cx));
        for d in &self.drives {
            let root = d.root.to_string_lossy().into_owned();
            let used = if d.total > 0 { 1.0 - d.free as f32 / d.total as f32 } else { 0.0 };
            let bar = if used > 0.9 { cx.theme().danger } else { cx.theme().primary };
            col = col.child(
                v_flex()
                    .child(item(format!("drive-{root}").into(), IconName::HardDrive, drive_label(d), d.root.clone(), cx))
                    .when(d.total > 0, |c| {
                        c.child(
                            div().mx_3().ml(px(36.)).mb_1().h(px(3.)).rounded_full().bg(cx.theme().muted).child(
                                div().h_full().rounded_full().w(relative(used)).bg(bar),
                            ),
                        )
                    }),
            );
        }
        col
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let source = cx.entity_id();
        let mut strip = h_flex()
            .id("tab-strip")
            .h(px(36.))
            .px_1()
            .gap_1()
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.border)
            .on_drop(cx.listener(|this, drag: &DraggedTab, _, cx| this.drop_tab(drag, None, cx)));
        for (ix, tab) in self.tabs.iter().enumerate() {
            let active = ix == self.active;
            strip = strip.child(
                h_flex()
                    .id(("tab", ix))
                    .gap_2()
                    .pl_3()
                    .pr_1()
                    .h(px(28.))
                    .max_w(px(220.))
                    .rounded(cx.theme().radius)
                    .when(active, |d| d.bg(cx.theme().background))
                    .hover(|d| d.bg(cx.theme().accent))
                    .cursor_pointer()
                    .child(Icon::new(IconName::Folder).small().text_color(cx.theme().warning))
                    .child(div().truncate().child(tab.title()))
                    .when(self.tabs.len() > 1, |d| {
                        d.child(
                            Button::new(("close-tab", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab_at(ix, window, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.switch_tab(ix, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab_at(ix, window, cx)),
                    )
                    .on_drag(DraggedTab { source, ix, title: tab.title() }, |drag, _, _, cx| {
                        cx.default_global::<TabDragState>().0 = Some(drag.clone());
                        cx.new(|_| drag.clone())
                    })
                    .drag_over::<DraggedTab>(|style, _, _, cx| style.bg(cx.theme().accent))
                    .on_drop(cx.listener(move |this, drag: &DraggedTab, _, cx| this.drop_tab(drag, Some(ix), cx))),
            );
        }
        strip.child(
            Button::new("new-tab")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .on_click(cx.listener(|this, _, window, cx| this.new_tab(&NewTab, window, cx))),
        )
    }

    fn render_address_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab();
        let nav = |id: &'static str, icon: IconName, enabled: bool, action: Box<dyn Action>| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .disabled(!enabled)
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        };

        let location = if self.editing_path {
            // While suggestions are shown, Up/Down/Tab/Escape drive the list instead of the input.
            div()
                .flex_1()
                .relative()
                .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                    if !this.suggestions.is_empty() {
                        cx.stop_propagation();
                        this.move_suggestion(1, cx);
                    }
                }))
                .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                    if !this.suggestions.is_empty() {
                        cx.stop_propagation();
                        this.move_suggestion(-1, cx);
                    }
                }))
                .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                    cx.stop_propagation();
                    this.accept_suggestion(window, cx);
                }))
                .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                    cx.stop_propagation();
                    this.finish_edit_path(window, cx);
                }))
                .child(Input::new(&self.path_input).small())
                .when(!self.suggestions.is_empty(), |d| d.child(self.render_suggestions(cx)))
                .into_any_element()
        } else {
            // Breadcrumb: one clickable segment per ancestor; clicking the empty space
            // after them switches to the editable text path.
            let mut crumbs = h_flex().id("breadcrumb").flex_1().h_full().gap_0p5().overflow_hidden().cursor_text();
            let ancestors: Vec<PathBuf> = tab.path.ancestors().map(Path::to_path_buf).collect();
            for (i, p) in ancestors.into_iter().rev().enumerate() {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string_lossy().trim_end_matches('\\').to_string());
                if i > 0 {
                    crumbs = crumbs.child(Icon::new(IconName::ChevronRight).xsmall().text_color(cx.theme().muted_foreground));
                }
                crumbs = crumbs.child(
                    Button::new(("crumb", i)).ghost().small().label(name).on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.navigate(p.clone(), true, window, cx);
                        },
                    )),
                );
            }
            crumbs
                .on_click(cx.listener(|this, _, window, cx| this.edit_path(&EditPath, window, cx)))
                .into_any_element()
        };

        h_flex()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(nav("back", IconName::ArrowLeft, !tab.back.is_empty(), Box::new(GoBack)))
            .child(nav("forward", IconName::ArrowRight, !tab.forward.is_empty(), Box::new(GoForward)))
            .child(nav("up", IconName::ArrowUp, tab.path.parent().is_some(), Box::new(GoUp)))
            .child(nav("refresh", IconName::RefreshCw, true, Box::new(Refresh)))
            .child(location)
            .child(
                div().w(px(220.)).child(
                    Input::new(&self.filter).small().cleanable(true).prefix(Icon::new(IconName::Search).small()),
                ),
            )
    }

    /// Completion list under the path input. Deferred so it paints over the table.
    fn render_suggestions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.suggestions.iter().enumerate().map(|(ix, s)| {
            let path = s.path.clone();
            let icon = if s.is_drive { IconName::HardDrive } else { IconName::Folder };
            h_flex()
                .id(("suggestion", ix))
                .gap_2()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .cursor_pointer()
                .when(self.suggestion_ix == Some(ix), |d| d.bg(theme.accent))
                .hover(|d| d.bg(theme.accent.opacity(0.6)))
                .child(Icon::new(icon).small().text_color(if s.is_drive { theme.muted_foreground } else { theme.warning }))
                .child(div().truncate().child(s.label.clone()))
                // Mouse-down, not click: the input blurs on the way and would hide the list first.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.finish_edit_path(window, cx);
                        this.navigate(path.clone(), true, window, cx);
                    }),
                )
        });
        div().absolute().top_full().left_0().right_0().mt_1().child(
            deferred(
                v_flex()
                    .id("path-suggestions")
                    .occlude()
                    .p_1()
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(theme.radius)
                    .shadow_md()
                    .children(rows),
            )
            .with_priority(1),
        )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.table.read(cx);
        let s = i18n::t(cx);
        let (dirs, files) = t.delegate().summary();
        let selected = t
            .selected_row()
            .and_then(|r| t.delegate().entry(r))
            .filter(|e| !e.is_dir)
            .map(|e| format!(" · {}: {}", s.selected, fs::format_size(e.size)))
            .unwrap_or_default();
        let text = match &self.error {
            Some(err) => err.to_string(),
            None => format!("{}{selected}", (s.counts)(dirs, files)),
        };
        h_flex()
            .h(px(24.))
            .px_3()
            .text_xs()
            .border_t_1()
            .border_color(cx.theme().border)
            .text_color(if self.error.is_some() { cx.theme().danger } else { cx.theme().muted_foreground })
            .child(text)
    }
}

/// A context-menu entry whose handler runs on the `FileManager` behind `this`.
fn menu_item(
    this: &WeakEntity<FileManager>,
    label: &'static str,
    icon: impl Into<Icon>,
    handler: impl Fn(&mut FileManager, &mut Window, &mut Context<FileManager>) + 'static,
) -> PopupMenuItem {
    let this = this.clone();
    PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
        this.update(cx, |fm, cx| handler(fm, window, cx)).ok();
    })
}

/// "Open in new tab" / "Open in new window" for a folder.
fn open_elsewhere_items(
    menu: PopupMenu,
    this: &WeakEntity<FileManager>,
    s: &'static i18n::Strings,
    folder: &Path,
) -> PopupMenu {
    let (tab_path, window_path) = (folder.to_path_buf(), folder.to_path_buf());
    menu.item(menu_item(this, s.open_new_tab, LucideIcon::SquarePlus, move |fm, window, cx| {
        fm.open_tab(tab_path.clone(), window, cx)
    }))
    .item(menu_item(this, s.open_new_window, LucideIcon::AppWindow, move |_, _, cx| {
        let tab = Tab::new(window_path.clone());
        cx.defer(move |cx| open_window(vec![tab], None, cx));
    }))
}

impl Focusable for FileManager {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FileManager {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            // Mouse back/forward buttons.
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|this, _, window, cx| this.go_back(&GoBack, window, cx)))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Forward), cx.listener(|this, _, window, cx| this.go_forward(&GoForward, window, cx)))
            // End of a tab drag: outside the window → detach; inside but not on the strip → cancel.
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, window, cx| this.release_tab_outside(window, cx)))
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.default_global::<TabDragState>().0 = None)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(self.render_title_bar(cx))
            .child(
                h_flex().flex_1().min_h_0().child(self.render_sidebar(cx)).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.render_tabs(cx))
                        .child(self.render_address_bar(cx))
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .capture_any_mouse_down(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    if ev.button == MouseButton::Right {
                                        this.table.update(cx, |t, cx| t.set_right_clicked_row(None, cx));
                                    }
                                }))
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                        this.on_table_right_mouse_down(ev, window, cx)
                                    }),
                                )
                                .child(DataTable::new(&self.table).bordered(false).small()),
                        )
                        .child(self.render_status(cx)),
                ),
            )
            .when_some(self.context_menu.as_ref(), |d, m| {
                d.child(
                    deferred(anchored().position(m.position).snap_to_window_with_margin(px(8.)).child(m.menu.clone()))
                        .with_priority(1),
                )
            })
    }
}
