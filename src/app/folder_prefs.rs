//! Sorting, and the view and sorting kept for single folders: column header clicks,
//! "Keep for this folder" (right click on a column header or on the view button) and
//! the one-time snackbar that offers it the first time the sorting or the view changes.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;

use super::menu::menu_item;
use super::*;
use crate::fs::SortKey;

/// How long the snackbar stays up when left alone.
const SNACKBAR_TIMEOUT: Duration = Duration::from_secs(15);

/// What can be kept for a folder.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Pref {
    Sort,
    View,
}

/// The snackbar offering to keep the sorting or the view for the current folder.
pub(super) struct Snackbar {
    pref: Pref,
    /// Distinct per snackbar, so each one plays its entrance.
    id: usize,
    _timeout: Task<()>,
}

impl FileManager {
    /// A click on a column header: sorts by it, or flips the direction if already sorted by it.
    pub(super) fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let sort = self.tab().sort.clicked(key);
        let keep = self.selected_entry(cx).map(|e| e.name.clone());
        let tab = &mut self.tabs[self.active];
        tab.sort = sort;
        tab.picked_sort = sort;
        self.table.update(cx, |t, cx| {
            t.delegate_mut().set_sort(sort);
            cx.notify();
        });
        self.reselect(keep, cx);
        Settings::update(cx, |s| s.sort = sort);
        self.offer_to_keep(Pref::Sort, cx);
        cx.notify();
    }

    fn kept(&self, pref: Pref, cx: &App) -> bool {
        let prefs = Settings::get(cx).folder(&self.tab().path);
        match pref {
            Pref::Sort => prefs.sort == Some(self.tab().sort),
            Pref::View => prefs.view == Some(self.view()),
        }
    }

    /// Keeps the current sorting or view for the current folder: it comes back every
    /// time the folder is opened, in any tab.
    pub(super) fn keep(&mut self, pref: Pref, cx: &mut Context<Self>) {
        let (path, sort, view) = (self.tab().path.clone(), self.tab().sort, self.view());
        Settings::update(cx, |s| {
            s.edit_folder(&path, |p| match pref {
                Pref::Sort => p.sort = Some(sort),
                Pref::View => p.view = Some(view),
            })
        });
        if self.snackbar.as_ref().is_some_and(|bar| bar.pref == pref) {
            self.snackbar = None;
        }
        cx.notify();
    }

    fn forget(&mut self, pref: Pref, cx: &mut Context<Self>) {
        let path = self.tab().path.clone();
        Settings::update(cx, |s| {
            s.edit_folder(&path, |p| match pref {
                Pref::Sort => p.sort = None,
                Pref::View => p.view = None,
            })
        });
        cx.notify();
    }

    /// The first time ever the sorting (or the view) changes, offers to keep it for the folder.
    pub(super) fn offer_to_keep(&mut self, pref: Pref, cx: &mut Context<Self>) {
        let settings = Settings::get(cx);
        let shown = match pref {
            Pref::Sort => settings.sort_tip_shown,
            Pref::View => settings.view_tip_shown,
        };
        if shown || self.kept(pref, cx) {
            return;
        }
        Settings::update(cx, |s| match pref {
            Pref::Sort => s.sort_tip_shown = true,
            Pref::View => s.view_tip_shown = true,
        });
        let timeout = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SNACKBAR_TIMEOUT).await;
            this.update(cx, |this, cx| this.dismiss_snackbar(cx)).ok();
        });
        let id = self.snackbar.as_ref().map_or(0, |bar| bar.id + 1);
        self.snackbar = Some(Snackbar { pref, id, _timeout: timeout });
        cx.notify();
    }

    pub(super) fn dismiss_snackbar(&mut self, cx: &mut Context<Self>) {
        if self.snackbar.take().is_some() {
            cx.notify();
        }
    }

    /// Right click on a column header, or on the view button.
    pub(super) fn show_keep_menu(&mut self, pref: Pref, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let s = i18n::t(cx);
        let prefs = Settings::get(cx).folder(&self.tab().path);
        let saved = match pref {
            Pref::Sort => prefs.sort.is_some(),
            Pref::View => prefs.view.is_some(),
        };
        let kept = self.kept(pref, cx);
        let (keep, forget) = match pref {
            Pref::Sort => (s.keep_sort, s.forget_sort),
            Pref::View => (s.keep_view, s.forget_view),
        };
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            menu.min_w(px(230.))
                .when(!kept, |menu| menu.item(menu_item(&this, keep, LucideIcon::Pin, move |fm, _, cx| fm.keep(pref, cx))))
                .when(saved, |menu| {
                    menu.item(menu_item(&this, forget, LucideIcon::PinOff, move |fm, _, cx| fm.forget(pref, cx)))
                })
        });
        self.show_popup(menu, position, window, cx);
    }

    /// Whether the view shown is the one kept for the folder, for the view button.
    pub(super) fn view_kept(&self, cx: &App) -> bool {
        self.kept(Pref::View, cx)
    }

    /// Keeps the view for the folder, or forgets it: the toggle in the view picker.
    pub(super) fn toggle_keep_view(&mut self, cx: &mut Context<Self>) {
        if self.kept(Pref::View, cx) { self.forget(Pref::View, cx) } else { self.keep(Pref::View, cx) }
    }

    /// Bottom center of the file view.
    pub(super) fn render_snackbar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let bar = self.snackbar.as_ref()?;
        let s = i18n::t(cx);
        let pref = bar.pref;
        let (title, hint) = match pref {
            Pref::Sort => (s.sort_tip, s.sort_tip_hint),
            Pref::View => (s.view_tip, s.view_tip_hint),
        };
        let theme = cx.theme();
        let card = h_flex()
            .id(("snackbar", bar.id))
            .occlude()
            .max_w(px(620.))
            .gap_3()
            .pl_3()
            .pr_1p5()
            .py_2()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .shadow_lg()
            .child(Icon::new(LucideIcon::Pin).small().text_color(theme.primary))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(hint)),
            )
            .child(
                Button::new("snackbar-keep")
                    .primary()
                    .small()
                    .label(s.keep)
                    .on_click(cx.listener(move |this, _, _, cx| this.keep(pref, cx))),
            )
            .child(
                Button::new("snackbar-dismiss")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(s.dismiss)
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_snackbar(cx))),
            )
            .with_animation(
                ("snackbar-in", bar.id),
                Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                |card, t| card.opacity(t).relative().top(px(12. * (1. - t))),
            );
        Some(h_flex().absolute().left_0().right_0().bottom_3().px_3().justify_center().child(card))
    }
}
