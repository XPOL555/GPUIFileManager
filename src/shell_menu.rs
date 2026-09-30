//! Explorer's context menu read entry by entry, so the app's own menu can offer what
//! it holds: "Open with", what apps add (editors, terminals, archivers…), "Send to",
//! "Give access to", "New".
//!
//! Each menu lives on a thread of its own, a COM apartment that pumps messages: shell
//! extensions may take a while to load and may show windows. The thread builds the
//! menu, hands its entries over, and keeps it until the app's menu closes; the entry
//! picked is invoked there too, and the thread lingers while what it opened is shown.

use std::path::PathBuf;

use crate::shell::ScreenPoint;
use crate::shell_images::Bitmap;

/// An entry of Explorer's menu.
pub struct ShellItem {
    /// What `ShellMenu::invoke` takes; 0 for submenus and separators.
    pub id: u32,
    /// Tells entries apart, submenus included, for icons that come later.
    pub key: u32,
    pub label: String,
    /// The entry's canonical verb ("open", "properties", "openas"…), when it has one.
    pub verb: Option<String>,
    pub icon: Option<Bitmap>,
    pub disabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub children: Vec<ShellItem>,
}

/// What Explorer's menu gets after it was first read: the shell loads some icons in
/// the background, and adds the entries of packaged apps ("Open in Terminal"…) a
/// moment later.
pub enum Late {
    /// Icons of entries, by `key`.
    Icons(Vec<(u32, Bitmap)>),
    /// Entries of the menu itself, to add after the others.
    Entries(Vec<ShellItem>),
}

/// Explorer's menu for some items, kept on its thread until this is dropped.
pub struct ShellMenu {
    pub items: Vec<ShellItem>,
    /// What comes later, within a second or so.
    pub late: Option<futures::channel::mpsc::UnboundedReceiver<Late>>,
    commands: std::sync::mpsc::Sender<(u32, ScreenPoint)>,
}

impl ShellMenu {
    /// Runs entry `id` (from `items`) as if it was picked at `at`.
    pub fn invoke(&self, id: u32, at: ScreenPoint) {
        let _ = self.commands.send((id, at));
    }
}

/// Canonical verbs of entries the app's menu leaves out: it has them in its own way,
/// or they would be mistaken for its own (Explorer's Quick access favorites).
const OWN_VERBS: &[&str] = &[
    "pintohome",
    "pintohomefile",
    "unpinfromhome",
    "open",
    "opennewwindow",
    "opennewtab",
    "opennewprocess",
    "explore",
    "cut",
    "copy",
    "paste",
    "delete",
    "rename",
    "properties",
    "copyaspath",
    "newfolder",
];

/// Whether the app's menu shows an entry of Explorer's itself.
pub fn is_own_verb(verb: &str) -> bool {
    OWN_VERBS.iter().any(|own| own.eq_ignore_ascii_case(verb))
}

/// Builds Explorer's menu for `paths`, or for the background of `folder` when `paths`
/// is empty, on a thread of its own; `extended` adds what Shift + right click shows.
/// `owner` (a window) owns whatever its commands show. `None` if it can't be built.
pub fn load(
    folder: PathBuf,
    paths: Vec<PathBuf>,
    extended: bool,
    owner: isize,
) -> impl Future<Output = Option<ShellMenu>> {
    let (done, menu) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("shell-menu".into())
        .spawn(move || platform::serve(&folder, &paths, extended, owner, done));
    async move {
        if spawned.is_err() {
            return None;
        }
        menu.await.ok().flatten()
    }
}

/// Builds Explorer's menu for `folder` once and lets it go, on a thread of its own. The
/// first menu a process asks for comes without the entries of packaged apps ("Open in
/// Terminal", VS Code…): the shell only loads their list then. This one loads it, and
/// the shell extensions, before the first right click.
pub fn warm_up(folder: PathBuf) {
    let _ = std::thread::Builder::new().name("shell-menu-warm-up".into()).spawn(move || platform::warm_up(&folder));
}

/// Menu labels without their accelerator marks (`&Open` → `Open`, `&&` → `&`) or the
/// shortcut text after a tab.
fn clean_label(raw: &str) -> String {
    let raw = raw.split('\t').next().unwrap_or_default();
    let mut label = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                label.push('&');
                chars.next();
            }
            continue;
        }
        label.push(c);
    }
    label.trim().to_string()
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::collections::HashSet;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Shell::{
        CMF_EXPLORE, CMF_EXTENDEDVERBS, CMF_NORMAL, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, GCS_VERBW, IContextMenu,
        IContextMenu2, IContextMenu3,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreatePopupMenu, DestroyMenu, DispatchMessageW, EnumThreadWindows, GetMenuItemCount, GetMenuItemInfoW, HMENU,
        IsWindowVisible, MENUITEMINFOW, MFS_CHECKED, MFS_DISABLED, MFT_OWNERDRAW, MFT_SEPARATOR, MIIM_BITMAP,
        MIIM_CHECKMARKS, MIIM_DATA, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, MSG,
        MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, SW_SHOWNORMAL, TranslateMessage,
        WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM,
    };
    use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODA_DRAWENTIRE, ODT_MENU};
    use windows::core::{BOOL, Interface, PCSTR, PCWSTR, PSTR, PWSTR};

    /// `hbmpItem` of entries that draw their icon themselves.
    const HBMMENU_CALLBACK: isize = -1;
    const FIRST: u32 = 1;
    const LAST: u32 = 0x7FFF;
    const CMIC_MASK_UNICODE: u32 = 0x0000_4000;
    const CMIC_MASK_PTINVOKE: u32 = 0x2000_0000;
    const CMIC_MASK_ASYNCOK: u32 = 0x0010_0000;
    /// Submenus of submenus of submenus: deep enough for any real menu.
    const MAX_DEPTH: usize = 4;

    pub(super) fn serve(
        folder: &std::path::Path,
        paths: &[PathBuf],
        extended: bool,
        owner: isize,
        done: futures::channel::oneshot::Sender<Option<ShellMenu>>,
    ) {
        unsafe {
            // Apartment threaded, with OLE: some commands use the clipboard or drag.
            let ole = OleInitialize(None);
            let hwnd = HWND(owner as _);
            match build(folder, paths, extended, hwnd) {
                Ok((menu, hmenu, items)) => {
                    let (commands, received) = mpsc::channel();
                    let (later, late) = futures::channel::mpsc::unbounded();
                    let (mut missing, mut known) = (HashSet::new(), HashSet::new());
                    without_icons(&items, &mut missing);
                    keys(&items, &mut known);
                    let handed = done.send(Some(ShellMenu { items, late: Some(late), commands }));
                    // Until the app's menu closes (the sender goes) or an entry is picked.
                    if handed.is_ok()
                        && let Some((id, at)) = wait(&received, |tenths| {
                            // Most icons come within a few tenths of a second.
                            if !missing.is_empty() && matches!(tenths, 1 | 4) {
                                let found = late_icons(&menu, hmenu, &mut missing);
                                if !found.is_empty() {
                                    let _ = later.unbounded_send(Late::Icons(found));
                                }
                            }
                            if matches!(tenths, 3 | 8 | 15 | 30) {
                                let found = late_entries(&menu, hmenu, &mut known);
                                if !found.is_empty() {
                                    let _ = later.unbounded_send(Late::Entries(found));
                                }
                            }
                        })
                    {
                        invoke(&menu, id, at, hwnd, folder);
                        linger();
                    }
                    let _ = DestroyMenu(hmenu);
                    drop(menu);
                }
                Err(_) => {
                    let _ = done.send(None);
                }
            }
            if ole.is_ok() {
                OleUninitialize();
            }
        }
    }

    pub(super) fn warm_up(folder: &std::path::Path) {
        unsafe {
            let ole = OleInitialize(None);
            if let Ok((menu, hmenu, _)) = build(folder, &[], false, HWND::default()) {
                // The list loads in the background: keep things alive meanwhile.
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(3) {
                    let _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
                    pump();
                }
                let _ = DestroyMenu(hmenu);
                drop(menu);
            }
            if ole.is_ok() {
                OleUninitialize();
            }
        }
    }

    unsafe fn build(
        folder: &std::path::Path,
        paths: &[PathBuf],
        extended: bool,
        _hwnd: HWND,
    ) -> windows::core::Result<(IContextMenu, HMENU, Vec<ShellItem>)> {
        unsafe {
            // No owner while it is built: the shell adds some entries later through the
            // owner window, which lives on the UI thread; this thread pumps its own.
            let menu = crate::shell::context_menu_of(folder, paths, HWND::default())?;
            let hmenu = CreatePopupMenu()?;
            let mut flags = CMF_NORMAL | CMF_EXPLORE;
            if extended {
                flags |= CMF_EXTENDEDVERBS;
            }
            if let Err(err) = menu.QueryContextMenu(hmenu, 0, FIRST, LAST, flags).ok() {
                let _ = DestroyMenu(hmenu);
                return Err(err);
            }
            let items = read_menu(&menu, hmenu, 0);
            Ok((menu, hmenu, items))
        }
    }

    /// Keys of all the entries.
    fn keys(items: &[ShellItem], known: &mut HashSet<u32>) {
        for item in items {
            if !item.separator {
                known.insert(item.key);
            }
            keys(&item.children, known);
        }
    }

    /// Entries added to the menu since it was read (not in `known`, which they join).
    fn late_entries(menu: &IContextMenu, hmenu: HMENU, known: &mut HashSet<u32>) -> Vec<ShellItem> {
        let mut found = Vec::new();
        unsafe {
            for position in 0..GetMenuItemCount(Some(hmenu)).max(0) as u32 {
                let mut info = MENUITEMINFOW {
                    cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE | MIIM_ID | MIIM_SUBMENU,
                    ..Default::default()
                };
                if GetMenuItemInfoW(hmenu, position, true, &mut info).is_err()
                    || info.fType.contains(MFT_SEPARATOR)
                    || known.contains(&key_of(&info))
                {
                    continue;
                }
                if let Some(item) = read_item(menu, hmenu, position, 0) {
                    keys(std::slice::from_ref(&item), known);
                    found.push(item);
                }
            }
        }
        found
    }

    /// Keys of the entries that have no icon yet.
    fn without_icons(items: &[ShellItem], missing: &mut HashSet<u32>) {
        for item in items {
            if !item.separator && item.icon.is_none() {
                missing.insert(item.key);
            }
            without_icons(&item.children, missing);
        }
    }

    /// Key of an entry: its command id, or its submenu's handle (both unique in the menu).
    fn key_of(info: &MENUITEMINFOW) -> u32 {
        if info.hSubMenu.is_invalid() { info.wID } else { 0x8000_0000 | (info.hSubMenu.0 as usize as u32 & 0x7FFF_FFFF) }
    }

    /// The icon of an entry, whichever way it comes.
    unsafe fn icon_of(menu: &IContextMenu, hmenu: HMENU, info: &MENUITEMINFOW) -> Option<Bitmap> {
        unsafe {
            crate::shell_images::menu_bitmap(info.hbmpItem)
                .or_else(|| crate::shell_images::menu_bitmap(info.hbmpUnchecked))
                .or_else(|| (info.hbmpItem.0 as isize == HBMMENU_CALLBACK).then(|| drawn_icon(menu, hmenu, info)).flatten())
        }
    }

    /// Icons of entries in `missing` that have one by now; found ones leave `missing`.
    /// Submenus are not opened again: they were filled when read.
    fn late_icons(menu: &IContextMenu, hmenu: HMENU, missing: &mut HashSet<u32>) -> Vec<(u32, Bitmap)> {
        let mut found = Vec::new();
        let mut menus = vec![hmenu];
        while let Some(hmenu) = menus.pop() {
            unsafe {
                for position in 0..GetMenuItemCount(Some(hmenu)).max(0) as u32 {
                    let mut info = MENUITEMINFOW {
                        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_FTYPE | MIIM_ID | MIIM_SUBMENU | MIIM_BITMAP | MIIM_CHECKMARKS | MIIM_DATA,
                        ..Default::default()
                    };
                    if GetMenuItemInfoW(hmenu, position, true, &mut info).is_err() || info.fType.contains(MFT_SEPARATOR) {
                        continue;
                    }
                    if !info.hSubMenu.is_invalid() {
                        menus.push(info.hSubMenu);
                    }
                    let key = key_of(&info);
                    if missing.contains(&key)
                        && let Some(icon) = icon_of(menu, hmenu, &info)
                    {
                        missing.remove(&key);
                        found.push((key, icon));
                    }
                }
            }
        }
        found
    }

    /// The entries of `hmenu`, and of its submenus (filled in as they would be when opened).
    unsafe fn read_menu(menu: &IContextMenu, hmenu: HMENU, depth: usize) -> Vec<ShellItem> {
        unsafe {
            let count = GetMenuItemCount(Some(hmenu)).max(0) as u32;
            (0..count).filter_map(|position| read_item(menu, hmenu, position, depth)).collect()
        }
    }

    /// The entry at `position` of `hmenu`, with its submenu; `None` for what can't be
    /// shown (owner-drawn labels, empty submenus).
    unsafe fn read_item(menu: &IContextMenu, hmenu: HMENU, position: u32, depth: usize) -> Option<ShellItem> {
        unsafe {
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE
                    | MIIM_STATE
                    | MIIM_ID
                    | MIIM_SUBMENU
                    | MIIM_STRING
                    | MIIM_BITMAP
                    | MIIM_CHECKMARKS
                    | MIIM_DATA,
                ..Default::default()
            };
            GetMenuItemInfoW(hmenu, position, true, &mut info).ok()?;
            if info.fType.contains(MFT_SEPARATOR) {
                return Some(ShellItem::separator());
            }
            // Owner-drawn entries draw their label themselves: nothing to show.
            if info.fType.contains(MFT_OWNERDRAW) || info.cch == 0 {
                return None;
            }
            let mut text = vec![0u16; info.cch as usize + 1];
            info.dwTypeData = PWSTR(text.as_mut_ptr());
            info.cch += 1;
            GetMenuItemInfoW(hmenu, position, true, &mut info).ok()?;
            let len = text.iter().position(|&c| c == 0).unwrap_or(text.len());
            let label = clean_label(&String::from_utf16_lossy(&text[..len]));
            if label.is_empty() {
                return None;
            }
            let submenu = !info.hSubMenu.is_invalid();
            let children = if submenu && depth < MAX_DEPTH {
                open_submenu(menu, info.hSubMenu, position);
                read_menu(menu, info.hSubMenu, depth + 1)
            } else {
                Vec::new()
            };
            // A submenu left empty (nothing to send to, no app to open with…).
            if submenu && children.iter().all(|c| c.separator) {
                return None;
            }
            let id = if submenu { 0 } else { info.wID };
            let verb = (!submenu && (FIRST..=LAST).contains(&id)).then(|| verb_of(menu, id - FIRST)).flatten();
            let icon = icon_of(menu, hmenu, &info);
            Some(ShellItem {
                id,
                key: key_of(&info),
                label,
                verb,
                icon,
                disabled: info.fState.contains(MFS_DISABLED),
                checked: info.fState.contains(MFS_CHECKED),
                separator: false,
                children,
            })
        }
    }

    /// Lets the menu fill a submenu the way it does when the submenu opens ("Open
    /// with", "Send to" and "New" are only filled then).
    unsafe fn open_submenu(menu: &IContextMenu, submenu: HMENU, position: u32) {
        unsafe {
            let (wparam, lparam) = (WPARAM(submenu.0 as usize), LPARAM(position as isize));
            if let Ok(menu3) = menu.cast::<IContextMenu3>() {
                let _ = menu3.HandleMenuMsg2(WM_INITMENUPOPUP, wparam, lparam, None);
            } else if let Ok(menu2) = menu.cast::<IContextMenu2>() {
                let _ = menu2.HandleMenuMsg(WM_INITMENUPOPUP, wparam, lparam);
            }
        }
    }

    /// The icon of an entry that draws it itself (`HBMMENU_CALLBACK`): the menu is asked
    /// for its size and to draw it, as a real menu would.
    unsafe fn drawn_icon(menu: &IContextMenu, hmenu: HMENU, info: &MENUITEMINFOW) -> Option<Bitmap> {
        unsafe {
            let menu3 = menu.cast::<IContextMenu3>().ok()?;
            let mut measure =
                MEASUREITEMSTRUCT { CtlType: ODT_MENU, itemID: info.wID, itemData: info.dwItemData, ..Default::default() };
            menu3.HandleMenuMsg2(WM_MEASUREITEM, WPARAM(0), LPARAM(&mut measure as *mut _ as isize), None).ok()?;
            crate::shell_images::draw_to_bitmap(measure.itemWidth, measure.itemHeight, |dc, rect| {
                let mut draw = DRAWITEMSTRUCT {
                    CtlType: ODT_MENU,
                    itemID: info.wID,
                    itemAction: ODA_DRAWENTIRE,
                    hwndItem: HWND(hmenu.0),
                    hDC: dc,
                    rcItem: rect,
                    itemData: info.dwItemData,
                    ..Default::default()
                };
                let _ = menu3.HandleMenuMsg2(WM_DRAWITEM, WPARAM(0), LPARAM(&mut draw as *mut _ as isize), None);
            })
        }
    }

    unsafe fn verb_of(menu: &IContextMenu, offset: u32) -> Option<String> {
        unsafe {
            let mut buffer = [0u16; 256];
            menu.GetCommandString(offset as usize, GCS_VERBW, None, PSTR(buffer.as_mut_ptr() as *mut u8), buffer.len() as u32)
                .ok()?;
            let len = buffer.iter().position(|&c| c == 0).unwrap_or(0);
            (len > 0).then(|| String::from_utf16_lossy(&buffer[..len]))
        }
    }

    /// Pumps this thread's messages until a command comes, or the app's menu is gone.
    /// `tick` runs every tenth of a second with the tenths elapsed.
    fn wait(received: &mpsc::Receiver<(u32, ScreenPoint)>, mut tick: impl FnMut(u32)) -> Option<(u32, ScreenPoint)> {
        let start = Instant::now();
        let mut ticks = 0;
        loop {
            match received.recv_timeout(Duration::from_millis(15)) {
                Ok(command) => return Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => pump(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
            let tenths = (start.elapsed().as_millis() / 100) as u32;
            if tenths > ticks {
                ticks = tenths;
                tick(ticks);
            }
        }
    }

    fn pump() {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe fn invoke(menu: &IContextMenu, id: u32, at: ScreenPoint, hwnd: HWND, folder: &std::path::Path) {
        use std::os::windows::ffi::OsStrExt;
        if !(FIRST..=LAST).contains(&id) {
            return;
        }
        let offset = (id - FIRST) as usize;
        let directory: Vec<u16> = folder.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let info = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE | CMIC_MASK_ASYNCOK,
            hwnd,
            lpVerb: PCSTR(offset as *const u8),
            lpVerbW: PCWSTR(offset as *const u16),
            lpDirectoryW: PCWSTR(directory.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: POINT { x: at.0, y: at.1 },
            ..Default::default()
        };
        unsafe {
            if let Err(err) = menu.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO) {
                eprintln!("shell menu command: {err}");
            }
        }
    }

    /// After a command: keeps the thread (and the objects on it) while it shows windows,
    /// and a few seconds more for work it finishes in the background.
    fn linger() {
        const GRACE: Duration = Duration::from_secs(4);
        let mut quiet_since = Instant::now();
        loop {
            unsafe {
                let _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
            }
            pump();
            if has_visible_windows() {
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() > GRACE {
                return;
            }
        }
    }

    fn has_visible_windows() -> bool {
        unsafe extern "system" fn visible(hwnd: HWND, found: LPARAM) -> BOOL {
            unsafe {
                if IsWindowVisible(hwnd).as_bool() {
                    *(found.0 as *mut bool) = true;
                    return false.into();
                }
            }
            true.into()
        }
        let mut found = false;
        unsafe {
            let _ = EnumThreadWindows(GetCurrentThreadId(), Some(visible), LPARAM(&mut found as *mut bool as isize));
        }
        found
    }

    impl ShellItem {
        fn separator() -> Self {
            ShellItem {
                id: 0,
                key: 0,
                label: String::new(),
                verb: None,
                icon: None,
                disabled: false,
                checked: false,
                separator: true,
                children: Vec::new(),
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn serve(
        _folder: &std::path::Path,
        _paths: &[PathBuf],
        _extended: bool,
        _owner: isize,
        done: futures::channel::oneshot::Sender<Option<ShellMenu>>,
    ) {
        let _ = done.send(None);
    }
    pub(super) fn warm_up(_folder: &std::path::Path) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_lose_their_accelerators() {
        assert_eq!(clean_label("&Open"), "Open");
        assert_eq!(clean_label("Save && &Close\tCtrl+S"), "Save & Close");
        assert_eq!(clean_label("  Apri con &Code  "), "Apri con Code");
        assert!(is_own_verb("Properties") && !is_own_verb("openas"));
    }

    /// Explorer's menus for a file and for a folder background come out with entries,
    /// verbs and submenus, and a command can be dropped unused.
    #[cfg(windows)]
    #[test]
    fn explorer_menus_can_be_read() {
        let root = std::env::temp_dir().join(format!("fm-menu-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("note.txt");
        std::fs::write(&file, "x").unwrap();

        let block = |future| futures::executor::block_on(future);
        let mut menu = block(load(root.clone(), vec![file.clone()], false, 0)).expect("a file's menu");
        // What is still loading comes in a moment later.
        std::thread::sleep(std::time::Duration::from_millis(1700));
        let mut late = menu.late.take().unwrap();
        while let Ok(late) = late.try_recv() {
            match late {
                Late::Icons(icons) => {
                    for (key, icon) in icons {
                        set_icon(&mut menu.items, key, &icon);
                    }
                }
                Late::Entries(entries) => {
                    if std::env::var_os("FM_DUMP_MENU").is_some() {
                        println!("-- came later:");
                        dump(&entries, 0);
                    }
                    menu.items.extend(entries);
                }
            }
        }
        let verbs: Vec<&str> = menu.items.iter().filter_map(|i| i.verb.as_deref()).collect();
        assert!(verbs.iter().any(|v| v.eq_ignore_ascii_case("open")), "verbs: {verbs:?}");
        assert!(menu.items.iter().any(|i| !i.separator && !i.label.is_empty()));
        if std::env::var_os("FM_DUMP_MENU").is_some() {
            dump(&menu.items, 0);
        }
        drop(menu);

        let mut background = block(load(root.clone(), Vec::new(), false, 0)).expect("a folder background's menu");
        assert!(background.items.iter().any(|i| !i.children.is_empty()), "the New submenu");
        if std::env::var_os("FM_DUMP_MENU").is_some() {
            dump(&background.items, 0);
            std::thread::sleep(std::time::Duration::from_millis(1700));
            let mut late = background.late.take().unwrap();
            while let Ok(late) = late.try_recv() {
                if let Late::Entries(entries) = late {
                    println!("-- came later:");
                    dump(&entries, 0);
                }
            }
        }
        drop(background);
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn set_icon(items: &mut [ShellItem], key: u32, icon: &Bitmap) -> bool {
        for item in items {
            if item.key == key {
                item.icon = Some(Bitmap { width: icon.width, height: icon.height, bgra: icon.bgra.clone() });
                return true;
            }
            if set_icon(&mut item.children, key, icon) {
                return true;
            }
        }
        false
    }

    fn dump(items: &[ShellItem], depth: usize) {
        for item in items {
            let icon = item.icon.as_ref().map(|b| format!("{}x{}", b.width, b.height)).unwrap_or_default();
            if item.separator {
                println!("{:indent$}----", "", indent = depth * 2);
            } else {
                println!("{:indent$}{} [{:?}] {icon}", "", item.label, item.verb, indent = depth * 2);
            }
            dump(&item.children, depth + 1);
        }
    }
}
