//! Tab strip and tab dragging. On the strip the dragged tab moves between the others
//! as the cursor goes; dropped on another window, of this instance or of another one,
//! it moves there, where the cursor is; anywhere else it opens in a new window. While
//! dragging, a floating preview (`drag_preview`) follows the cursor and says what will
//! happen, and the window the tab would move to shows where it would land.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, ElementExt as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;

use super::sidebar::DragGhost;
use super::*;
use crate::dnd;
use crate::drag_preview::DropAction;
use crate::fs::Entry;
use crate::hooks::TabSignal;
use crate::icons::{self, IconSize};

/// Payload of a tab drag.
#[derive(Clone)]
pub(super) struct DraggedTab {
    source: EntityId,
    id: u64,
    title: SharedString,
    path: PathBuf,
}

/// The tab being dragged, from drag start until the button is released. GPUI drops
/// its own drag state on a release that no drop target takes, which is exactly the
/// case (released outside the window) that has to detach the tab.
#[derive(Default)]
struct TabDragState(Option<DraggedTab>);

impl Global for TabDragState {}

/// A tab from another window hovering this one.
pub(super) struct IncomingTab {
    title: SharedString,
    /// Where it would land among the tabs.
    index: usize,
}

/// The window a dragged tab would move to.
pub(super) enum DragTarget {
    /// One of this instance.
    Local(WeakEntity<FileManager>, AnyWindowHandle),
    /// One of another instance, by its window handle.
    Foreign(isize),
}

impl DragTarget {
    fn same(&self, other: &DragTarget) -> bool {
        match (self, other) {
            (DragTarget::Local(a, _), DragTarget::Local(b, _)) => a.entity_id() == b.entity_id(),
            (DragTarget::Foreign(a), DragTarget::Foreign(b)) => a == b,
            _ => false,
        }
    }
}

impl FileManager {
    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let source = cx.entity_id();
        let strip_bounds = self.tab_strip.clone();
        let dragged = cx
            .try_global::<TabDragState>()
            .and_then(|state| state.0.as_ref())
            .filter(|drag| drag.source == source)
            .map(|drag| drag.id);
        let theme = cx.theme();
        let (title_bar, border, drop_target, drag_border) =
            (theme.title_bar, theme.border, theme.drop_target, theme.drag_border);
        let mut strip = h_flex()
            .id("tab-strip")
            .h(px(36.))
            .px_1()
            .gap_1()
            .bg(title_bar)
            .border_b_1()
            .border_color(border)
            .on_prepaint(move |bounds, _, _| strip_bounds.set(bounds))
            .on_drop(cx.listener(|this, _: &DraggedTab, _, cx| this.end_tab_drag(cx)))
            .when(self.incoming_tab.is_some(), |strip| strip.bg(drop_target));
        let incoming_at = self.incoming_tab.as_ref().map(|incoming| incoming.index);
        for (ix, tab) in self.tabs.iter().enumerate() {
            if incoming_at == Some(ix) {
                strip = strip.children(self.render_incoming_tab(cx));
            }
            let active = ix == self.active;
            let lifted = dragged == Some(tab.id);
            let files_over = self.drop_hover == Some(SpotKey::Tab(ix));
            let theme = cx.theme();
            let (background, accent, primary, radius) = (theme.background, theme.accent, theme.primary, theme.radius);
            let drag = DraggedTab { source, id: tab.id, title: tab.title(cx), path: tab.path.clone() };
            let bounds = self.tab_bounds.clone();
            strip = strip.child(
                h_flex()
                    .id(("tab", tab.id as usize))
                    .gap_2()
                    .pl_3()
                    .pr_1()
                    .h(px(28.))
                    .max_w(px(220.))
                    .rounded(radius)
                    .border_1()
                    .border_color(transparent_black())
                    .when(active, |d| d.bg(background))
                    .hover(move |d| d.bg(accent))
                    .when(lifted, |d| d.bg(background).border_color(primary).shadow_md())
                    .when(files_over, |d| d.bg(drop_target).border_color(drag_border))
                    .cursor_pointer()
                    .child(icons::entry_icon(&Entry::folder(&tab.path), IconSize::Small, px(16.), cx))
                    .child(div().truncate().child(tab.title(cx)))
                    .when(self.tabs.len() > 1, |d| {
                        d.child(
                            Button::new(("close-tab", tab.id as usize))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab_at(ix, window, cx);
                                })),
                        )
                    })
                    .on_prepaint(move |b, _, _| {
                        let mut bounds = bounds.borrow_mut();
                        if bounds.len() <= ix {
                            bounds.resize(ix + 1, Bounds::default());
                        }
                        bounds[ix] = b;
                    })
                    .on_prepaint(dnd::zone(&self.drop_zones, SpotKey::Tab(ix), tab.path.clone(), 2))
                    .on_click(cx.listener(move |this, _, window, cx| this.switch_tab(ix, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab_at(ix, window, cx)),
                    )
                    // The preview is a window of its own (`drag_preview`); GPUI's would be clipped.
                    .on_drag(drag, |drag, _, _, cx| {
                        cx.default_global::<TabDragState>().0 = Some(drag.clone());
                        cx.new(|_| DragGhost)
                    })
                    .on_drop(cx.listener(|this, _: &DraggedTab, _, cx| this.end_tab_drag(cx))),
            );
        }
        if incoming_at.is_some_and(|ix| ix >= self.tabs.len()) {
            strip = strip.children(self.render_incoming_tab(cx));
        }
        strip.child(
            Button::new("new-tab")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .on_click(cx.listener(|this, _, window, cx| this.new_tab(&NewTab, window, cx))),
        )
    }

    /// Where a tab from another window would land.
    fn render_incoming_tab(&self, cx: &App) -> Option<impl IntoElement> {
        let incoming = self.incoming_tab.as_ref()?;
        let theme = cx.theme();
        Some(
            h_flex()
                .gap_2()
                .px_3()
                .h(px(28.))
                .max_w(px(220.))
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.drag_border)
                .bg(theme.background.opacity(0.7))
                .child(Icon::new(IconName::Folder).small().text_color(theme.drag_border))
                .child(div().truncate().child(incoming.title.clone())),
        )
    }

    /// Every mouse move of a tab drag (the source window keeps the mouse captured, so
    /// this also runs outside it): move the tab along the strip or the preview along
    /// the screen, and update what a release would do.
    pub(super) fn tab_drag_moved(&mut self, ev: &DragMoveEvent<DraggedTab>, window: &mut Window, cx: &mut Context<Self>) {
        let drag = ev.drag(cx).clone();
        if drag.source != cx.entity_id() {
            return;
        }
        let position = ev.event.position;
        let cursor = shell::cursor_pos();
        let inside = Bounds::new(Point::default(), window.viewport_size()).contains(&position);
        let (action, target) = if inside {
            if self.tab_strip.get().contains(&position) {
                self.reorder_dragged(drag.id, position.x, cx);
                (DropAction::Reorder, None)
            } else {
                (DropAction::Cancel, None)
            }
        } else {
            match self.target_under(cursor, cx) {
                Some(target) => (DropAction::MoveTo, Some(target)),
                None if self.tabs.len() > 1 => (DropAction::NewWindow, None),
                None => (DropAction::MoveWindow, None),
            }
        };
        if self.drag_preview.is_none() {
            let origin = window.bounds().origin + position;
            self.drag_preview = DragPreviewWindow::open(Entry::folder(&drag.path), drag.title.clone(), origin, cx);
        }
        let scale = window.scale_factor();
        if let Some(preview) = &mut self.drag_preview {
            preview.update(cursor, scale, action, cx);
        }
        self.set_drag_target(target, &drag.title, cursor, cx);
    }

    /// Moves the dragged tab to where the cursor is on the strip: after every other tab
    /// whose middle is left of it.
    fn reorder_dragged(&mut self, id: u64, x: Pixels, cx: &mut Context<Self>) {
        let Some(from) = self.tabs.iter().position(|t| t.id == id) else { return };
        let to = self
            .tab_bounds
            .borrow()
            .iter()
            .take(self.tabs.len())
            .enumerate()
            .filter(|&(ix, bounds)| ix != from && bounds.center().x < x)
            .count();
        let to = to.min(self.tabs.len() - 1);
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

    /// Where a tab dropped at `cursor` (screen pixels) lands among this window's tabs:
    /// at the end, unless the cursor is on the strip.
    fn insert_index(&self, cursor: shell::ScreenPoint, window: &Window) -> usize {
        let (x, y) = shell::screen_to_client(self.hwnd, cursor);
        let scale = window.scale_factor();
        let at = point(px(x as f32 / scale), px(y as f32 / scale));
        if !self.tab_strip.get().contains(&at) {
            return self.tabs.len();
        }
        self.tab_bounds.borrow().iter().take(self.tabs.len()).filter(|b| b.center().x < at.x).count()
    }

    /// Another window of this instance, or of another one, under the cursor.
    fn target_under(&self, cursor: shell::ScreenPoint, cx: &mut App) -> Option<DragTarget> {
        let under = shell::root_window_at(cursor);
        if under == 0 || under == self.hwnd {
            return None;
        }
        let local = cx
            .default_global::<OpenWindows>()
            .0
            .iter()
            .find(|w| w.hwnd == under && w.view.upgrade().is_some())
            .map(|w| DragTarget::Local(w.view.clone(), w.handle));
        local.or_else(|| hooks::is_foreign_app_window(under).then_some(DragTarget::Foreign(under)))
    }

    /// Tells the window the tab would move to where the cursor is, so it shows where the
    /// tab would land, and the one it left that it is gone.
    fn set_drag_target(
        &mut self,
        target: Option<DragTarget>,
        title: &SharedString,
        cursor: shell::ScreenPoint,
        cx: &mut App,
    ) {
        let changed = match (&self.drag_target, &target) {
            (Some(a), Some(b)) => !a.same(b),
            (None, None) => false,
            _ => true,
        };
        if changed && let Some(old) = self.drag_target.take() {
            match old {
                DragTarget::Local(view, handle) => {
                    handle
                        .update(cx, |_, _, cx| {
                            view.update(cx, |fm, cx| fm.clear_incoming(cx)).ok();
                        })
                        .ok();
                }
                DragTarget::Foreign(hwnd) => {
                    hooks::send_tab(hwnd, TabSignal::Leave, title, cursor, None);
                }
            }
        }
        match &target {
            Some(DragTarget::Local(view, handle)) => {
                let (view, title) = (view.clone(), title.clone());
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |fm, cx| fm.show_incoming(title, cursor, window, cx)).ok();
                    })
                    .ok();
            }
            Some(DragTarget::Foreign(hwnd)) => {
                hooks::send_tab(*hwnd, TabSignal::Hover, title, cursor, None);
            }
            None => {}
        }
        self.drag_target = target;
    }

    /// A tab from another window hovers this one at `cursor`.
    pub(super) fn show_incoming(
        &mut self,
        title: SharedString,
        cursor: shell::ScreenPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = self.insert_index(cursor, window);
        if self.incoming_tab.as_ref().is_none_or(|i| i.index != index || i.title != title) {
            self.incoming_tab = Some(IncomingTab { title, index });
            cx.notify();
        }
    }

    pub(super) fn clear_incoming(&mut self, cx: &mut Context<Self>) {
        if self.incoming_tab.take().is_some() {
            cx.notify();
        }
    }

    /// A tab another instance dropped here.
    pub(super) fn receive_tab(
        &mut self,
        mut tab: Tab,
        cursor: shell::ScreenPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tab.id = next_tab_id();
        self.incoming_tab = None;
        let index = self.insert_index(cursor, window);
        self.insert_tab_at(tab, index, window, cx);
        window.activate_window();
    }

    /// Ends a tab drag however it ended: closes the preview, clears the highlight.
    pub(super) fn end_tab_drag(&mut self, cx: &mut Context<Self>) {
        if cx.default_global::<TabDragState>().0.take().is_some() {
            cx.notify();
        }
        if let Some(preview) = self.drag_preview.take() {
            preview.close(cx);
        }
        self.set_drag_target(None, &SharedString::default(), shell::cursor_pos(), cx);
    }

    /// The mouse was released outside this window during a tab drag (the window keeps
    /// the mouse captured while a button is down, so the release still arrives here).
    /// Over another of our windows, or one of another instance, the tab moves there;
    /// anywhere else it becomes a new window. Dragging the only tab moves the window.
    pub(super) fn release_tab_outside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = cx.default_global::<TabDragState>().0.clone() else { return };
        self.end_tab_drag(cx);
        let Some(ix) = self.tabs.iter().position(|t| t.id == drag.id) else { return };
        if drag.source != cx.entity_id() {
            return;
        }
        let cursor = shell::cursor_pos();
        let tab = self.tabs[ix].clone();
        match self.target_under(cursor, cx) {
            Some(DragTarget::Local(view, handle)) => {
                self.remove_dragged_tab(ix, window, cx);
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        view.update(cx, |fm, cx| {
                            let index = fm.insert_index(cursor, window);
                            fm.clear_incoming(cx);
                            fm.insert_tab_at(tab, index, window, cx);
                        })
                        .ok();
                        window.activate_window();
                    });
                });
            }
            Some(DragTarget::Foreign(hwnd)) if hooks::send_tab(hwnd, TabSignal::Drop, &drag.title, cursor, Some(&tab)) => {
                self.remove_dragged_tab(ix, window, cx);
                shell::activate_window(hwnd);
            }
            _ if self.tabs.len() > 1 => {
                self.close_tab_at(ix, window, cx);
                cx.defer(move |cx| open_window(vec![tab], Some(cursor), cx));
            }
            _ => {
                if let Some(own) = shell::hwnd(window) {
                    place_under_cursor(own, cursor, window.scale_factor());
                }
            }
        }
    }

    /// The dragged tab moved to another window: close it here, or the window if it was
    /// the only one.
    fn remove_dragged_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 {
            window.remove_window();
        } else {
            self.close_tab_at(ix, window, cx);
        }
    }
}
