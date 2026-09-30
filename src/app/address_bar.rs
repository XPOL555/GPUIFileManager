//! Address bar: navigation buttons, breadcrumb, and the editable path with folder
//! autocompletion, plus the filter box.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, ElementExt as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;

use super::*;
use crate::dnd;
use crate::fs::Entry;
use crate::icons::{self, IconSize};

/// Address-bar completions shown at once. Keeps the list short enough to never scroll.
const MAX_SUGGESTIONS: usize = 12;

pub(super) struct Suggestion {
    label: SharedString,
    path: PathBuf,
}

impl FileManager {
    pub(super) fn on_path_input_event(
        &mut self,
        input: &Entity<InputState>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                self.update_suggestions(text, cx);
            }
            InputEvent::PressEnter { .. } => {
                let target = match self.suggestion_ix.and_then(|ix| self.suggestions.get(ix)) {
                    Some(s) => s.path.clone(),
                    None => PathBuf::from(input.read(cx).value().trim()),
                };
                self.finish_edit_path(window, cx);
                self.navigate(target, true, window, cx);
            }
            InputEvent::Blur => {
                self.editing_path = false;
                self.suggestions.clear();
                cx.notify();
            }
            _ => {}
        }
    }

    pub(super) fn edit_path(&mut self, _: &EditPath, window: &mut Window, cx: &mut Context<Self>) {
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
        self.focus_view(window, cx);
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
                .map(|d| Suggestion { label: drive_label(d).into(), path: d.root.clone() })
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
                this.suggestions =
                    names.into_iter().map(|name| Suggestion { path: dir.join(&name), label: name.into() }).collect();
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

    pub(super) fn render_address_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            // The Recycle Bin is a place of its own, with nothing above it.
            let ancestors: Vec<PathBuf> = if fs::is_recycle_bin(&tab.path) {
                vec![tab.path.clone()]
            } else {
                tab.path.ancestors().map(Path::to_path_buf).collect()
            };
            for (i, p) in ancestors.into_iter().rev().enumerate() {
                let name = if fs::is_recycle_bin(&p) {
                    folder_name(&p, cx)
                } else {
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.to_string_lossy().trim_end_matches('\\').to_string())
                };
                if i > 0 {
                    crumbs = crumbs.child(Icon::new(IconName::ChevronRight).xsmall().text_color(cx.theme().muted_foreground));
                }
                // Files can be dropped on an ancestor folder.
                let files_over = self.drop_hover == Some(SpotKey::Crumb(i));
                let zone = dnd::zone(&self.drop_zones, SpotKey::Crumb(i), p.clone(), 2);
                let button = Button::new(("crumb", i)).ghost().small().label(name).on_click(cx.listener(
                    move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.navigate(p.clone(), true, window, cx);
                    },
                ));
                crumbs = crumbs.child(
                    div()
                        .rounded(cx.theme().radius)
                        .when(files_over, |d| d.bg(cx.theme().drop_target))
                        .child(button)
                        .on_prepaint(zone),
                );
            }
            crumbs.on_click(cx.listener(|this, _, window, cx| this.edit_path(&EditPath, window, cx))).into_any_element()
        };

        h_flex()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(nav("back", IconName::ArrowLeft, !tab.back.is_empty(), Box::new(GoBack)))
            .child(nav("forward", IconName::ArrowRight, !tab.forward.is_empty(), Box::new(GoForward)))
            .child(nav("up", IconName::ArrowUp, fs::parent(&tab.path).is_some(), Box::new(GoUp)))
            .child(nav("refresh", IconName::RefreshCw, true, Box::new(Refresh)))
            .child(location)
            .child(
                div().w(px(220.)).child(
                    Input::new(&self.filter).small().cleanable(true).prefix(Icon::new(IconName::Search).small()),
                ),
            )
    }

    /// Completion list under the path input. Deferred so it paints over the files.
    fn render_suggestions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .suggestions
            .iter()
            .enumerate()
            .map(|(ix, s)| {
                let path = s.path.clone();
                let theme = cx.theme();
                let (accent, radius) = (theme.accent, theme.radius);
                h_flex()
                    .id(("suggestion", ix))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(radius)
                    .cursor_pointer()
                    .when(self.suggestion_ix == Some(ix), |d| d.bg(accent))
                    .hover(move |d| d.bg(accent.opacity(0.6)))
                    .child(icons::entry_icon(&Entry::folder(&s.path), IconSize::Small, px(16.), cx))
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
                    .into_any_element()
            })
            .collect();
        let theme = cx.theme();
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
}
