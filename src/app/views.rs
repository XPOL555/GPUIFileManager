//! View modes (details, tree, list, icons), the icon grid, the preview pane and the
//! status bar with its info bar (counts, preview toggle, view mode picker).

use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use serde::{Deserialize, Serialize};

use super::menu::MenuTarget;
use super::*;
use crate::icons::{self, IconSize};
use crate::table::type_label;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[default]
    Details,
    Tree,
    List,
    MediumIcons,
    LargeIcons,
    ExtraLargeIcons,
}

/// Size of one item in the grid views, in logical pixels.
#[derive(Clone, Copy)]
struct Tile {
    w: f32,
    h: f32,
    /// Icon or thumbnail box.
    image: f32,
    icon: IconSize,
    /// Thumbnail request size in device pixels.
    thumb: u32,
    /// Icon beside the name (list) instead of above it.
    horizontal: bool,
}

const GRID_GAP: f32 = 4.;
const GRID_PAD: f32 = 8.;
const PREVIEW_WIDTH: f32 = 300.;

impl ViewMode {
    /// Smallest to largest, the order of Ctrl + mouse wheel.
    pub const ALL: [ViewMode; 6] = [
        ViewMode::Details,
        ViewMode::Tree,
        ViewMode::List,
        ViewMode::MediumIcons,
        ViewMode::LargeIcons,
        ViewMode::ExtraLargeIcons,
    ];

    pub fn is_table(self) -> bool {
        matches!(self, ViewMode::Details | ViewMode::Tree)
    }

    fn step(self, delta: i32) -> Self {
        Self::ALL[(self as i32 + delta).clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    /// Position on the vertical picker slider: 0 (details) at the bottom.
    pub(super) fn slider_value(self) -> f32 {
        self as usize as f32
    }

    pub(super) fn from_slider(value: f32) -> Self {
        Self::ALL[(value.round() as usize).min(Self::ALL.len() - 1)]
    }

    fn icon(self) -> LucideIcon {
        match self {
            ViewMode::Details => LucideIcon::Sheet,
            ViewMode::Tree => LucideIcon::ListTree,
            ViewMode::List => LucideIcon::List,
            _ => LucideIcon::Image,
        }
    }

    fn tile(self) -> Tile {
        let tile = |w, h, image, icon, thumb| Tile { w, h, image, icon, thumb, horizontal: false };
        match self {
            ViewMode::MediumIcons => tile(100., 112., 48., IconSize::ExtraLarge, 96),
            ViewMode::LargeIcons => tile(148., 156., 96., IconSize::Jumbo, 160),
            ViewMode::ExtraLargeIcons => tile(232., 244., 192., IconSize::Jumbo, 256),
            _ => Tile { w: 260., h: 28., image: 16., icon: IconSize::Small, thumb: 0, horizontal: true },
        }
    }
}

impl FileManager {
    pub(super) fn set_view_mode(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.view() {
            return;
        }
        // Keep the selected item selected in the new view.
        let keep = self.selected_entry(cx).map(|e| e.name.clone());
        self.tabs[self.active].view = mode;
        self.table.update(cx, |t, cx| {
            t.delegate_mut().set_tree(mode == ViewMode::Tree);
            t.refresh(cx);
        });
        let row = keep.and_then(|name| self.table.read(cx).delegate().row_of(&name));
        self.grid_selected = row;
        self.table.update(cx, |t, cx| match row {
            Some(row) if mode.is_table() => {
                t.set_selected_row(row, cx);
                t.scroll_to_row(row, cx);
            }
            _ => t.clear_selection(cx),
        });
        if let Some(row) = row {
            self.scroll_grid_to(row);
        }
        self.sync_view_slider(window, cx);
        Settings::update(cx, |s| s.view_mode = mode);
        self.focus_view(window, cx);
        cx.notify();
    }

    pub(super) fn sync_view_slider(&self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.view().slider_value();
        self.view_slider.update(cx, |s, cx| s.set_value(value, window, cx));
    }

    pub(super) fn toggle_preview(&mut self, _: &TogglePreview, _: &mut Window, cx: &mut Context<Self>) {
        self.preview_pane = !self.preview_pane;
        let open = self.preview_pane;
        Settings::update(cx, |s| s.preview_pane = open);
        cx.notify();
    }

    /// Ctrl + mouse wheel over the files switches the view mode, like File Pilot. The
    /// table scrolls in the capture phase and stops the event there, so this listener
    /// must be painted (registered) before the view it covers; it stops the event too.
    pub(super) fn zoom_listener(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let this = this.clone();
                window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && e.modifiers.control && bounds.contains(&e.position) {
                        cx.stop_propagation();
                        this.update(cx, |fm, cx| fm.zoom(e.delta, window, cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }

    fn zoom(&mut self, delta: ScrollDelta, window: &mut Window, cx: &mut Context<Self>) {
        // A wheel notch is one step; a touchpad sends pixels, one step per 60.
        let step = match delta {
            ScrollDelta::Lines(lines) => lines.y.signum() as i32,
            ScrollDelta::Pixels(pixels) => {
                self.zoom_pixels += f32::from(pixels.y);
                let step = (self.zoom_pixels / 60.).trunc() as i32;
                self.zoom_pixels -= step as f32 * 60.;
                step
            }
        };
        if step != 0 {
            self.set_view_mode(self.view().step(step), window, cx);
        }
    }

    // ---- grid ---------------------------------------------------------------

    pub(super) fn scroll_grid_to(&self, row: usize) {
        self.grid_scroll.scroll_to_item(row / self.grid_columns.max(1), ScrollStrategy::Nearest);
    }

    fn grid_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.table.read(cx).delegate().len();
        if count == 0 {
            return;
        }
        let row = match self.grid_selected {
            None => 0,
            Some(row) => (row as isize + delta).clamp(0, count as isize - 1) as usize,
        };
        self.grid_selected = Some(row);
        self.scroll_grid_to(row);
        cx.notify();
    }

    pub(super) fn render_grid(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tile = mode.tile();
        let preview = if self.preview_pane { PREVIEW_WIDTH } else { 0. };
        let available =
            f32::from(window.viewport_size().width) - self.sidebar_visible_width() - preview - 2. * GRID_PAD - 14.;
        let columns = (((available + GRID_GAP) / (tile.w + GRID_GAP)) as usize).max(1);
        self.grid_columns = columns;
        let count = self.table.read(cx).delegate().len();
        let lines = count.div_ceil(columns);
        let cols = columns as isize;

        v_flex()
            .id("grid")
            .size_full()
            .track_focus(&self.grid_focus)
            .key_context(GRID_CONTEXT)
            .on_action(cx.listener(|this, _: &GridLeft, _, cx| this.grid_move(-1, cx)))
            .on_action(cx.listener(|this, _: &GridRight, _, cx| this.grid_move(1, cx)))
            .on_action(cx.listener(move |this, _: &GridUp, _, cx| this.grid_move(-cols, cx)))
            .on_action(cx.listener(move |this, _: &GridDown, _, cx| this.grid_move(cols, cx)))
            .on_action(cx.listener(|this, _: &GridHome, _, cx| this.grid_move(isize::MIN / 2, cx)))
            .on_action(cx.listener(|this, _: &GridEnd, _, cx| this.grid_move(isize::MAX / 2, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.grid_focus.focus(window, cx)))
            // Tiles stop their clicks, so these only see the empty space around them.
            .on_click(cx.listener(|this, _, _, cx| {
                this.grid_selected = None;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| this.show_menu(MenuTarget::background(), ev, window, cx)),
            )
            .child(
                uniform_list(
                    "grid-lines",
                    lines,
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        range.map(|line| this.render_grid_line(line, columns, count, tile, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.grid_scroll)
                .size_full()
                .px(px(GRID_PAD))
                .py(px(GRID_PAD / 2.)),
            )
            .into_any_element()
    }

    fn render_grid_line(&self, line: usize, columns: usize, count: usize, tile: Tile, cx: &mut Context<Self>) -> AnyElement {
        let start = line * columns;
        let tiles: Vec<AnyElement> =
            (start..(start + columns).min(count)).filter_map(|ix| self.render_tile(ix, tile, cx)).collect();
        h_flex().h(px(tile.h + GRID_GAP)).gap(px(GRID_GAP)).items_start().children(tiles).into_any_element()
    }

    fn render_tile(&self, ix: usize, tile: Tile, cx: &mut Context<Self>) -> Option<AnyElement> {
        let e = self.table.read(cx).delegate().entry(ix)?.clone();
        let selected = self.grid_selected == Some(ix);
        let theme = cx.theme();
        let (active, active_border, hover, radius) =
            (theme.table_active, theme.table_active_border, theme.accent.opacity(0.5), theme.radius);
        let image = if tile.horizontal {
            icons::entry_icon(&e, tile.icon, px(tile.image), cx)
        } else {
            icons::entry_thumbnail(&e, tile.thumb, tile.icon, px(tile.image), cx)
        };
        let name = div().min_w_0().overflow_hidden().child(e.name.clone());
        let body = if tile.horizontal {
            h_flex().w_full().min_w_0().gap_2().px_2().child(image).child(name.flex_1().truncate())
        } else {
            // The name wraps inside the tile (long names without spaces break anywhere)
            // and is cut after two lines.
            v_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_1()
                .pt_1()
                .px_1()
                .child(div().size(px(tile.image)).flex().items_center().justify_center().child(image))
                .child(name.w_full().text_xs().text_center().text_ellipsis().line_clamp(2))
        };
        let (open, menu, middle) = (e.clone(), e.clone(), e.clone());
        Some(
            div()
                .id(("tile", ix))
                .w(px(tile.w))
                .h(px(tile.h))
                .flex()
                .when(!tile.horizontal, |d| d.justify_center())
                .when(tile.horizontal, |d| d.items_center())
                .rounded(radius)
                .border_1()
                .border_color(if selected { active_border } else { transparent_black() })
                .when(selected, |d| d.bg(active))
                .when(!selected, |d| d.hover(move |d| d.bg(hover)))
                .when(e.hidden, |d| d.opacity(0.55))
                .child(body)
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.grid_selected = Some(ix);
                    this.grid_focus.focus(window, cx);
                    if ev.click_count() >= 2 {
                        this.open_entry(open.clone(), window, cx);
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.grid_selected = Some(ix);
                        this.show_menu(MenuTarget::from_entry(Some(menu.clone())), ev, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| {
                        if middle.is_dir {
                            this.open_tab(middle.path.clone(), window, cx);
                        }
                    }),
                )
                .into_any_element(),
        )
    }

    // ---- preview pane -------------------------------------------------------

    pub(super) fn render_preview_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        let entry = self.selected_entry(cx);
        let pane = v_flex()
            .w(px(PREVIEW_WIDTH))
            .h_full()
            .flex_shrink_0()
            .p_3()
            .gap_3()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar);
        let Some(e) = entry else {
            return pane
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(div().text_center().child(s.no_selection));
        };
        let muted = cx.theme().muted_foreground;
        let detail = |label: &'static str, value: String| {
            h_flex()
                .justify_between()
                .gap_2()
                .text_xs()
                .child(div().text_color(muted).child(label))
                .child(div().truncate().child(value))
        };
        pane.child(
            div()
                .w_full()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .child(icons::entry_thumbnail(&e, 512, IconSize::Jumbo, px(256.), cx)),
        )
        .child(div().text_base().font_weight(FontWeight::MEDIUM).child(e.name.clone()))
        .child(
            v_flex()
                .gap_1()
                .child(detail(s.col_type, type_label(&e, cx)))
                .when(!e.is_dir, |d| d.child(detail(s.col_size, fs::format_size(e.size))))
                .child(detail(s.col_modified, fs::format_time(e.modified))),
        )
    }

    // ---- status bar -----------------------------------------------------------

    pub(super) fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        let (dirs, files) = self.table.read(cx).delegate().summary();
        let left = match (&self.error, self.selected_entry(cx)) {
            (Some(err), _) => err.to_string(),
            (None, Some(e)) if !e.is_dir => format!("{} · {}", e.name, fs::format_size(e.size)),
            (None, Some(e)) => e.name.to_string(),
            (None, None) => String::new(),
        };
        let muted = cx.theme().muted_foreground;
        let stat = |icon: IconName, n: usize, label: &'static str, id: &'static str| {
            h_flex()
                .id(id)
                .gap_1()
                .px_1p5()
                .child(Icon::new(icon).xsmall())
                .child(n.to_string())
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx))
        };
        let divider = || div().w(px(1.)).h(px(14.)).mx_1().bg(cx.theme().border);
        h_flex()
            .h(px(30.))
            .pl_3()
            .pr_1()
            .gap_2()
            .text_xs()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().title_bar)
            .text_color(muted)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(self.error.is_some(), |d| d.text_color(cx.theme().danger))
                    .child(left),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .child(stat(IconName::Folder, dirs, s.folders, "count-folders"))
                    .child(stat(IconName::File, files, s.files, "count-files"))
                    .child(divider())
                    .child(
                        Button::new("preview-pane")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(LucideIcon::PanelRight))
                            .selected(self.preview_pane)
                            .tooltip(s.preview_pane)
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_preview(&TogglePreview, window, cx))),
                    )
                    .child(divider())
                    .child(self.render_view_picker(cx)),
            )
    }

    /// The info bar's view mode button and its popover: the modes, largest on top,
    /// beside a vertical slider, like File Pilot.
    fn render_view_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        let mode = self.view();
        let this = cx.entity().downgrade();
        let slider = self.view_slider.clone();
        Popover::new("view-mode")
            .anchor(Anchor::BottomRight)
            .trigger(
                Button::new("view-mode-button")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(mode.icon()))
                    .label(s.view_mode(mode)),
            )
            .content(move |_, _, cx| {
                let s = i18n::t(cx);
                let current = this.upgrade().map(|fm| fm.read(cx).view()).unwrap_or_default();
                let rows = ViewMode::ALL.iter().rev().map(|&mode| {
                    let this = this.clone();
                    let selected = mode == current;
                    h_flex()
                        .id(("view-mode", mode as usize))
                        .h(px(28.))
                        .px_2()
                        .gap_2()
                        .rounded(cx.theme().radius)
                        .cursor_pointer()
                        .when(selected, |d| d.bg(cx.theme().primary).text_color(cx.theme().primary_foreground))
                        .when(!selected, |d| d.hover(|d| d.bg(cx.theme().accent)))
                        .child(Icon::new(mode.icon()).small())
                        .child(s.view_mode(mode))
                        .on_click(move |_, window, cx| {
                            this.update(cx, |fm, cx| fm.set_view_mode(mode, window, cx)).ok();
                        })
                });
                v_flex()
                    .gap_2()
                    .p_1()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_stretch()
                            .child(v_flex().w(px(150.)).gap_0p5().children(rows))
                            .child(Slider::new(&slider).vertical().h(px(6. * 28. + 5. * 2.))),
                    )
                    .child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(s.view_hint),
                    )
            })
    }
}
