//! Main window: sidebar (places + drives), tab strip, address bar, file table, status bar.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::i18n::{self, Language};
use crate::table::FileTable;
use crate::{fs, shell};

actions!(
    file_manager,
    [
        OpenSelected,
        GoBack,
        GoForward,
        GoUp,
        Refresh,
        NewTab,
        CloseTab,
        NextTab,
        FocusFilter,
        EditPath,
        ToggleHidden,
        OpenSettings
    ]
);

const CONTEXT: &str = "FileManager";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", OpenSelected, Some(CONTEXT)),
        KeyBinding::new("backspace", GoUp, Some(CONTEXT)),
        KeyBinding::new("alt-up", GoUp, Some(CONTEXT)),
        KeyBinding::new("alt-left", GoBack, Some(CONTEXT)),
        KeyBinding::new("alt-right", GoForward, Some(CONTEXT)),
        KeyBinding::new("f5", Refresh, Some(CONTEXT)),
        KeyBinding::new("ctrl-t", NewTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-w", CloseTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-f", FocusFilter, Some(CONTEXT)),
        KeyBinding::new("ctrl-l", EditPath, Some(CONTEXT)),
        KeyBinding::new("ctrl-h", ToggleHidden, Some(CONTEXT)),
        KeyBinding::new("ctrl-,", OpenSettings, Some(CONTEXT)),
    ]);
}

struct Tab {
    path: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl Tab {
    fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
            .into()
    }
}

pub struct FileManager {
    focus: FocusHandle,
    tabs: Vec<Tab>,
    active: usize,
    table: Entity<TableState<FileTable>>,
    filter: Entity<InputState>,
    path_input: Entity<InputState>,
    editing_path: bool,
    error: Option<SharedString>,
    places: Vec<(shell::KnownFolder, PathBuf)>,
    drives: Vec<shell::Drive>,
    /// Bumped on each navigation so late results from a previous listing are dropped.
    load_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl FileManager {
    pub fn new(start: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                    this.table.read(cx).focus_handle(cx).focus(window, cx);
                }
                _ => {}
            }),
            cx.subscribe_in(&path_input, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    let p = PathBuf::from(input.read(cx).value().trim());
                    this.editing_path = false;
                    this.navigate(p, true, window, cx);
                }
                InputEvent::Blur => {
                    this.editing_path = false;
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe_global_in::<i18n::CurrentLanguage>(window, |this, window, cx| {
                let placeholder = i18n::t(cx).filter;
                this.filter.update(cx, |f, cx| f.set_placeholder(placeholder, window, cx));
            }),
        ];

        let mut this = Self {
            focus: cx.focus_handle(),
            tabs: vec![Tab { path: start.clone(), back: vec![], forward: vec![] }],
            active: 0,
            table,
            filter,
            path_input,
            editing_path: false,
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

    /// Bubble phase of a right mouse-down anywhere over the table. The row's own
    /// handler (child, runs first) has already set `right_clicked_row`; the capture
    /// handler cleared it beforehand, so `None` means empty space → folder background menu.
    fn on_table_right_mouse_down(&mut self, cx: &mut Context<Self>) {
        let t = self.table.read(cx);
        let paths: Vec<PathBuf> = t
            .right_clicked_row()
            .and_then(|r| t.delegate().entry(r))
            .map(|e| vec![e.path.clone()])
            .unwrap_or_default();
        self.open_context_menu(paths, cx);
    }

    /// The native menu runs a nested Win32 modal loop. Running it inside this
    /// event handler would re-enter GPUI while `App` is borrowed, so it is
    /// deferred to a foreground task where no GPUI borrow is held, and the
    /// folder is reloaded afterwards (the command may have renamed/deleted/created files).
    fn open_context_menu(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let folder = self.tab().path.clone();
        cx.spawn(async move |this, cx| {
            if let Err(err) = shell::show_context_menu(&folder, &paths) {
                eprintln!("context menu: {err:?}");
            }
            let _ = this.update(cx, |_, cx| cx.notify());
            // Reload needs a Window; the view re-lists on the next Refresh via the action path.
            let _ = cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, cx| window.dispatch_action(Box::new(Refresh), cx));
                }
            });
        })
        .detach();
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
        let path = self.tab().path.clone();
        self.tabs.push(Tab { path, back: vec![], forward: vec![] });
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

    fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn edit_path(&mut self, _: &EditPath, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.tab().path.to_string_lossy().into_owned();
        self.editing_path = true;
        self.path_input.update(cx, |i, cx| i.set_value(p, window, cx));
        self.path_input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
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

    /// Custom window title bar (the native one is hidden, see `main`).
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
                            .icon(Icon::new(gpui_kit::assets::IconName::Menu))
                            .on_click(cx.listener(|this, _, window, cx| this.open_settings(&OpenSettings, window, cx))),
                    ),
                )
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("FileManager")),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let current = self.tab().path.clone();
        let item = |id: SharedString, icon: IconName, label: String, path: PathBuf, cx: &mut Context<Self>| {
            let active = current == path;
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
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(path.clone(), true, window, cx)))
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
            let label = if d.label.is_empty() { root.clone() } else { format!("{} ({})", d.label, &root[..2]) };
            let used = if d.total > 0 { 1.0 - d.free as f32 / d.total as f32 } else { 0.0 };
            let bar = if used > 0.9 { cx.theme().danger } else { cx.theme().primary };
            col = col.child(
                v_flex()
                    .child(item(format!("drive-{root}").into(), IconName::HardDrive, label, d.root.clone(), cx))
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
        let mut strip = h_flex().h(px(36.)).px_1().gap_1().bg(theme.title_bar).border_b_1().border_color(theme.border);
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
                    ),
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
            div().flex_1().child(Input::new(&self.path_input).small()).into_any_element()
        } else {
            // Breadcrumb: one clickable segment per ancestor.
            let mut crumbs = h_flex().id("breadcrumb").flex_1().gap_0p5().overflow_hidden();
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
                    Button::new(("crumb", i))
                        .ghost()
                        .small()
                        .label(name)
                        .on_click(cx.listener(move |this, _, window, cx| this.navigate(p.clone(), true, window, cx))),
                );
            }
            crumbs
                .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                    if ev.click_count() >= 2 {
                        this.edit_path(&EditPath, window, cx);
                    }
                }))
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
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::focus_filter))
            .on_action(cx.listener(Self::edit_path))
            .on_action(cx.listener(Self::toggle_hidden))
            .on_action(cx.listener(Self::open_settings))
            // Mouse back/forward buttons.
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|this, _, window, cx| this.go_back(&GoBack, window, cx)))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Forward), cx.listener(|this, _, window, cx| this.go_forward(&GoForward, window, cx)))
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
                                    cx.listener(|this, _, _, cx| this.on_table_right_mouse_down(cx)),
                                )
                                .child(DataTable::new(&self.table).bordered(false).small()),
                        )
                        .child(self.render_status(cx)),
                ),
            )
    }
}
