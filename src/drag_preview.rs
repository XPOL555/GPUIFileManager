//! A small always-on-top window that follows the cursor while a tab is dragged, also
//! outside the app's windows, and says what releasing the button will do. GPUI's own
//! drag preview is clipped to the window the drag started in.

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use crate::fs::Entry;
use crate::i18n;
use crate::icons::{self, IconSize};
use crate::shell;

type LucideIcon = gpui_kit::assets::IconName;

/// What releasing the tab here does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// Over the tab strip of its own window.
    Reorder,
    /// Outside any window of the app.
    NewWindow,
    /// Over another window of the app.
    MoveTo,
    /// The window's only tab, outside the window: the window follows.
    MoveWindow,
    /// Inside its own window, off the tab strip.
    Cancel,
}

pub struct DragPreview {
    entry: Entry,
    title: SharedString,
    action: DropAction,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        let theme = cx.theme();
        let (popover, text, border, muted, primary, radius, font) = (
            theme.popover,
            theme.popover_foreground,
            theme.border,
            theme.muted_foreground,
            theme.primary,
            theme.radius_lg,
            theme.font_family.clone(),
        );
        let (hint, icon, active) = match self.action {
            DropAction::Reorder => (s.drag_reorder, LucideIcon::ArrowLeftRight, true),
            DropAction::NewWindow => (s.drag_detach, LucideIcon::AppWindow, true),
            DropAction::MoveTo => (s.drag_move_to, LucideIcon::SquareArrowOutUpRight, true),
            DropAction::MoveWindow => (s.drag_move_window, LucideIcon::Move, true),
            DropAction::Cancel => (s.drag_cancel, LucideIcon::Info, false),
        };
        // The window background is transparent: only the card is drawn.
        div().size_full().p(px(6.)).font_family(font).child(
            v_flex()
                .rounded(radius)
                .bg(popover)
                .border_1()
                .border_color(if active { primary } else { border })
                .shadow_lg()
                .px_3()
                .py_2()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .text_color(text)
                        .child(icons::entry_icon(&self.entry, IconSize::Small, px(16.), cx))
                        .child(div().truncate().child(self.title.clone())),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .text_xs()
                        .text_color(if active { text } else { muted })
                        .child(Icon::new(icon).xsmall().text_color(if active { primary } else { muted }))
                        .child(hint),
                ),
        )
    }
}

pub struct DragPreviewWindow {
    handle: WindowHandle<DragPreview>,
    hwnd: isize,
    action: DropAction,
}

/// Distance of the card from the cursor, so the cursor never covers it.
const OFFSET: (f32, f32) = (14., 16.);

impl DragPreviewWindow {
    /// Opens the preview at `origin` (global logical pixels, the cursor position).
    pub fn open(entry: Entry, title: SharedString, origin: Point<Pixels>, cx: &mut App) -> Option<Self> {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: origin + point(px(OFFSET.0), px(OFFSET.1)),
                size: size(px(340.), px(78.)),
            })),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        let handle = cx
            .open_window(options, |_, cx| cx.new(|_| DragPreview { entry, title, action: DropAction::Cancel }))
            .ok()?;
        let hwnd = handle.update(cx, |_, window, _| shell::hwnd(window)).ok().flatten()?;
        // Disabled windows are invisible to `WindowFromPoint`, which finds the drop target.
        shell::disable_window(hwnd);
        Some(Self { handle, hwnd, action: DropAction::Cancel })
    }

    /// Follows the cursor (screen pixels) and shows what a release there does.
    pub fn update(&mut self, cursor: shell::ScreenPoint, scale: f32, action: DropAction, cx: &mut App) {
        let offset = ((OFFSET.0 * scale) as i32, (OFFSET.1 * scale) as i32);
        shell::set_window_origin(self.hwnd, (cursor.0 + offset.0, cursor.1 + offset.1));
        if action != self.action {
            self.action = action;
            self.handle
                .update(cx, |preview, _, cx| {
                    preview.action = action;
                    cx.notify();
                })
                .ok();
        }
    }

    pub fn close(self, cx: &mut App) {
        self.handle.update(cx, |_, window, _| window.remove_window()).ok();
    }
}
