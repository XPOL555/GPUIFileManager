//! Tab strip and tab dragging: drop on the strip to reorder, on another window to
//! move the tab there, anywhere else to open it in a new window. While dragging, a
//! floating preview (`drag_preview`) follows the cursor and says what will happen,
//! and a window the tab would move to highlights its tab strip.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, ElementExt as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;

use super::sidebar::DragGhost;
use super::*;
use crate::drag_preview::DropAction;
use crate::fs::Entry;
use crate::icons::{self, IconSize};

/// Payload of a tab drag.
#[derive(Clone)]
pub(super) struct DraggedTab {
    source: EntityId,
    ix: usize,
    title: SharedString,
    path: PathBuf,
}

/// The tab being dragged, from drag start until the button is released. GPUI drops
/// its own drag state on a release that no drop target takes, which is exactly the
/// case (released outside the window) that has to detach the tab.
#[derive(Default)]
struct TabDragState(Option<DraggedTab>);

impl Global for TabDragState {}

impl FileManager {
    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let source = cx.entity_id();
        let strip_bounds = self.tab_strip.clone();
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
            .on_drop(cx.listener(|this, drag: &DraggedTab, _, cx| this.drop_tab(drag, None, cx)));
        for (ix, tab) in self.tabs.iter().enumerate() {
            let active = ix == self.active;
            let theme = cx.theme();
            let (background, accent, radius) = (theme.background, theme.accent, theme.radius);
            let dragged = DraggedTab { source, ix, title: tab.title(), path: tab.path.clone() };
            strip = strip.child(
                h_flex()
                    .id(("tab", ix))
                    .gap_2()
                    .pl_3()
                    .pr_1()
                    .h(px(28.))
                    .max_w(px(220.))
                    .rounded(radius)
                    .when(active, |d| d.bg(background))
                    .hover(move |d| d.bg(accent))
                    .cursor_pointer()
                    .child(icons::entry_icon(&Entry::folder(&tab.path), IconSize::Small, px(16.), cx))
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
                    // The preview is a window of its own (`drag_preview`); GPUI's would be clipped.
                    .on_drag(dragged, |drag, _, _, cx| {
                        cx.default_global::<TabDragState>().0 = Some(drag.clone());
                        cx.new(|_| DragGhost)
                    })
                    .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(accent))
                    .on_drop(cx.listener(move |this, drag: &DraggedTab, _, cx| this.drop_tab(drag, Some(ix), cx))),
            );
        }
        strip
            .child(
                Button::new("new-tab")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .on_click(cx.listener(|this, _, window, cx| this.new_tab(&NewTab, window, cx))),
            )
            // A tab from another window hovers here: show where it will land.
            .when_some(self.incoming_tab.clone(), |strip, title| {
                strip.bg(drop_target).child(
                    h_flex()
                        .gap_2()
                        .px_3()
                        .h(px(28.))
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(drag_border)
                        .bg(cx.theme().background.opacity(0.7))
                        .child(Icon::new(IconName::Folder).small().text_color(drag_border))
                        .child(title),
                )
            })
    }

    /// Every mouse move of a tab drag (the source window keeps the mouse captured, so
    /// this also runs outside it): move the preview and update what a release would do.
    pub(super) fn tab_drag_moved(&mut self, ev: &DragMoveEvent<DraggedTab>, window: &mut Window, cx: &mut Context<Self>) {
        let drag = ev.drag(cx).clone();
        if drag.source != cx.entity_id() {
            return;
        }
        let position = ev.event.position;
        let cursor = shell::cursor_pos();
        let inside = Bounds::new(Point::default(), window.viewport_size()).contains(&position);
        let (action, target) = if inside {
            let action = if self.tab_strip.get().contains(&position) { DropAction::Reorder } else { DropAction::Cancel };
            (action, None)
        } else {
            match self.window_under(cursor, window, cx) {
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
        self.set_drag_target(target, drag.title, cx);
    }

    /// Another window of the app under the cursor.
    fn window_under(
        &self,
        cursor: shell::ScreenPoint,
        window: &Window,
        cx: &mut App,
    ) -> Option<(WeakEntity<FileManager>, AnyWindowHandle)> {
        let under = shell::root_window_at(cursor);
        let own = shell::hwnd(window).unwrap_or(0);
        cx.default_global::<OpenWindows>()
            .0
            .iter()
            .find(|w| w.hwnd == under && under != own && w.view.upgrade().is_some())
            .map(|w| (w.view.clone(), w.handle))
    }

    /// Highlights the tab strip of the window the tab would move to.
    fn set_drag_target(
        &mut self,
        target: Option<(WeakEntity<FileManager>, AnyWindowHandle)>,
        title: SharedString,
        cx: &mut App,
    ) {
        let same = match (&self.drag_target, &target) {
            (Some((a, _)), Some((b, _))) => a.entity_id() == b.entity_id(),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        let mark = |target: &(WeakEntity<FileManager>, AnyWindowHandle), title: Option<SharedString>, cx: &mut App| {
            let (view, handle) = target.clone();
            handle
                .update(cx, |_, _, cx| {
                    view.update(cx, |fm, cx| {
                        fm.incoming_tab = title;
                        cx.notify();
                    })
                    .ok();
                })
                .ok();
        };
        if let Some(old) = self.drag_target.take() {
            mark(&old, None, cx);
        }
        if let Some(new) = &target {
            mark(new, Some(title), cx);
        }
        self.drag_target = target;
    }

    /// Ends a tab drag however it ended: closes the preview, clears the highlight.
    pub(super) fn end_tab_drag(&mut self, cx: &mut Context<Self>) {
        cx.default_global::<TabDragState>().0 = None;
        if let Some(preview) = self.drag_preview.take() {
            preview.close(cx);
        }
        self.set_drag_target(None, SharedString::default(), cx);
    }

    /// A tab dropped on this window's tab strip: reorder. `to` is the tab it was dropped
    /// on, `None` for the empty end of the strip.
    fn drop_tab(&mut self, drag: &DraggedTab, to: Option<usize>, cx: &mut Context<Self>) {
        self.end_tab_drag(cx);
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
    pub(super) fn release_tab_outside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = cx.default_global::<TabDragState>().0.take() else { return };
        self.end_tab_drag(cx);
        if drag.source != cx.entity_id() || drag.ix >= self.tabs.len() {
            return;
        }
        let cursor = shell::cursor_pos();
        let target = self.window_under(cursor, window, cx);
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
            None => {
                if let Some(own) = shell::hwnd(window) {
                    place_under_cursor(own, cursor, window.scale_factor());
                }
            }
        }
    }
}
