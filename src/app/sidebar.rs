//! Sidebar: favorites (saved, removable, reorderable by dragging; folders dragged onto
//! their heading join them) and drives, as accordion sections, and the Recycle Bin
//! below them. It can be resized from its right edge and collapsed with an animation
//! (Ctrl+B or the title bar button).

use gpui_kit::component::accordion::{Accordion, AccordionItem};
use gpui_kit::component::{ActiveTheme as _, ElementExt as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;

use super::menu::MenuTarget;
use super::*;
use crate::dnd::{self, Target};
use crate::fs::Entry;
use crate::icons::{self, IconSize};

const MIN_WIDTH: f32 = 160.;
const MAX_WIDTH: f32 = 480.;
const DEFAULT_WIDTH: f32 = 230.;

/// First click of a possible sidebar double-click, kept so the second click can undo it.
pub(super) struct SidebarClick {
    path: PathBuf,
    tab: usize,
    before: Tab,
}

/// Drag payload of the resize handle.
#[derive(Clone)]
pub(super) struct SidebarResize;

/// Drag view for drags that draw their own feedback.
pub(super) struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// A favorite being dragged to a new position.
#[derive(Clone)]
struct DraggedFavorite {
    ix: usize,
    label: SharedString,
}

impl Render for DraggedFavorite {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded(cx.theme().radius)
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_md()
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(self.label.clone())
    }
}

impl FileManager {
    /// Favorites in order; the known folders until the user changes the list.
    pub(super) fn favorites(&self, cx: &App) -> Vec<PathBuf> {
        Settings::get(cx).favorites.clone().unwrap_or_else(|| self.places.iter().map(|(_, p)| p.clone()).collect())
    }

    fn favorite_label(&self, path: &Path, s: &'static i18n::Strings, cx: &App) -> String {
        match self.places.iter().find(|(_, p)| p == path) {
            Some((known, _)) => s.known_folder(*known).to_string(),
            None => folder_name(path, cx),
        }
    }

    pub(super) fn add_favorite(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let mut favorites = self.favorites(cx);
        if !favorites.contains(&path) {
            favorites.push(path);
            Settings::update(cx, |s| s.favorites = Some(favorites));
        }
    }

    pub(super) fn remove_favorite(&mut self, path: &Path, cx: &mut Context<Self>) {
        let mut favorites = self.favorites(cx);
        favorites.retain(|p| p != path);
        Settings::update(cx, |s| s.favorites = Some(favorites));
    }

    fn move_favorite(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let mut favorites = self.favorites(cx);
        if from != to && from < favorites.len() && to < favorites.len() {
            let path = favorites.remove(from);
            favorites.insert(to, path);
            Settings::update(cx, |s| s.favorites = Some(favorites));
        }
    }

    /// A click navigates the active tab, a double click opens the folder in a new tab
    /// instead. The first click of a double click has already navigated, so the second
    /// one restores the tab as it was before.
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

    pub(super) fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        let collapsed = self.sidebar_collapsed;
        Settings::update(cx, |s| s.sidebar_collapsed = collapsed);
        cx.notify();
    }

    /// Width on screen this frame, animation included.
    pub(super) fn sidebar_visible_width(&self) -> f32 {
        self.sidebar_visible
    }

    pub(super) fn sidebar_resize_moved(&mut self, ev: &DragMoveEvent<SidebarResize>, _: &mut Window, cx: &mut Context<Self>) {
        self.resizing_sidebar = true;
        self.sidebar_width = f32::from(ev.event.position.x).clamp(MIN_WIDTH, MAX_WIDTH);
        cx.notify();
    }

    /// Mouse released: keep the new width for the next windows and runs.
    pub(super) fn end_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.resizing_sidebar) {
            let width = self.sidebar_width;
            Settings::update(cx, |s| s.sidebar_width = width);
        }
    }

    pub(super) fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 0 = collapsed, 1 = open; the width follows, the content keeps its width and
        // slides under the clip instead of reflowing.
        let open = gpui_kit::base::spring(
            "sidebar-open",
            if self.sidebar_collapsed { 0f32 } else { 1f32 },
            cx.theme().motion_tokens().spring_control,
            window,
            cx,
        );
        self.sidebar_visible = (self.sidebar_width * open).max(0.);
        let s = i18n::t(cx);
        let settings = Settings::get(cx);
        let (favorites_open, drives_open) = (settings.favorites_open, settings.drives_open);
        let favorites: Vec<Div> = self
            .favorites(cx)
            .into_iter()
            .enumerate()
            .map(|(ix, path)| {
                let label = self.favorite_label(&path, s, cx);
                self.render_favorite(ix, path, label, cx)
            })
            .collect();
        let drives: Vec<Div> = (0..self.drives.len()).map(|ix| self.render_drive(ix, cx)).collect();
        let heading = |label: &'static str, cx: &App| {
            div().text_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().muted_foreground).child(label)
        };
        let favorites_title = self.render_favorites_title(heading(s.favorites, cx), cx);

        // Sections as tall as their content (the accordion fills its parent by default),
        // on the sidebar's own background, aligned with the items.
        let section = |item: AccordionItem| {
            item.bg(transparent_black())
                .title_style(StyleRefinement::default().px_2())
                .content_style(StyleRefinement::default().px_0().pb_1())
        };
        let sections = Accordion::new("sidebar-sections")
            .multiple(true)
            .bordered(false)
            .small()
            .h_auto()
            .item(|item| section(item).title(favorites_title).open(favorites_open).child(v_flex().children(favorites)))
            .item(|item| section(item).title(heading(s.drives, cx)).open(drives_open).child(v_flex().children(drives)))
            .on_toggle_click(|open: &[usize], _, cx| {
                Settings::update(cx, |s| {
                    s.favorites_open = open.contains(&0);
                    s.drives_open = open.contains(&1);
                })
            });

        div()
            .id("sidebar")
            .relative()
            .h_full()
            .flex_shrink_0()
            .w(px(self.sidebar_visible))
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .when(self.sidebar_visible >= 1., |d| d.border_r_1().border_color(cx.theme().border))
            .when(self.sidebar_visible >= 1., |d| {
                d.child(
                    v_flex()
                        .w(px(self.sidebar_width))
                        .h_full()
                        .child(v_flex().id("sidebar-content").flex_1().min_h_0().overflow_y_scroll().p_1().child(sections))
                        .child(
                            div()
                                .flex_shrink_0()
                                .p_1()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .child(self.render_bin_item(cx)),
                        ),
                )
            })
            .when(!self.sidebar_collapsed, |d| {
                d.child(
                    div()
                        .id("sidebar-resize")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .w(px(5.))
                        .cursor_col_resize()
                        .hover(|d| d.bg(cx.theme().drag_border.opacity(0.6)))
                        .when(self.resizing_sidebar, |d| d.bg(cx.theme().drag_border))
                        .on_drag(SidebarResize, |_, _, _, cx| cx.new(|_| DragGhost))
                        .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                            if ev.click_count() == 2 {
                                this.sidebar_width = DEFAULT_WIDTH;
                                Settings::update(cx, |s| s.sidebar_width = DEFAULT_WIDTH);
                            }
                        })),
                )
            })
    }

    /// A folder of the sidebar; files can be dropped on it.
    fn sidebar_item(&self, id: ElementId, path: &Path, label: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let active = self.tab().path == path;
        let files_over = matches!(&self.drop_hover, Some(SpotKey::Sidebar(p)) if p == path);
        let (click, middle) = (path.to_path_buf(), path.to_path_buf());
        h_flex()
            .id(id)
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .when(active, |d| d.bg(cx.theme().accent))
            .hover(|d| d.bg(cx.theme().accent.opacity(0.6)))
            .when(files_over, |d| d.bg(cx.theme().drop_target))
            .cursor_pointer()
            .on_prepaint(dnd::zone(&self.drop_zones, SpotKey::Sidebar(path.to_path_buf()), path.to_path_buf(), 2))
            .child(icons::entry_icon(&Entry::folder(path), IconSize::Small, px(16.), cx))
            .child(div().truncate().child(label))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.sidebar_click(click.clone(), ev.click_count(), window, cx)
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.open_tab(middle.clone(), window, cx)),
            )
    }

    /// The favorites heading. While folders are dragged over the window it shows where
    /// to drop them to make them favorites.
    fn render_favorites_title(&self, heading: Div, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        let theme = cx.theme();
        let over = self.drop_hover == Some(SpotKey::Favorites);
        h_flex()
            .w_full()
            .min_h(px(22.))
            .gap_2()
            .justify_between()
            .on_prepaint(dnd::zone(&self.drop_zones, SpotKey::Favorites, Target::Favorites, 3))
            .child(heading)
            .when(self.dragging_folders, |d| {
                d.child(
                    h_flex()
                        .flex_shrink_0()
                        .gap_1()
                        .px_2()
                        .py_0p5()
                        .rounded(theme.radius)
                        .border_1()
                        .border_dashed()
                        .border_color(theme.drag_border)
                        .text_xs()
                        .text_color(if over { theme.foreground } else { theme.muted_foreground })
                        .when(over, |d| d.bg(theme.drop_target))
                        .child(Icon::new(LucideIcon::Plus).xsmall())
                        .child(s.favorites_drop),
                )
            })
    }

    /// The Recycle Bin, below the sections: open it, drop files on it to delete them.
    fn render_bin_item(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let path = PathBuf::from(fs::RECYCLE_BIN);
        let label = i18n::t(cx).recycle_bin.to_string();
        self.sidebar_item("recycle-bin".into(), &path, label, cx).on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                this.show_menu(MenuTarget::sidebar(&path, false), ev.position, false, window, cx)
            }),
        )
    }

    fn render_favorite(&self, ix: usize, path: PathBuf, label: String, cx: &mut Context<Self>) -> Div {
        let drag = DraggedFavorite { ix, label: label.clone().into() };
        let item = self
            .sidebar_item(ElementId::NamedInteger("favorite".into(), ix as u64), &path, label, cx)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.show_menu(MenuTarget::sidebar(&path, true), ev.position, ev.modifiers.shift, window, cx)
                }),
            )
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .drag_over::<DraggedFavorite>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .on_drop(cx.listener(move |this, drag: &DraggedFavorite, _, cx| this.move_favorite(drag.ix, ix, cx)));
        div().child(item)
    }

    fn render_drive(&self, ix: usize, cx: &mut Context<Self>) -> Div {
        let d = &self.drives[ix];
        let root = d.root.clone();
        let used = if d.total > 0 { 1.0 - d.free as f32 / d.total as f32 } else { 0.0 };
        let bar = if used > 0.9 { cx.theme().danger } else { cx.theme().primary };
        let has_size = d.total > 0;
        let item = self
            .sidebar_item(ElementId::NamedInteger("drive".into(), ix as u64), &root, drive_label(d), cx)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.show_menu(MenuTarget::sidebar(&root, false), ev.position, ev.modifiers.shift, window, cx)
                }),
            );
        v_flex().child(item).when(has_size, |c| {
            c.child(
                div()
                    .mx_2()
                    .ml(px(32.))
                    .mb_1()
                    .h(px(3.))
                    .rounded_full()
                    .bg(cx.theme().muted)
                    .child(div().h_full().rounded_full().w(relative(used)).bg(bar)),
            )
        })
    }
}
