//! Windows shell integration: drives, known folders, ShellExecute and the
//! native Explorer context menu (IContextMenu).
//!
//! Everything here runs on the UI thread (COM STA). `show_context_menu` runs a
//! nested Win32 modal loop, so callers must NOT hold a GPUI borrow while
//! calling it — see `FileManager::open_context_menu`.

use std::path::{Path, PathBuf};

pub struct Drive {
    pub root: PathBuf,
    pub label: String,
    pub total: u64,
    pub free: u64,
}

/// Sidebar places; the display name comes from `i18n::Strings::known_folder`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnownFolder {
    Home,
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
}

impl KnownFolder {
    pub const COUNT: usize = 7;
}

/// A point in physical screen pixels, as Win32 reports the cursor.
pub type ScreenPoint = (i32, i32);

/// Native handle of a window, e.g. a GPUI `Window`.
pub fn hwnd(window: &impl raw_window_handle::HasWindowHandle) -> Option<isize> {
    match window.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::cell::RefCell;
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    };
    use windows::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetActiveWindow, GetKeyState, VK_SHIFT};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        BHID_SFObject, BHID_SFUIObject, CMF_EXPLORE, CMF_EXTENDEDVERBS, CMF_NORMAL,
        CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX,
        DefSubclassProc, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads,
        FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Videos, IContextMenu,
        IContextMenu2, IContextMenu3, IShellFolder, IShellItem, IShellItemArray, ILFree,
        KF_FLAG_DEFAULT, RemoveWindowSubclass, SHCreateItemFromParsingName,
        SHCreateShellItemArrayFromIDLists, SHGetKnownFolderPath, SHParseDisplayName,
        SetWindowSubclass, ShellExecuteW,
    };
    use windows::Win32::UI::Shell::{SHOP_FILEPATH, SHObjectProperties};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreatePopupMenu, DestroyMenu, GA_ROOT, GetAncestor, GetCursorPos, GetWindowRect, SW_SHOWNORMAL,
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
        WM_MENUSELECT, WindowFromPoint,
    };
    use windows::core::{GUID, Interface, PCSTR, PCWSTR, w};

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn init_com() {
        // GPUI normally already did OleInitialize on this thread; this is a no-op then.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    }

    pub fn drives() -> Vec<Drive> {
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| {
                let root = format!("{}:\\", (b'A' + i) as char);
                let w = wide(root.as_ref());
                let mut label = [0u16; 261];
                let (mut free, mut total) = (0u64, 0u64);
                unsafe {
                    // DRIVE_REMOTE = 4: skip slow volume queries on network drives.
                    if GetDriveTypeW(PCWSTR(w.as_ptr())) != 4 {
                        let _ = GetVolumeInformationW(
                            PCWSTR(w.as_ptr()),
                            Some(&mut label),
                            None,
                            None,
                            None,
                            None,
                        );
                        let _ = GetDiskFreeSpaceExW(
                            PCWSTR(w.as_ptr()),
                            None,
                            Some(&mut total),
                            Some(&mut free),
                        );
                    }
                }
                let len = label.iter().position(|&c| c == 0).unwrap_or(0);
                Drive {
                    root: root.into(),
                    label: String::from_utf16_lossy(&label[..len]),
                    total,
                    free,
                }
            })
            .collect()
    }

    pub fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
        let ids: [(KnownFolder, GUID); KnownFolder::COUNT] = [
            (KnownFolder::Home, FOLDERID_Profile),
            (KnownFolder::Desktop, FOLDERID_Desktop),
            (KnownFolder::Downloads, FOLDERID_Downloads),
            (KnownFolder::Documents, FOLDERID_Documents),
            (KnownFolder::Pictures, FOLDERID_Pictures),
            (KnownFolder::Music, FOLDERID_Music),
            (KnownFolder::Videos, FOLDERID_Videos),
        ];
        ids.into_iter()
            .filter_map(|(name, id)| unsafe {
                let p = SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None).ok()?;
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as _));
                Some((name, PathBuf::from(s?)))
            })
            .collect()
    }

    /// Opens a file with its default handler (like double-click in Explorer).
    pub fn open(path: &Path) {
        let w = wide(path.as_os_str());
        unsafe {
            ShellExecuteW(None, w!("open"), PCWSTR(w.as_ptr()), None, None, SW_SHOWNORMAL);
        }
    }

    /// Explorer's Properties sheet. It runs on its own thread, so this returns immediately.
    pub fn show_properties(path: &Path) {
        let w = wide(path.as_os_str());
        unsafe {
            let _ = SHObjectProperties(None, SHOP_FILEPATH, PCWSTR(w.as_ptr()), PCWSTR::null());
        }
    }

    pub fn cursor_pos() -> ScreenPoint {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        (pt.x, pt.y)
    }

    /// Top-level window under a screen point, or 0.
    pub fn root_window_at((x, y): ScreenPoint) -> isize {
        unsafe {
            let hwnd = WindowFromPoint(POINT { x, y });
            if hwnd.is_invalid() { 0 } else { GetAncestor(hwnd, GA_ROOT).0 as isize }
        }
    }

    /// Moves a top-level window so its top-left corner is at `origin`, keeping its size,
    /// but inside the work area of the monitor under `anchor` (the cursor): a window
    /// straddling two monitors with different scale factors gets rescaled by Windows.
    pub fn move_window(hwnd: isize, (mut x, mut y): ScreenPoint, (ax, ay): ScreenPoint) {
        let hwnd = HWND(hwnd as _);
        unsafe {
            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);
            let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
            let monitor = MonitorFromPoint(POINT { x: ax, y: ay }, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let work = info.rcWork;
                x = x.clamp(work.left, (work.right - w).max(work.left));
                y = y.clamp(work.top, (work.bottom - h).max(work.top));
            }
            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// Moves a window without clamping or activating it (the tab drag preview).
    pub fn set_window_origin(hwnd: isize, (x, y): ScreenPoint) {
        unsafe {
            let _ = SetWindowPos(HWND(hwnd as _), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// A disabled window takes no input and `WindowFromPoint` looks through it.
    pub fn disable_window(hwnd: isize) {
        unsafe {
            let _ = EnableWindow(HWND(hwnd as _), false);
        }
    }

    thread_local! {
        /// The menu currently shown, so the subclass proc can forward owner-draw
        /// and submenu messages ("Send to", "Open with" are populated lazily).
        static ACTIVE_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
    }

    const SUBCLASS_ID: usize = 0xF11E;
    const CMIC_MASK_UNICODE: u32 = 0x0000_4000;
    const CMIC_MASK_PTINVOKE: u32 = 0x2000_0000;

    unsafe extern "system" fn menu_subclass(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR | WM_MENUSELECT) {
            let handled = ACTIVE_MENU.with_borrow(|m| {
                let m = m.as_ref()?;
                if let Ok(m3) = m.cast::<IContextMenu3>() {
                    let mut result = LRESULT(0);
                    unsafe { m3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)) }.ok()?;
                    Some(result)
                } else if let Ok(m2) = m.cast::<IContextMenu2>() {
                    unsafe { m2.HandleMenuMsg(msg, wparam, lparam) }.ok()?;
                    Some(LRESULT(0))
                } else {
                    None
                }
            });
            if let Some(r) = handled {
                return r;
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    /// Context menu for `paths`, or the folder background menu of `folder`
    /// (New, Paste, …) when `paths` is empty, shown at `at`. Blocks until the menu closes.
    pub fn show_context_menu(folder: &Path, paths: &[PathBuf], at: ScreenPoint) -> windows::core::Result<()> {
        unsafe {
            let hwnd = GetActiveWindow();
            let menu: IContextMenu = if paths.is_empty() {
                let w = wide(folder.as_os_str());
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None)?;
                let sf: IShellFolder = item.BindToHandler(None, &BHID_SFObject)?;
                sf.CreateViewObject(hwnd)?
            } else {
                let mut pidls: Vec<*mut ITEMIDLIST> = Vec::with_capacity(paths.len());
                for p in paths {
                    let w = wide(p.as_os_str());
                    let mut pidl = std::ptr::null_mut();
                    if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() {
                        pidls.push(pidl);
                    }
                }
                let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as _).collect();
                let arr = SHCreateShellItemArrayFromIDLists(&consts);
                for p in pidls {
                    ILFree(Some(p));
                }
                let arr: IShellItemArray = arr?;
                arr.BindToHandler(None, &BHID_SFUIObject)?
            };

            let hmenu = CreatePopupMenu()?;
            let mut flags = CMF_NORMAL | CMF_EXPLORE;
            if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                flags |= CMF_EXTENDEDVERBS;
            }
            const FIRST: u32 = 1;
            menu.QueryContextMenu(hmenu, 0, FIRST, 0x7FFF, flags).ok()?;

            let pt = POINT { x: at.0, y: at.1 };
            ACTIVE_MENU.set(Some(menu.clone()));
            let _ = SetWindowSubclass(hwnd, Some(menu_subclass), SUBCLASS_ID, 0);
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None);
            let _ = RemoveWindowSubclass(hwnd, Some(menu_subclass), SUBCLASS_ID);
            ACTIVE_MENU.set(None);

            let result = if cmd.0 >= FIRST as i32 {
                let verb = (cmd.0 as u32 - FIRST) as usize;
                let mut info = CMINVOKECOMMANDINFOEX {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                    fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
                    hwnd,
                    lpVerb: PCSTR(verb as *const u8),
                    lpVerbW: PCWSTR(verb as *const u16),
                    nShow: SW_SHOWNORMAL.0,
                    ptInvoke: pt,
                    ..Default::default()
                };
                let w = wide(folder.as_os_str());
                info.lpDirectoryW = PCWSTR(w.as_ptr());
                menu.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO)
            } else {
                Ok(())
            };
            let _ = DestroyMenu(hmenu);
            result
        }
    }

    /// A WinHTTP handle, closed on drop.
    struct Internet(*mut core::ffi::c_void);

    impl Internet {
        fn new(handle: *mut core::ffi::c_void, step: &str) -> Result<Self, String> {
            if handle.is_null() {
                Err(format!("{step}: {}", windows::core::Error::from_thread()))
            } else {
                Ok(Self(handle))
            }
        }
    }

    impl Drop for Internet {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }

    /// HTTPS GET through WinHTTP (system proxy settings, Windows TLS, redirects
    /// followed). Blocking: call it from a background task. Returns the status code and
    /// at most `max_len` bytes of the body.
    pub fn https_get(host: &str, path: &str, headers: &str, max_len: usize) -> Result<(u16, Vec<u8>), String> {
        let failed = |step: &str, e: windows::core::Error| format!("{step}: {e}");
        let agent = wide(concat!("FileManager/", env!("CARGO_PKG_VERSION")).as_ref());
        let (host, path) = (wide(host.as_ref()), wide(path.as_ref()));
        let headers: Vec<u16> = headers.encode_utf16().collect();
        unsafe {
            let session = Internet::new(
                WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0),
                "WinHttpOpen",
            )?;
            let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 15_000);
            let connection = Internet::new(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), 443, 0), "WinHttpConnect")?;
            let request = Internet::new(
                WinHttpOpenRequest(
                    connection.0,
                    w!("GET"),
                    PCWSTR(path.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    WINHTTP_FLAG_SECURE,
                ),
                "WinHttpOpenRequest",
            )?;
            let headers = (!headers.is_empty()).then_some(headers.as_slice());
            WinHttpSendRequest(request.0, headers, None, 0, 0, 0).map_err(|e| failed("WinHttpSendRequest", e))?;
            WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|e| failed("WinHttpReceiveResponse", e))?;

            let mut status = 0u32;
            let mut len = size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status as *mut u32 as *mut _),
                &mut len,
                std::ptr::null_mut(),
            )
            .map_err(|e| failed("WinHttpQueryHeaders", e))?;

            let mut body = Vec::new();
            let mut chunk = [0u8; 8192];
            while body.len() < max_len {
                let mut read = 0u32;
                WinHttpReadData(request.0, chunk.as_mut_ptr() as *mut _, chunk.len() as u32, &mut read)
                    .map_err(|e| failed("WinHttpReadData", e))?;
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..read as usize]);
            }
            body.truncate(max_len);
            Ok((status as u16, body))
        }
    }
}

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub fn init_com() {}
    pub fn drives() -> Vec<Drive> {
        vec![Drive { root: "/".into(), label: String::new(), total: 0, free: 0 }]
    }
    pub fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
        std::env::var_os("HOME").map(|h| vec![(KnownFolder::Home, PathBuf::from(h))]).unwrap_or_default()
    }
    pub fn open(_path: &Path) {}
    pub fn show_properties(_path: &Path) {}
    pub fn cursor_pos() -> ScreenPoint {
        (0, 0)
    }
    pub fn root_window_at(_at: ScreenPoint) -> isize {
        0
    }
    pub fn move_window(_hwnd: isize, _origin: ScreenPoint, _anchor: ScreenPoint) {}
    pub fn set_window_origin(_hwnd: isize, _origin: ScreenPoint) {}
    pub fn disable_window(_hwnd: isize) {}
    pub fn show_context_menu(_folder: &Path, _paths: &[PathBuf], _at: ScreenPoint) -> Result<(), ()> {
        Ok(())
    }
    pub fn https_get(_host: &str, _path: &str, _headers: &str, _max_len: usize) -> Result<(u16, Vec<u8>), String> {
        Err("HTTPS is only implemented on Windows".into())
    }
}
#[cfg(not(windows))]
pub use fallback::*;
