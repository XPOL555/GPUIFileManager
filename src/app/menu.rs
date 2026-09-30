//! The app's context menus (drawn by `context_menu`): for the selected items, the
//! folder's background, sidebar entries and the Recycle Bin. Explorer's own entries
//! ("Open with", what apps add, "Send to", "New"…) join them as they load
//! (`shell_menu`); the last entry, or Shift + right click, opens Explorer's own menu.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use futures::StreamExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::PopupMenuItem;

use super::*;
use crate::context_menu::{ContextMenu, MenuEntry, MenuItem, ToolButton};
use crate::file_ops;
use crate::fs::{Entry, Sort, SortKey};
use crate::shell_images::Bitmap;
use crate::shell_menu::{self, Late, ShellItem, ShellMenu};

/// Where Explorer's "Open with" goes: right after Open.
const OPEN_WITH: u8 = 0;
/// Where the rest of Explorer's entries go.
const SHELL: u8 = 1;

/// What was right-clicked.
pub(super) enum MenuTarget {
    /// The selected items of the file view.
    Items(Vec<Entry>),
    /// The background of the listed folder.
    Background,
    /// A folder of the sidebar (a favorite or a drive), or the Recycle Bin.
    Sidebar { path: PathBuf, favorite: bool },
}

impl MenuTarget {
    pub(super) fn items(entries: Vec<Entry>) -> Self {
        MenuTarget::Items(entries)
    }

    pub(super) fn background() -> Self {
        MenuTarget::Background
    }

    pub(super) fn sidebar(path: &Path, favorite: bool) -> Self {
        MenuTarget::Sidebar { path: path.to_path_buf(), favorite }
    }
}

pub(super) struct OpenMenu {
    pub(super) menu: Entity<ContextMenu>,
    _dismiss: Subscription,
    /// Explorer's entries, on their way.
    _shell: Option<Task<()>>,
}

/// A handler that runs on the `FileManager` behind `this`.
fn on_fm(
    this: &WeakEntity<FileManager>,
    f: impl Fn(&mut FileManager, &mut Window, &mut Context<FileManager>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let this = this.clone();
    move |window, cx| {
        this.update(cx, |fm, cx| f(fm, window, cx)).ok();
    }
}

fn entry(
    this: &WeakEntity<FileManager>,
    label: &'static str,
    icon: LucideIcon,
    f: impl Fn(&mut FileManager, &mut Window, &mut Context<FileManager>) + 'static,
) -> MenuEntry {
    MenuEntry::new(label).icon(icon).on_click(on_fm(this, f))
}

fn tool(
    this: &WeakEntity<FileManager>,
    icon: LucideIcon,
    label: String,
    f: impl Fn(&mut FileManager, &mut Window, &mut Context<FileManager>) + 'static,
) -> ToolButton {
    ToolButton::new(icon, label, on_fm(this, f))
}

impl FileManager {
    /// Right mouse-down at `position`: the app's menu, or Explorer's with Shift held.
    pub(super) fn show_menu(
        &mut self,
        target: MenuTarget,
        position: Point<Pixels>,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = shell::cursor_pos();
        let bin = self.in_bin() || matches!(&target, MenuTarget::Sidebar { path, .. } if fs::is_recycle_bin(path));
        if bin {
            let items = self.bin_menu(&target, cx);
            return self.show_items(items, position, None, at, window, cx);
        }
        let (folder, paths) = self.shell_request(&target);
        if shift {
            return self.open_native_menu(paths, at, window, cx);
        }
        let items = match &target {
            MenuTarget::Items(entries) => self.items_menu(entries, at, cx),
            MenuTarget::Background => self.background_menu_items(at, cx),
            MenuTarget::Sidebar { path, favorite } => self.sidebar_menu(path, *favorite, at, cx),
        };
        self.show_items(items, position, Some((folder, paths)), at, window, cx);
    }

    /// The folder Explorer's menu is about, and its items (none: the folder's background).
    fn shell_request(&self, target: &MenuTarget) -> (PathBuf, Vec<PathBuf>) {
        match target {
            MenuTarget::Items(entries) => (self.tab().path.clone(), entries.iter().map(|e| e.path.clone()).collect()),
            MenuTarget::Background => (self.tab().path.clone(), Vec::new()),
            MenuTarget::Sidebar { path, .. } => (path.parent().unwrap_or(path).to_path_buf(), vec![path.clone()]),
        }
    }

    /// Shows `items` at `position` (window coordinates). Explorer's entries for
    /// `shell` (a folder and its items) fill the menu's slots as they load.
    pub(super) fn show_items(
        &mut self,
        items: Vec<MenuItem>,
        position: Point<Pixels>,
        shell: Option<(PathBuf, Vec<PathBuf>)>,
        at: shell::ScreenPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| ContextMenu::new(items, position, window, cx));
        let dismiss = cx.subscribe_in(&menu, window, |this, menu, _: &DismissEvent, window, cx| {
            // A menu opened meanwhile (a right click elsewhere) stays.
            if this.context_menu.as_ref().is_some_and(|open| open.menu == *menu) {
                // Back to the files, unless the command put the keyboard elsewhere.
                let refocus = menu.read(cx).focus_handle(cx).is_focused(window);
                this.context_menu = None;
                if refocus {
                    this.focus_view(window, cx);
                }
                cx.notify();
            }
        });
        let loading = shell.map(|(folder, paths)| {
            menu.update(cx, |menu, cx| menu.set_loading(true, cx));
            self.load_shell_entries(&menu, folder, paths, at, cx)
        });
        menu.read(cx).focus_handle(cx).focus(window, cx);
        self.context_menu = Some(OpenMenu { menu, _dismiss: dismiss, _shell: loading });
        cx.notify();
    }

    /// Reads Explorer's menu on its thread, then puts its entries in the open menu,
    /// and their icons as they come.
    fn load_shell_entries(
        &self,
        menu: &Entity<ContextMenu>,
        folder: PathBuf,
        paths: Vec<PathBuf>,
        at: shell::ScreenPoint,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let loading = shell_menu::load(folder, paths, false, self.hwnd);
        let menu = menu.downgrade();
        cx.spawn(async move |_, cx| {
            let Some(mut shell) = loading.await else {
                cx.update(|cx| menu.update(cx, |menu, cx| menu.set_loading(false, cx)).ok());
                return;
            };
            let late = shell.late.take();
            let items = std::mem::take(&mut shell.items);
            // The entries' handlers keep Explorer's menu alive while the menu is open.
            let shell = Rc::new(shell);
            let fill = |items: Vec<ShellItem>, cx: &mut App| {
                let (open_with, rest) = shell_entries(items, &shell, at, cx);
                menu.update(cx, |menu, cx| {
                    menu.set_loading(false, cx);
                    menu.fill_slot(OPEN_WITH, open_with, cx);
                    menu.fill_slot(SHELL, rest, cx);
                })
                .is_ok()
            };
            if !cx.update(|cx| fill(items, cx)) {
                return;
            }
            let Some(mut late) = late else { return };
            while let Some(late) = late.next().await {
                let open = cx.update(|cx| match late {
                    Late::Entries(items) => fill(items, cx),
                    Late::Icons(icons) => {
                        let images: Vec<_> = icons.into_iter().map(|(key, icon)| (key, menu_image(icon, cx))).collect();
                        menu.update(cx, |menu, cx| {
                            for (key, image) in images {
                                menu.set_image(key, image, cx);
                            }
                        })
                        .is_ok()
                    }
                });
                if !open {
                    return;
                }
            }
        })
    }

    /// The menu of the selected items.
    fn items_menu(&self, entries: &[Entry], at: shell::ScreenPoint, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let s = i18n::t(cx);
        let this = cx.entity().downgrade();
        let single = match entries {
            [e] => Some(e),
            _ => None,
        };
        let mut items = vec![
            // Windows 11's row of icons.
            MenuItem::Toolbar(vec![
                tool(&this, LucideIcon::Scissors, format!("{} (Ctrl+X)", s.cut), |fm, _, cx| fm.put_on_clipboard(true, cx)),
                tool(&this, LucideIcon::Copy, format!("{} (Ctrl+C)", s.copy), |fm, _, cx| fm.put_on_clipboard(false, cx)),
                tool(&this, LucideIcon::PencilLine, format!("{} (F2)", s.rename), |fm, window, cx| {
                    fm.rename_selected(&RenameSelected, window, cx)
                })
                .disabled(single.is_none()),
                tool(&this, LucideIcon::Trash, format!("{} ({})", s.delete, s.key_delete), |fm, window, cx| {
                    fm.delete(true, window, cx)
                }),
            ]),
            MenuItem::Separator,
        ];
        match single {
            Some(e) => {
                let icon = if e.is_dir { LucideIcon::FolderOpen } else { LucideIcon::ExternalLink };
                let opened = e.clone();
                items.push(
                    entry(&this, s.open, icon, move |fm, window, cx| fm.open_entry(opened.clone(), window, cx))
                        .shortcut(s.key_enter)
                        .into(),
                );
                if e.is_dir {
                    items.extend(open_elsewhere(&this, s, &e.path));
                }
                if let Some(target) = e.is_shortcut().then(|| shell::shortcut_target(&e.path)).flatten() {
                    items.push(
                        entry(&this, s.open_file_location, LucideIcon::FolderSymlink, move |fm, window, cx| {
                            fm.reveal(target.clone(), window, cx)
                        })
                        .into(),
                    );
                }
            }
            None => items.push(
                entry(&this, s.open, LucideIcon::ExternalLink, |fm, window, cx| fm.open_selected(&OpenSelected, window, cx))
                    .shortcut(s.key_enter)
                    .into(),
            ),
        }
        items.extend([MenuItem::Slot(OPEN_WITH), MenuItem::Separator, MenuItem::Slot(SHELL), MenuItem::Separator]);
        if let Some(e) = single.filter(|e| e.is_dir) {
            if file_ops::clipboard_has_files() {
                let dest = e.path.clone();
                items.push(
                    entry(&this, s.paste_into, LucideIcon::ClipboardPaste, move |fm, window, cx| {
                        fm.paste_into(dest.clone(), window, cx)
                    })
                    .into(),
                );
            }
            items.extend(self.favorite_toggle(&e.path, s.add_favorite, cx));
        }
        let paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
        items.extend(tail(&this, s, paths.clone(), paths, at));
        items
    }

    /// The menu of the folder's background.
    fn background_menu_items(&self, at: shell::ScreenPoint, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let s = i18n::t(cx);
        let this = cx.entity().downgrade();
        let folder = self.tab().path.clone();
        let show_hidden = Settings::get(cx).show_hidden;
        let (label, icon) = if show_hidden {
            (s.hide_hidden, LucideIcon::EyeOff)
        } else {
            (s.show_hidden, LucideIcon::Eye)
        };
        let paste_to = folder.clone();
        let mut items = vec![
            self.view_submenu(&this, cx).into(),
            self.sort_submenu(&this, cx).into(),
            entry(&this, s.refresh, LucideIcon::RefreshCw, |fm, window, cx| fm.refresh(&Refresh, window, cx))
                .shortcut("F5")
                .into(),
            entry(&this, label, icon, |fm, window, cx| fm.toggle_hidden(&ToggleHidden, window, cx)).shortcut("Ctrl+H").into(),
            MenuItem::Separator,
            entry(&this, s.paste, LucideIcon::ClipboardPaste, move |fm, window, cx| fm.paste_into(paste_to.clone(), window, cx))
                .shortcut("Ctrl+V")
                .disabled(!file_ops::clipboard_has_files())
                .into(),
            entry(&this, s.new_folder, LucideIcon::FolderPlus, |fm, window, cx| fm.new_folder(&NewFolder, window, cx))
                .shortcut("Ctrl+Shift+N")
                .into(),
            MenuItem::Separator,
            MenuItem::Slot(SHELL),
            MenuItem::Separator,
        ];
        items.extend(open_elsewhere(&this, s, &folder));
        items.extend(self.favorite_toggle(&folder, s.add_folder_favorite, cx));
        items.extend(tail(&this, s, vec![folder], Vec::new(), at));
        items
    }

    /// The menu of a favorite or a drive.
    fn sidebar_menu(&self, path: &Path, favorite: bool, at: shell::ScreenPoint, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let s = i18n::t(cx);
        let this = cx.entity().downgrade();
        let opened = path.to_path_buf();
        let mut items = vec![entry(&this, s.open, LucideIcon::FolderOpen, move |fm, window, cx| {
            fm.navigate(opened.clone(), true, window, cx)
        })
        .into()];
        items.extend(open_elsewhere(&this, s, path));
        items.extend([MenuItem::Separator, MenuItem::Slot(SHELL), MenuItem::Separator]);
        if favorite {
            let removed = path.to_path_buf();
            items.push(
                entry(&this, s.remove_favorite, LucideIcon::StarOff, move |fm, _, cx| fm.remove_favorite(&removed, cx)).into(),
            );
        } else {
            items.extend(self.favorite_toggle(path, s.add_favorite, cx));
        }
        items.extend(tail(&this, s, vec![path.to_path_buf()], vec![path.to_path_buf()], at));
        items
    }

    /// The Recycle Bin's menus: its sidebar entry, its items, its background.
    fn bin_menu(&self, target: &MenuTarget, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let s = i18n::t(cx);
        let this = cx.entity().downgrade();
        let full = crate::recycle::looks_full();
        let empty = entry(&this, s.empty_bin, LucideIcon::Trash, |fm, window, cx| fm.empty_bin(window, cx)).disabled(!full);
        match target {
            MenuTarget::Sidebar { path, .. } => {
                let opened = path.clone();
                let mut items = vec![entry(&this, s.open, LucideIcon::FolderOpen, move |fm, window, cx| {
                    fm.navigate(opened.clone(), true, window, cx)
                })
                .into()];
                items.extend(open_elsewhere(&this, s, path));
                items.extend([MenuItem::Separator, empty.into()]);
                items
            }
            MenuTarget::Items(_) => vec![
                entry(&this, s.restore, LucideIcon::ArchiveRestore, |fm, window, cx| fm.restore_selected(window, cx)).into(),
                entry(&this, s.delete_permanently, LucideIcon::Trash, |fm, window, cx| fm.purge_selected(window, cx))
                    .shortcut(s.key_delete)
                    .into(),
            ],
            MenuTarget::Background => vec![
                self.view_submenu(&this, cx).into(),
                self.sort_submenu(&this, cx).into(),
                entry(&this, s.refresh, LucideIcon::RefreshCw, |fm, window, cx| fm.refresh(&Refresh, window, cx))
                    .shortcut("F5")
                    .into(),
                MenuItem::Separator,
                entry(&this, s.restore_all, LucideIcon::ArchiveRestore, |fm, window, cx| fm.restore_all(window, cx))
                    .disabled(!full)
                    .into(),
                empty.into(),
            ],
        }
    }

    /// "Add to favorites", unless the folder is one already.
    fn favorite_toggle(&self, path: &Path, label: &'static str, cx: &mut Context<Self>) -> Option<MenuItem> {
        if self.favorites(cx).iter().any(|f| f == path) {
            return None;
        }
        let this = cx.entity().downgrade();
        let added = path.to_path_buf();
        Some(entry(&this, label, LucideIcon::Star, move |fm, _, cx| fm.add_favorite(added.clone(), cx)).into())
    }

    fn view_submenu(&self, this: &WeakEntity<FileManager>, cx: &App) -> MenuEntry {
        let s = i18n::t(cx);
        let current = self.view();
        let modes = ViewMode::ALL
            .iter()
            .map(|&mode| {
                MenuEntry::new(s.view_mode(mode))
                    .checked(mode == current)
                    .on_click(on_fm(this, move |fm, window, cx| fm.set_view_mode(mode, window, cx)))
                    .into()
            })
            .collect();
        MenuEntry::new(s.view).icon(LucideIcon::LayoutGrid).submenu(modes)
    }

    fn sort_submenu(&self, this: &WeakEntity<FileManager>, cx: &App) -> MenuEntry {
        let s = i18n::t(cx);
        let sort = self.tab().sort;
        let bin = self.in_bin();
        let keys = [
            (SortKey::Name, s.col_name),
            (SortKey::Type, if bin { s.col_origin } else { s.col_type }),
            (SortKey::Modified, if bin { s.col_deleted } else { s.col_modified }),
            (SortKey::Size, s.col_size),
        ];
        let mut items: Vec<MenuItem> = keys
            .into_iter()
            .map(|(key, label)| {
                MenuEntry::new(label)
                    .checked(sort.key == key)
                    .on_click(on_fm(this, move |fm, _, cx| fm.set_sort(Sort { key, descending: sort.descending }, cx)))
                    .into()
            })
            .collect();
        items.push(MenuItem::Separator);
        for (descending, label) in [(false, s.ascending), (true, s.descending)] {
            items.push(
                MenuEntry::new(label)
                    .checked(sort.descending == descending)
                    .on_click(on_fm(this, move |fm, _, cx| fm.set_sort(Sort { key: sort.key, descending }, cx)))
                    .into(),
            );
        }
        MenuEntry::new(s.sort_by).icon(LucideIcon::ArrowUpDown).submenu(items)
    }

    /// Explorer's own menu. It runs a nested Win32 modal loop; running it inside an
    /// event handler would re-enter GPUI while `App` is borrowed, so it is deferred to a
    /// foreground task where no GPUI borrow is held. The folder is listed again
    /// afterwards (the command may have renamed, deleted or created files).
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

/// "Open in new tab" / "Open in new window" for a folder.
fn open_elsewhere(this: &WeakEntity<FileManager>, s: &'static i18n::Strings, folder: &Path) -> [MenuItem; 2] {
    let (tab_path, window_path) = (folder.to_path_buf(), folder.to_path_buf());
    [
        entry(this, s.open_new_tab, LucideIcon::SquarePlus, move |fm, window, cx| fm.open_tab(tab_path.clone(), window, cx))
            .into(),
        MenuEntry::new(s.open_new_window)
            .icon(LucideIcon::AppWindow)
            .on_click(move |_, cx| {
                let tab = Tab::new(window_path.clone(), cx);
                cx.defer(move |cx| open_window(vec![tab], None, cx));
            })
            .into(),
    ]
}

/// The last entries of the menus: copy the path, properties, Explorer's own menu.
fn tail(
    this: &WeakEntity<FileManager>,
    s: &'static i18n::Strings,
    paths: Vec<PathBuf>,
    native: Vec<PathBuf>,
    at: shell::ScreenPoint,
) -> Vec<MenuItem> {
    let (copied, props) = (paths.clone(), paths);
    vec![
        MenuEntry::new(s.copy_path)
            .icon(LucideIcon::Copy)
            .on_click(move |_, cx| {
                let text = copied.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\r\n");
                cx.write_to_clipboard(ClipboardItem::new_string(text))
            })
            .into(),
        MenuEntry::new(s.properties)
            .icon(LucideIcon::Info)
            .shortcut("Alt+Enter")
            .on_click(move |_, _| show_properties(&props))
            .into(),
        MenuItem::Separator,
        entry(this, s.more_options, LucideIcon::Ellipsis, move |fm, window, cx| {
            fm.open_native_menu(native.clone(), at, window, cx)
        })
        .into(),
    ]
}

/// Explorer's Properties of one item, or of several at once.
pub(super) fn show_properties(paths: &[PathBuf]) {
    match paths {
        [] => {}
        [one] => shell::show_properties(one),
        many => file_ops::show_properties_of(many),
    }
}

/// Explorer's entries for the app's menu: "Open with" apart (it goes after Open), then
/// the rest, without what the app's menu has in its own way.
fn shell_entries(
    items: Vec<ShellItem>,
    shell: &Rc<ShellMenu>,
    at: shell::ScreenPoint,
    cx: &mut App,
) -> (Vec<MenuItem>, Vec<MenuItem>) {
    let (mut open_with, mut rest) = (Vec::new(), Vec::new());
    for item in items {
        if item.verb.as_deref().is_some_and(shell_menu::is_own_verb) {
            continue;
        }
        let is_open_with =
            item.children.iter().any(|c| c.verb.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("openas")));
        let entry = shell_entry(item, shell, at, cx);
        if is_open_with { open_with.push(entry) } else { rest.push(entry) }
    }
    (open_with, rest)
}

fn shell_entry(item: ShellItem, shell: &Rc<ShellMenu>, at: shell::ScreenPoint, cx: &mut App) -> MenuItem {
    if item.separator {
        return MenuItem::Separator;
    }
    let mut entry = MenuEntry::new(item.label).disabled(item.disabled).checked(item.checked).tag(item.key);
    if let Some(icon) = item.icon {
        entry = entry.image(menu_image(icon, cx));
    }
    if item.children.is_empty() {
        let (shell, id) = (shell.clone(), item.id);
        entry.on_click(move |_, _| shell.invoke(id, at)).into()
    } else {
        entry.submenu(item.children.into_iter().map(|child| shell_entry(child, shell, at, cx)).collect()).into()
    }
}

/// Icons of Explorer's entries, by content: the same few come back in every menu, so
/// they are made once (and freed from the GPU when too many piled up).
#[derive(Default)]
struct MenuImages(HashMap<u64, Arc<RenderImage>>);

impl Global for MenuImages {}

fn menu_image(bitmap: Bitmap, cx: &mut App) -> Arc<RenderImage> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (bitmap.width, bitmap.height, &bitmap.bgra).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(image) = cx.default_global::<MenuImages>().0.get(&key) {
        return image.clone();
    }
    if cx.global::<MenuImages>().0.len() >= 512 {
        let old: Vec<_> = cx.global_mut::<MenuImages>().0.drain().map(|(_, image)| image).collect();
        for image in old {
            cx.drop_image(image, None);
        }
    }
    let buffer = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.bgra).expect("bitmap size");
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    cx.global_mut::<MenuImages>().0.insert(key, image.clone());
    image
}

/// An entry of gpui-component's menus (the title bar's), running on the `FileManager`
/// behind `this`.
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
