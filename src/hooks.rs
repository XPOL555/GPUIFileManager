//! Win32 hooks on the app's windows, and the channel that brings what they receive to
//! the views:
//! - a window property marks every window of the app, so a tab dragged out of one
//!   instance can find the windows of another one;
//! - a subclass receives the tabs other instances send (`WM_COPYDATA`) and the shell's
//!   change notifications for the listed folder (never recursive);
//! - the drop target (`dnd`) reports drops and what a drag hovers;
//! - file drags out of a window start from the subclass (`start_file_drag`).
//!
//! Hooks run inside the window procedure or Windows' drag loop, outside any GPUI
//! update, so they only queue `HookEvent`s. The task `app::init` starts hands them to
//! the view of their window.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use serde::{Deserialize, Serialize};

use crate::app::Tab;
use crate::dnd::{DropKind, SpotKey, Target};
use crate::shell::ScreenPoint;

pub enum HookEvent {
    /// A tab dragged by another instance hovers this window.
    TabHover { title: String, cursor: ScreenPoint },
    /// It left, or the drag ended elsewhere.
    TabLeave,
    /// It was dropped here.
    TabDrop { tab: Tab, cursor: ScreenPoint },
    /// Something changed in the watched folder.
    FolderChanged,
    /// A file drag hovers another zone (or none).
    DropHover(Option<SpotKey>),
    /// Files were dropped on a zone.
    DropFiles { paths: Vec<PathBuf>, target: Target, kind: DropKind },
    /// A drag holding folders came over the window, or left it.
    DraggingFolders(bool),
    /// Something changed in the Recycle Bin.
    BinChanged,
    /// A file drag started by this window is over; `dropped` if it went somewhere.
    FileDragEnded { dropped: bool },
}

/// What one instance tells another about a tab it drags.
#[derive(Serialize, Deserialize)]
struct TabMessage {
    title: String,
    cursor: ScreenPoint,
    #[serde(default)]
    tab: Option<Tab>,
}

#[derive(Clone, Copy)]
pub enum TabSignal {
    Hover,
    Leave,
    Drop,
}

thread_local! {
    static EVENTS: RefCell<Option<UnboundedSender<(isize, HookEvent)>>> = const { RefCell::new(None) };
}

/// Creates the channel. Call once, on the UI thread, before any window opens.
pub fn init() -> UnboundedReceiver<(isize, HookEvent)> {
    let (tx, rx) = unbounded();
    EVENTS.set(Some(tx));
    rx
}

/// Queues an event for the view of window `hwnd`.
pub fn send(hwnd: isize, event: HookEvent) {
    EVENTS.with_borrow(|events| {
        if let Some(events) = events {
            let _ = events.unbounded_send((hwnd, event));
        }
    });
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::cell::Cell;
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Shell::{
        DefSubclassProc, ILFree, RemoveWindowSubclass, SHCNE_ATTRIBUTES, SHCNE_CREATE, SHCNE_DELETE, SHCNE_MKDIR,
        SHCNE_RENAMEFOLDER, SHCNE_RENAMEITEM, SHCNE_RMDIR, SHCNE_UPDATEDIR, SHCNE_UPDATEITEM, SHCNRF_InterruptLevel,
        SHCNRF_NewDelivery, SHCNRF_ShellLevel, SHChangeNotification_Lock, SHChangeNotification_Unlock,
        SHChangeNotifyDeregister, SHChangeNotifyEntry, SHChangeNotifyRegister, SHParseDisplayName, SetWindowSubclass,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetPropW, GetWindowThreadProcessId, PostMessageW, RegisterWindowMessageW, RemovePropW, SMTO_ABORTIFHUNG,
        SMTO_BLOCK, SendMessageTimeoutW, SetPropW, WM_COPYDATA, WM_NCDESTROY,
    };
    use windows::core::{PCWSTR, w};

    /// Marks the app's windows; the value is the version of the tab messages.
    const PROPERTY: PCWSTR = w!("FileManager.Window");
    const PROTOCOL: usize = 1;
    const SUBCLASS_ID: usize = 0xF11F;
    /// `COPYDATASTRUCT::dwData` of the tab messages.
    const TAB_HOVER: usize = 0x464D_0001;
    const TAB_LEAVE: usize = 0x464D_0002;
    const TAB_DROP: usize = 0x464D_0003;

    thread_local! {
        static FOLDER_CHANGED: Cell<u32> = const { Cell::new(0) };
        static BIN_CHANGED: Cell<u32> = const { Cell::new(0) };
        static START_DRAG: Cell<u32> = const { Cell::new(0) };
        /// The files `start_file_drag` was given, until its message arrives.
        static PENDING_DRAG: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
    }

    /// The message the shell sends when the watched folder changes.
    fn folder_changed_message() -> u32 {
        FOLDER_CHANGED.with(|m| {
            if m.get() == 0 {
                m.set(unsafe { RegisterWindowMessageW(w!("FileManager.FolderChanged")) });
            }
            m.get()
        })
    }

    /// The message the shell sends when a Recycle Bin folder changes.
    fn bin_changed_message() -> u32 {
        BIN_CHANGED.with(|m| {
            if m.get() == 0 {
                m.set(unsafe { RegisterWindowMessageW(w!("FileManager.BinChanged")) });
            }
            m.get()
        })
    }

    fn start_drag_message() -> u32 {
        START_DRAG.with(|m| {
            if m.get() == 0 {
                m.set(unsafe { RegisterWindowMessageW(w!("FileManager.StartDrag")) });
            }
            m.get()
        })
    }

    /// Drags `paths` out of window `hwnd` (to Explorer, other windows…) once the current
    /// input is handled. Windows' drag loop runs from the window procedure, not from a
    /// GPUI task: while a task runs, no other task can, and the drop targets of the
    /// app's own windows need theirs to show what the drag hovers.
    pub fn start_file_drag(hwnd: isize, paths: Vec<PathBuf>) {
        PENDING_DRAG.set(Some(paths));
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), start_drag_message(), WPARAM(0), LPARAM(0));
        }
    }

    /// Marks and subclasses a new window of the app.
    pub fn install(hwnd: isize) {
        let h = HWND(hwnd as _);
        unsafe {
            let _ = SetPropW(h, PROPERTY, Some(HANDLE(PROTOCOL as _)));
            let _ = SetWindowSubclass(h, Some(subclass), SUBCLASS_ID, 0);
        }
    }

    /// A window of another instance of the app that takes tabs.
    pub fn is_foreign_app_window(hwnd: isize) -> bool {
        if hwnd == 0 {
            return false;
        }
        let h = HWND(hwnd as _);
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(h, Some(&mut pid));
            pid != GetCurrentProcessId() && GetPropW(h, PROPERTY).0 as usize == PROTOCOL
        }
    }

    /// Tells a window of another instance about a tab dragged over it, or dropped on it
    /// (`tab` given). Returns whether it answered, and for a drop whether it took the tab.
    pub fn send_tab(target: isize, signal: TabSignal, title: &str, cursor: ScreenPoint, tab: Option<&Tab>) -> bool {
        let message = TabMessage { title: title.into(), cursor, tab: tab.cloned() };
        let Ok(json) = serde_json::to_vec(&message) else { return false };
        let (kind, timeout) = match signal {
            TabSignal::Hover => (TAB_HOVER, 150),
            TabSignal::Leave => (TAB_LEAVE, 150),
            TabSignal::Drop => (TAB_DROP, 2000),
        };
        let data = COPYDATASTRUCT { dwData: kind, cbData: json.len() as u32, lpData: json.as_ptr() as *mut _ };
        let mut result = 0usize;
        // SMTO_BLOCK: nothing else may run on this thread (inside a GPUI update) meanwhile.
        let sent = unsafe {
            SendMessageTimeoutW(
                HWND(target as _),
                WM_COPYDATA,
                WPARAM(0),
                LPARAM(&data as *const COPYDATASTRUCT as isize),
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                timeout,
                Some(&mut result),
            )
        };
        sent.0 != 0 && result == 1
    }

    /// A tab message from another instance; `None` if it isn't one.
    fn receive_tab(hwnd: isize, data: &COPYDATASTRUCT) -> Option<isize> {
        if !matches!(data.dwData, TAB_HOVER | TAB_LEAVE | TAB_DROP) || data.lpData.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
        let Ok(message) = serde_json::from_slice::<TabMessage>(bytes) else { return Some(0) };
        let event = match (data.dwData, message.tab) {
            (TAB_HOVER, _) => HookEvent::TabHover { title: message.title, cursor: message.cursor },
            (TAB_LEAVE, _) => HookEvent::TabLeave,
            (_, Some(tab)) => HookEvent::TabDrop { tab, cursor: message.cursor },
            (_, None) => return Some(0),
        };
        send(hwnd, event);
        Some(1)
    }

    unsafe extern "system" fn subclass(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match msg {
            WM_COPYDATA if lparam.0 != 0 => {
                let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
                if let Some(result) = receive_tab(hwnd.0 as isize, data) {
                    return LRESULT(result);
                }
            }
            WM_NCDESTROY => unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
                let _ = RemovePropW(hwnd, PROPERTY);
            },
            _ if msg == start_drag_message() => {
                if let Some(paths) = PENDING_DRAG.take() {
                    let dropped = crate::dnd::drag_files(&paths, hwnd.0 as isize);
                    send(hwnd.0 as isize, HookEvent::FileDragEnded { dropped });
                }
                return LRESULT(0);
            }
            _ if msg == folder_changed_message() || msg == bin_changed_message() => {
                // With SHCNRF_NewDelivery the notification lives in shared memory until unlocked.
                unsafe {
                    let lock = SHChangeNotification_Lock(HANDLE(wparam.0 as _), lparam.0 as u32, None, None);
                    if !lock.is_invalid() {
                        let _ = SHChangeNotification_Unlock(lock);
                    }
                }
                let event = if msg == bin_changed_message() { HookEvent::BinChanged } else { HookEvent::FolderChanged };
                send(hwnd.0 as isize, event);
                return LRESULT(0);
            }
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    /// Change notifications for some folders (not their subfolders) while this lives.
    pub struct Watch(u32);

    impl Drop for Watch {
        fn drop(&mut self) {
            unsafe {
                let _ = SHChangeNotifyDeregister(self.0);
            }
        }
    }

    /// Watches `folder` for window `hwnd`: its hook sends `FolderChanged`.
    pub fn watch(hwnd: isize, folder: &Path) -> Option<Watch> {
        register(hwnd, std::slice::from_ref(&folder.to_path_buf()), folder_changed_message())
    }

    /// Watches the Recycle Bin's folders for window `hwnd`: its hook sends `BinChanged`.
    pub fn watch_bin(hwnd: isize) -> Option<Watch> {
        register(hwnd, &crate::recycle::bins(), bin_changed_message())
    }

    fn register(hwnd: isize, folders: &[PathBuf], message: u32) -> Option<Watch> {
        unsafe {
            let mut entries = Vec::new();
            for folder in folders {
                let path: Vec<u16> = folder.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
                let mut pidl = std::ptr::null_mut();
                if SHParseDisplayName(PCWSTR(path.as_ptr()), None, &mut pidl, 0, None).is_ok() {
                    entries.push(SHChangeNotifyEntry { pidl, fRecursive: false.into() });
                }
            }
            if entries.is_empty() {
                return None;
            }
            let events = SHCNE_CREATE
                | SHCNE_DELETE
                | SHCNE_MKDIR
                | SHCNE_RMDIR
                | SHCNE_RENAMEITEM
                | SHCNE_RENAMEFOLDER
                | SHCNE_UPDATEITEM
                | SHCNE_UPDATEDIR
                | SHCNE_ATTRIBUTES;
            let id = SHChangeNotifyRegister(
                HWND(hwnd as _),
                SHCNRF_InterruptLevel | SHCNRF_ShellLevel | SHCNRF_NewDelivery,
                events.0 as i32,
                message,
                entries.len() as i32,
                entries.as_ptr(),
            );
            // The shell keeps copies of the items.
            for entry in entries {
                ILFree(Some(entry.pidl));
            }
            (id != 0).then_some(Watch(id))
        }
    }
}

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub fn install(_hwnd: isize) {}
    pub fn start_file_drag(_hwnd: isize, _paths: Vec<PathBuf>) {}
    pub fn is_foreign_app_window(_hwnd: isize) -> bool {
        false
    }
    pub fn send_tab(_target: isize, _signal: TabSignal, _title: &str, _cursor: ScreenPoint, _tab: Option<&Tab>) -> bool {
        false
    }
    pub struct Watch;
    pub fn watch(_hwnd: isize, _folder: &Path) -> Option<Watch> {
        None
    }
    pub fn watch_bin(_hwnd: isize) -> Option<Watch> {
        None
    }
}
#[cfg(not(windows))]
pub use fallback::*;
