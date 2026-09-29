//! Main window. This module holds the window, navigation and actions; the areas of
//! the window live in submodules: `sidebar` (favorites, drives), `tabs` (tab strip,
//! dragging tabs between windows), `address_bar`, `views` (view modes, icon grid,
//! preview pane, info bar) and `menu` (context menus).

mod address_bar;
mod menu;
mod sidebar;
mod tabs;
mod views;

pub use views::ViewMode;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::drag_preview::DragPreviewWindow;
use crate::i18n::{self, Language};
use crate::settings::Settings;
use crate::table::FileTable;
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
        GridLeft,
        GridRight,
        GridUp,
        GridDown,
        GridHome,
        GridEnd
    ]
);

const CONTEXT: &str = "FileManager";
const GRID_CONTEXT: &str = "FileGrid";

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
        KeyBinding::new("ctrl-b", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("alt-p", TogglePreview, Some(CONTEXT)),
        KeyBinding::new("left", GridLeft, Some(GRID_CONTEXT)),
        KeyBinding::new("right", GridRight, Some(GRID_CONTEXT)),
        KeyBinding::new("up", GridUp, Some(GRID_CONTEXT)),
        KeyBinding::new("down", GridDown, Some(GRID_CONTEXT)),
        KeyBinding::new("home", GridHome, Some(GRID_CONTEXT)),
        KeyBinding::new("end", GridEnd, Some(GRID_CONTEXT)),
    ]);
}

#[derive(Clone)]
pub struct Tab {
    path: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    view: ViewMode,
}

impl Tab {
    pub fn new(path: PathBuf, view: ViewMode) -> Self {
        Self { path, back: Vec::new(), forward: Vec::new(), view }
    }

    fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
            .into()
    }
}

/// Every open window, so a tab released outside its own window can find the window under the cursor.
#[derive(Default)]
struct OpenWindows(Vec<OpenWindow>);

struct OpenWindow {
    view: WeakEntity<FileManager>,
    handle: AnyWindowHandle,
    hwnd: isize,
}

impl Global for OpenWindows {}

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
            if let (Some(hwnd), Some(cursor)) = (hwnd, cursor) {
                place_under_cursor(hwnd, cursor, window.scale_factor());
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

    // Views (see `views`).
    grid_focus: FocusHandle,
    grid_scroll: UniformListScrollHandle,
    grid_selected: Option<usize>,
    grid_columns: usize,
    zoom_pixels: f32,
    view_slider: Entity<SliderState>,
    preview_pane: bool,

    // Tab dragging (see `tabs`).
    tab_strip: Rc<std::cell::Cell<Bounds<Pixels>>>,
    drag_preview: Option<DragPreviewWindow>,
    drag_target: Option<(WeakEntity<FileManager>, AnyWindowHandle)>,
    /// A tab from another window hovers over this one: its title.
    incoming_tab: Option<SharedString>,

    _subscriptions: Vec<Subscription>,
}

impl FileManager {
    fn new(tabs: Vec<Tab>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        assert!(!tabs.is_empty(), "a window needs at least one tab");
        let settings = Settings::get(cx).clone();
        let table = cx.new(|cx| TableState::new(FileTable::new(), window, cx).row_selectable(true));
        let this = cx.entity().downgrade();
        table.update(cx, |t, _| {
            t.delegate_mut().set_on_toggle(Rc::new(move |path, expand, window, cx| {
                this.update(cx, |fm, cx| fm.toggle_folder(path, expand, window, cx)).ok();
            }))
        });
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t(cx).filter));
        let path_input = cx.new(|cx| InputState::new(window, cx));
        let mode = tabs[0].view;
        let view_slider =
            cx.new(|_| SliderState::new().min(0.).max(5.).step(1.).default_value(mode.slider_value()));

        let subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
            cx.subscribe_in(&filter, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let q = input.read(cx).value();
                    this.table.update(cx, |t, cx| {
                        t.delegate_mut().set_filter(&q);
                        t.refresh(cx);
                    });
                    this.grid_selected = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if this.table.read(cx).delegate().len() > 0 {
                        this.select_row(0, cx);
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
            }),
        ];

        let mut this = Self {
            focus: cx.focus_handle(),
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
            grid_focus: cx.focus_handle(),
            grid_scroll: UniformListScrollHandle::new(),
            grid_selected: None,
            grid_columns: 1,
            zoom_pixels: 0.,
            view_slider,
            preview_pane: settings.preview_pane,
            tab_strip: Rc::default(),
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

    /// Keyboard focus to the file view of the current mode.
    fn focus_view(&self, window: &mut Window, cx: &mut App) {
        if self.view().is_table() {
            self.table.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.grid_focus.focus(window, cx);
        }
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
        let tree = self.view() == ViewMode::Tree;
        self.load_generation += 1;
        let generation = self.load_generation;
        self.error = None;
        self.grid_selected = None;
        self.grid_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.filter.update(cx, |f, cx| f.set_value("", window, cx));
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.loading = true;
            d.set_tree(tree);
            d.set_filter("");
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
                let row = this.table.update(cx, |t, cx| {
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
                    row
                });
                if let Some(row) = row {
                    this.grid_selected = Some(row);
                    this.scroll_grid_to(row);
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

    fn selected_row(&self, cx: &App) -> Option<usize> {
        if self.view().is_table() { self.table.read(cx).selected_row() } else { self.grid_selected }
    }

    fn selected_entry(&self, cx: &App) -> Option<fs::Entry> {
        let row = self.selected_row(cx)?;
        self.table.read(cx).delegate().entry(row).cloned()
    }

    fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.view().is_table() {
            self.table.update(cx, |t, cx| t.set_selected_row(row, cx));
        } else {
            self.grid_selected = Some(row);
            self.scroll_grid_to(row);
            cx.notify();
        }
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
        let tab = Tab::new(self.tab().path.clone(), self.view());
        cx.defer(move |cx| open_window(vec![tab], None, cx));
    }

    /// Opens `path` in a new tab and activates it.
    fn open_tab(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let view = Settings::get(cx).view_mode;
        self.insert_tab(Tab::new(path, view), window, cx);
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
        self.sync_view_slider(window, cx);
        self.load(None, window, cx);
        self.focus_view(window, cx);
    }

    fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |t, cx| {
            t.delegate_mut().toggle_hidden();
            t.refresh(cx);
        });
        self.grid_selected = None;
        cx.notify();
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
                        }),
                )
        });
    }

    // ---- rendering --------------------------------------------------------

    /// Custom window title bar (the native one is hidden, see `open_window`).
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        // On Windows the bar is an HTCAPTION area: an unhandled mouse-down enters the
        // native move loop, which swallows the mouse-up and the click.
        let button = |button: Button| div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(button);
        TitleBar::new().pl_1().child(
            h_flex()
                .gap_0p5()
                .child(button(
                    Button::new("settings")
                        .ghost()
                        .small()
                        .icon(Icon::new(LucideIcon::Menu))
                        .tooltip(s.settings)
                        .on_click(cx.listener(|this, _, window, cx| this.open_settings(&OpenSettings, window, cx))),
                ))
                .child(button(
                    Button::new("toggle-sidebar")
                        .ghost()
                        .small()
                        .icon(Icon::new(LucideIcon::PanelLeft))
                        .tooltip(s.toggle_sidebar)
                        .selected(!self.sidebar_collapsed)
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(&ToggleSidebar, window, cx))),
                ))
                .child(img(self.logo.clone()).size_4().ml_1p5())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("FileManager")),
        )
    }

    /// Bubble phase of a right mouse-down anywhere over the table. The row's own
    /// handler (child, runs first) has already set `right_clicked_row`; the capture
    /// handler cleared it beforehand, so `None` means empty space → folder background menu.
    fn on_table_right_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let t = self.table.read(cx);
        let entry = t.right_clicked_row().and_then(|r| t.delegate().entry(r)).cloned();
        self.show_menu(menu::MenuTarget::from_entry(entry), ev, window, cx);
    }

    fn render_table(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .capture_any_mouse_down(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                if ev.button == MouseButton::Right {
                    this.table.update(cx, |t, cx| t.set_right_clicked_row(None, cx));
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| this.on_table_right_mouse_down(ev, window, cx)),
            )
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
        // First: it measures the sidebar width the grid lays its columns out in.
        let sidebar = self.render_sidebar(window, cx).into_any_element();
        let mode = self.view();
        let content = if mode.is_table() {
            self.render_table(cx).into_any_element()
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
            // Mouse back/forward buttons.
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|this, _, window, cx| this.go_back(&GoBack, window, cx)))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Forward), cx.listener(|this, _, window, cx| this.go_forward(&GoForward, window, cx)))
            .on_drag_move(cx.listener(Self::tab_drag_moved))
            .on_drag_move(cx.listener(Self::sidebar_resize_moved))
            // End of a drag: a tab released outside the window moves or detaches; any
            // other release ends tab and sidebar drags.
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, window, cx| {
                this.release_tab_outside(window, cx);
                this.end_sidebar_resize(cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.end_tab_drag(cx);
                this.end_sidebar_resize(cx);
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
                                        .child(content),
                                )
                                .when(self.preview_pane, |d| d.child(self.render_preview_pane(cx))),
                        )
                        .child(self.render_status(cx)),
                ),
            )
            .when_some(self.context_menu.as_ref(), |d, m| d.child(m.render()))
    }
}
