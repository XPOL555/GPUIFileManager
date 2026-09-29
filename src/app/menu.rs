//! The app's context menu, for files, folders, the folder background and sidebar
//! items. A small last entry (or Shift + right click) opens Explorer's own menu.

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;

use super::*;
use crate::fs::Entry;

/// What was right-clicked.
pub(super) struct MenuTarget {
    /// `None`: the background of the current folder.
    entry: Option<Entry>,
    /// Right-clicked in the favorites list.
    favorite: bool,
}

impl MenuTarget {
    pub(super) fn from_entry(entry: Option<Entry>) -> Self {
        Self { entry, favorite: false }
    }

    pub(super) fn background() -> Self {
        Self { entry: None, favorite: false }
    }

    /// A sidebar folder: a favorite or a drive.
    pub(super) fn sidebar(path: &std::path::Path, favorite: bool) -> Self {
        Self { entry: Some(Entry::folder(path)), favorite }
    }

    /// What Explorer's menu applies to: the item, or the folder background.
    fn native_paths(&self) -> Vec<PathBuf> {
        self.entry.iter().map(|e| e.path.clone()).collect()
    }
}

pub(super) struct OpenMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    _dismiss: Subscription,
}

impl OpenMenu {
    pub(super) fn render(&self) -> impl IntoElement {
        deferred(anchored().position(self.position).snap_to_window_with_margin(px(8.)).child(self.menu.clone()))
            .with_priority(1)
    }
}

impl FileManager {
    /// Right mouse-down: the app menu, or Explorer's with Shift held.
    pub(super) fn show_menu(&mut self, target: MenuTarget, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let at = shell::cursor_pos();
        if ev.modifiers.shift {
            return self.open_native_menu(target.native_paths(), at, window, cx);
        }
        self.open_menu(target, ev.position, at, window, cx);
    }

    fn open_menu(
        &mut self,
        target: MenuTarget,
        position: Point<Pixels>,
        at: shell::ScreenPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        let s = i18n::t(cx);
        let folder = self.tab().path.clone();
        let show_hidden = self.table.read(cx).delegate().show_hidden();
        let favorites = self.favorites(cx);
        let view_focus = self.view_focus(cx);
        let native_paths = target.native_paths();

        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            let mut menu = menu.action_context(view_focus).min_w(px(230.));
            let path = target.entry.as_ref().map_or(&folder, |e| &e.path).clone();
            let is_dir = target.entry.as_ref().is_none_or(|e| e.is_dir);
            menu = match &target.entry {
                Some(e) => {
                    let open_icon = if e.is_dir { LucideIcon::FolderOpen } else { LucideIcon::ExternalLink };
                    let e = e.clone();
                    menu.item(menu_item(&this, s.open, open_icon, move |fm, window, cx| fm.open_entry(e.clone(), window, cx)))
                        .when(is_dir, |menu| open_elsewhere_items(menu, &this, s, &path))
                }
                None => menu
                    .item(menu_item(&this, s.refresh, LucideIcon::RefreshCw, |fm, window, cx| fm.refresh(&Refresh, window, cx)))
                    .item({
                        let (label, icon) = if show_hidden {
                            (s.hide_hidden, LucideIcon::EyeOff)
                        } else {
                            (s.show_hidden, LucideIcon::Eye)
                        };
                        menu_item(&this, label, icon, |fm, window, cx| fm.toggle_hidden(&ToggleHidden, window, cx))
                    })
                    .separator()
                    .map(|menu| open_elsewhere_items(menu, &this, s, &path)),
            };
            if target.favorite {
                let removed = path.clone();
                menu = menu.separator().item(menu_item(&this, s.remove_favorite, LucideIcon::StarOff, move |fm, _, cx| {
                    fm.remove_favorite(&removed, cx)
                }));
            } else if is_dir && !favorites.contains(&path) {
                let label = if target.entry.is_some() { s.add_favorite } else { s.add_folder_favorite };
                let added = path.clone();
                menu = menu.item(menu_item(&this, label, LucideIcon::Star, move |fm, _, cx| fm.add_favorite(added.clone(), cx)));
            }
            let (copied, props) = (path.clone(), path.clone());
            menu.separator()
                .item(menu_item(&this, s.copy_path, LucideIcon::Copy, move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copied.to_string_lossy().into_owned()))
                }))
                .item(menu_item(&this, s.properties, LucideIcon::Info, move |_, _, _| shell::show_properties(&props)))
                .separator()
                .item({
                    let this = this.clone();
                    PopupMenuItem::element(move |_, cx| {
                        div().text_xs().text_color(cx.theme().muted_foreground).child(s.more_options)
                    })
                    .icon(Icon::new(LucideIcon::Ellipsis).xsmall())
                    .on_click(move |_, window, cx| {
                        let paths = native_paths.clone();
                        this.update(cx, |fm, cx| fm.open_native_menu(paths, at, window, cx)).ok();
                    })
                })
        });
        self.show_popup(menu, position, window, cx);
    }

    /// Shows `menu` at `position` (window coordinates) until it is dismissed.
    pub(super) fn show_popup(
        &mut self,
        menu: Entity<PopupMenu>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismiss = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.context_menu = None;
            cx.notify();
        });
        menu.focus_handle(cx).focus(window, cx);
        self.context_menu = Some(OpenMenu { menu, position, _dismiss: dismiss });
        cx.notify();
    }

    /// Explorer's own menu. It runs a nested Win32 modal loop; running it inside an
    /// event handler would re-enter GPUI while `App` is borrowed, so it is deferred to a
    /// foreground task where no GPUI borrow is held. The folder is reloaded afterwards
    /// (the command may have renamed, deleted or created files).
    pub(super) fn open_native_menu(
        &mut self,
        paths: Vec<PathBuf>,
        at: shell::ScreenPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = self.tab().path.clone();
        cx.spawn_in(window, async move |this, cx| {
            if let Err(err) = shell::show_context_menu(&folder, &paths, at) {
                eprintln!("context menu: {err:?}");
            }
            this.update_in(cx, |this, window, cx| this.refresh(&Refresh, window, cx)).ok();
        })
        .detach();
    }
}

/// A context-menu entry whose handler runs on the `FileManager` behind `this`.
pub(super) fn menu_item(
    this: &WeakEntity<FileManager>,
    label: &'static str,
    icon: impl Into<Icon>,
    handler: impl Fn(&mut FileManager, &mut Window, &mut Context<FileManager>) + 'static,
) -> PopupMenuItem {
    let this = this.clone();
    PopupMenuItem::new(label).icon(icon).on_click(move |_, window, cx| {
        this.update(cx, |fm, cx| handler(fm, window, cx)).ok();
    })
}

/// "Open in new tab" / "Open in new window" for a folder.
fn open_elsewhere_items(
    menu: PopupMenu,
    this: &WeakEntity<FileManager>,
    s: &'static i18n::Strings,
    folder: &Path,
) -> PopupMenu {
    let (tab_path, window_path) = (folder.to_path_buf(), folder.to_path_buf());
    menu.item(menu_item(this, s.open_new_tab, LucideIcon::SquarePlus, move |fm, window, cx| {
        fm.open_tab(tab_path.clone(), window, cx)
    }))
    .item(menu_item(this, s.open_new_window, LucideIcon::AppWindow, move |_, _, cx| {
        let tab = Tab::new(window_path.clone(), cx);
        cx.defer(move |cx| open_window(vec![tab], None, cx));
    }))
}
