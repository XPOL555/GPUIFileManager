//! About dialog: logo, name, version, a link to the repository and the update check
//! (see `crate::update`).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, v_flex};

use super::*;
use crate::update::{self, REPO_URL, UpdateStatus, VERSION};

impl FileManager {
    pub(super) fn open_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let logo = self.logo.clone();
        // The builder runs on every render, so the dialog follows the check live.
        window.open_dialog(cx, move |dialog, _, cx| {
            let s = i18n::t(cx);
            let muted = cx.theme().muted_foreground;
            let check = |label: &'static str, loading: bool| {
                Button::new("check-updates")
                    .small()
                    .outline()
                    .label(label)
                    .loading(loading)
                    .on_click(|_, _, cx| update::check(cx))
            };
            let updates = match update::status(cx) {
                UpdateStatus::Idle => v_flex().child(check(s.check_updates, false)),
                UpdateStatus::Checking => v_flex().child(check(s.checking, true)),
                UpdateStatus::UpToDate => {
                    v_flex().gap_2().child(div().text_color(muted).child(s.up_to_date)).child(check(s.check_updates, false))
                }
                UpdateStatus::Failed => v_flex()
                    .gap_2()
                    .child(div().text_color(cx.theme().danger).child(s.update_failed))
                    .child(check(s.check_updates, false)),
                UpdateStatus::Available { version, url } => v_flex()
                    .gap_2()
                    .child(div().child((s.update_available)(&version)))
                    .child(
                        Button::new("open-release")
                            .small()
                            .primary()
                            .label(s.open_release)
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    ),
            };
            let repo = REPO_URL.trim_start_matches("https://");

            dialog.title(s.about).w(px(380.)).child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .pb_2()
                    .child(img(logo.clone()).size_16().mb_2())
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("FileManager"))
                    .child(div().text_color(muted).child((s.version)(VERSION)))
                    .child(div().text_xs().text_color(muted).child(s.about_description))
                    .child(
                        div()
                            .id("repo-link")
                            .text_xs()
                            .text_color(cx.theme().link)
                            .cursor_pointer()
                            .hover(|d| d.underline())
                            .on_click(|_, _, cx| cx.open_url(REPO_URL))
                            .child(repo),
                    )
                    .child(updates.items_center().pt_3()),
            )
        });
    }
}
