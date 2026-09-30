//! The Recycle Bin as a place of its own: listed like a folder (see `recycle`), with
//! where each item was deleted from and when. Its items can be restored or deleted for
//! good, and the whole bin emptied; files dropped on it (its sidebar entry, its tab,
//! its listing) are deleted to it.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;

use super::files::Reselect;
use super::*;
use crate::file_ops::FileOp;
use crate::recycle;

impl FileManager {
    /// The active tab shows the Recycle Bin.
    pub(super) fn in_bin(&self) -> bool {
        fs::is_recycle_bin(&self.tab().path)
    }

    /// Selected items of the bin, as their `$R…` file and the path they had.
    fn selected_deleted(&self, cx: &App) -> Vec<(PathBuf, PathBuf)> {
        self.table.read(cx).delegate().selected_entries().into_iter().filter_map(deleted_item).collect()
    }

    fn first_selected_row(&self, cx: &App) -> usize {
        self.table.read(cx).delegate().selected_rows().first().copied().unwrap_or(0)
    }

    /// The selected items back where they were deleted from.
    pub(super) fn restore_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.selected_deleted(cx);
        if !items.is_empty() {
            let row = self.first_selected_row(cx);
            self.run_op(FileOp::Restore { items }, Reselect::Row(row), window, cx).detach();
        }
    }

    /// Everything in the bin back where it was.
    pub(super) fn restore_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items: Vec<_> = self.table.read(cx).delegate().entries().iter().cloned().filter_map(deleted_item).collect();
        if !items.is_empty() {
            self.run_op(FileOp::Restore { items }, Reselect::Keep, window, cx).detach();
        }
    }

    /// Delete in the bin: the selected items go for good, once confirmed.
    pub(super) fn purge_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = self.selected_deleted(cx).into_iter().map(|(data, _)| data).collect();
        if paths.is_empty() {
            return;
        }
        let row = self.first_selected_row(cx);
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let s = i18n::t(cx);
            let (this, paths) = (this.clone(), paths.clone());
            dialog
                .confirm()
                .title(s.purge_title)
                .description((s.purge_message)(paths.len()))
                .ok_text(s.delete_permanently)
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(s.cancel)
                .on_ok(move |_, window, cx| {
                    let paths = paths.clone();
                    this.update(cx, |fm, cx| fm.run_op(FileOp::Purge { paths }, Reselect::Row(row), window, cx).detach())
                        .ok();
                    true
                })
        });
    }

    /// Everything in the bin, gone for good; the shell asks first.
    pub(super) fn empty_bin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_op(FileOp::EmptyBin, Reselect::Keep, window, cx).detach();
    }

    /// Something changed in the bin (the app, Explorer, anyone): its icon follows, and
    /// its listing when it is shown.
    pub(super) fn bin_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        check_bin(cx);
        if self.in_bin() {
            self.folder_changed(window, cx);
        }
    }
}

/// Looks at whether the bin is empty on a background thread; its icon changes with it.
pub(super) fn check_bin(cx: &mut App) {
    let checked = cx.background_spawn(async { recycle::check() });
    cx.spawn(async move |cx| {
        if checked.await {
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

/// An item of the bin as its `$R…` file and the path it had.
fn deleted_item(e: fs::Entry) -> Option<(PathBuf, PathBuf)> {
    let original = e.origin.as_ref()?.join(e.name.as_ref());
    Some((e.path, original))
}
