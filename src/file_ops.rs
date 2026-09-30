//! Copying, moving, deleting and renaming files through the shell (`IFileOperation`),
//! and files on the clipboard, in the formats Explorer reads and writes.
//!
//! An operation runs on a thread of its own: the shell shows its progress, conflict
//! and confirmation dialogs there, owned by the app's window, while the app keeps
//! drawing. Nothing is ever deleted outside the shell, so deletes can go to the
//! Recycle Bin and Explorer's own confirmations apply.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Files the app cut to the clipboard: drawn faded until they are pasted, or until the
/// clipboard's `sequence` number moves on (something else was copied).
pub struct CutFiles {
    pub sequence: u32,
    pub paths: HashSet<PathBuf>,
}

impl gpui_kit::Global for CutFiles {}

/// A file operation, run by `run`.
#[derive(Clone, Debug)]
pub enum FileOp {
    Copy { paths: Vec<PathBuf>, dest: PathBuf },
    Move { paths: Vec<PathBuf>, dest: PathBuf },
    /// To the Recycle Bin, or gone for good.
    Delete { paths: Vec<PathBuf>, recycle: bool },
    Rename { path: PathBuf, name: String },
    /// Items of the Recycle Bin (its `$R…` files) back where they were deleted from.
    Restore { items: Vec<(PathBuf, PathBuf)> },
    /// Items of the Recycle Bin gone for good (the app asked already).
    Purge { paths: Vec<PathBuf> },
    /// Everything in the Recycle Bin, after the shell's confirmation.
    EmptyBin,
}

impl FileOp {
    /// Where the items end up, when they keep their names.
    pub fn results(&self) -> Vec<PathBuf> {
        match self {
            FileOp::Copy { paths, dest } | FileOp::Move { paths, dest } => {
                paths.iter().filter_map(|p| p.file_name()).map(|n| dest.join(n)).collect()
            }
            FileOp::Rename { path, name } => path.parent().map(|p| vec![p.join(name)]).unwrap_or_default(),
            FileOp::Restore { items } => items.iter().map(|(_, original)| original.clone()).collect(),
            FileOp::Delete { .. } | FileOp::Purge { .. } | FileOp::EmptyBin => Vec::new(),
        }
    }

    /// Whether the Recycle Bin may look different afterwards.
    pub fn touches_bin(&self) -> bool {
        matches!(
            self,
            FileOp::Delete { recycle: true, .. } | FileOp::Restore { .. } | FileOp::Purge { .. } | FileOp::EmptyBin
        )
    }
}

/// Characters Windows never allows in a file name.
const INVALID: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Whether `name` can be a file name: not empty, no reserved characters, not only dots,
/// no trailing space or dot (Windows would silently drop them).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|c| INVALID.contains(&c) || c.is_control())
        && !name.trim_matches('.').is_empty()
        && !name.ends_with([' ', '.'])
}

/// `base`, or `base (2)`, `base (3)`… : the first name not taken in `dir`.
pub fn unique_name(dir: &Path, base: &str) -> String {
    let taken = |name: &str| dir.join(name).symlink_metadata().is_ok();
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} ({n})")).find(|name| !taken(name)).unwrap_or_else(|| base.to_string())
}

/// The part of a file name Explorer selects for renaming: the name without its
/// extension (the whole name for folders and names starting with a dot).
pub fn stem_len(name: &str, is_dir: bool) -> usize {
    match name.rfind('.') {
        Some(dot) if !is_dir && dot > 0 => dot,
        _ => name.len(),
    }
}

/// Operations still running: the app must not quit under them (see `main`).
static RUNNING: AtomicUsize = AtomicUsize::new(0);

pub fn running() -> usize {
    RUNNING.load(Ordering::SeqCst)
}

/// Counts an operation as running while it lives, even if it panics.
struct Running;

impl Running {
    fn start() -> Self {
        RUNNING.fetch_add(1, Ordering::SeqCst);
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Runs `op` on its own thread; the future resolves when it is done. `Err` carries the
/// system's message for the user (empty if there is none); a cancel is not an error.
/// `owner` (a window handle) owns the shell's dialogs.
pub fn run(op: FileOp, owner: isize) -> impl Future<Output = Result<(), String>> {
    let (done, result) = futures::channel::oneshot::channel();
    let running = Running::start();
    let spawned = std::thread::Builder::new().name("file-operation".into()).spawn(move || {
        let _running = running;
        let _ = done.send(perform(&op, owner));
    });
    async move {
        match spawned {
            Ok(_) => result.await.unwrap_or_else(|_| Err(String::new())),
            Err(_) => Err(String::new()),
        }
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
pub(crate) mod win {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
        CoUninitialize, DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL,
    };
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE, ReleaseStgMedium};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        DROPFILES, DragQueryFileW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_RENAMEONCOLLISION,
        FOF_WANTNUKEWARNING, FOFX_RECYCLEONDELETE, FileOperation, HDROP, IFileOperation, IShellItem, IShellItemArray,
        ILFree, SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists, SHMultiFileProperties,
        SHParseDisplayName,
    };
    use windows::core::{HRESULT, PCWSTR, w};

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// The shell items of `paths`, which may come from different folders.
    pub fn shell_items(paths: &[PathBuf]) -> windows::core::Result<IShellItemArray> {
        unsafe {
            let mut pidls: Vec<*mut ITEMIDLIST> = Vec::with_capacity(paths.len());
            for p in paths {
                let w = wide(p.as_os_str());
                let mut pidl = std::ptr::null_mut();
                if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() {
                    pidls.push(pidl);
                }
            }
            let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as _).collect();
            let items = SHCreateShellItemArrayFromIDLists(&consts);
            for p in pidls {
                ILFree(Some(p));
            }
            items
        }
    }

    fn shell_item(path: &Path) -> windows::core::Result<IShellItem> {
        let w = wide(path.as_os_str());
        unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
    }

    /// The user cancelled (the progress dialog's Cancel, "Skip" on the only item…).
    fn is_cancel(code: HRESULT) -> bool {
        const COPYENGINE_E_USER_CANCELLED: u32 = 0x8027_0000;
        const ERROR_CANCELLED: u32 = 0x8007_04C7;
        matches!(code.0 as u32, COPYENGINE_E_USER_CANCELLED | ERROR_CANCELLED)
    }

    pub(crate) fn perform(op: &FileOp, owner: isize) -> Result<(), String> {
        if let FileOp::EmptyBin = op {
            crate::recycle::empty(owner);
            return Ok(());
        }
        let result = perform_shell(op, owner);
        match op {
            // Items that left the bin leave their `$I…` files behind.
            FileOp::Restore { items } => {
                crate::recycle::forget_gone(&items.iter().map(|(data, _)| data.clone()).collect::<Vec<_>>())
            }
            FileOp::Purge { paths } => crate::recycle::forget_gone(paths),
            _ => {}
        }
        if op.touches_bin() {
            crate::recycle::update_icon();
        }
        result
    }

    fn perform_shell(op: &FileOp, owner: isize) -> Result<(), String> {
        unsafe {
            let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            let result = (|| -> windows::core::Result<()> {
                let fo: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
                let mut flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR;
                match op {
                    FileOp::Copy { paths, dest } => {
                        // Pasting into the same folder makes "name - Copy".
                        if paths.iter().any(|p| p.parent() == Some(dest.as_path())) {
                            flags |= FOF_RENAMEONCOLLISION;
                        }
                    }
                    FileOp::Delete { recycle: true, .. } => flags |= FOFX_RECYCLEONDELETE | FOF_WANTNUKEWARNING,
                    FileOp::Delete { recycle: false, .. } => flags = FOF_NOCONFIRMMKDIR,
                    // The shell would ask with the bin's own file names ("$RX1Y2Z.txt").
                    FileOp::Purge { .. } => flags = FOF_NOCONFIRMMKDIR | FOF_NOCONFIRMATION,
                    _ => {}
                }
                fo.SetOperationFlags(flags)?;
                if owner != 0 {
                    fo.SetOwnerWindow(HWND(owner as _))?;
                }
                match op {
                    FileOp::Copy { paths, dest } => fo.CopyItems(&shell_items(paths)?, &shell_item(dest)?)?,
                    FileOp::Move { paths, dest } => fo.MoveItems(&shell_items(paths)?, &shell_item(dest)?)?,
                    FileOp::Delete { paths, .. } => fo.DeleteItems(&shell_items(paths)?)?,
                    FileOp::Rename { path, name } => {
                        let name = wide(name.as_ref());
                        fo.RenameItem(&shell_item(path)?, PCWSTR(name.as_ptr()), None)?
                    }
                    FileOp::Restore { items } => {
                        for (data, original) in items {
                            let (Some(folder), Some(name)) = (original.parent(), original.file_name()) else { continue };
                            // The folder it was in may be gone too.
                            let _ = std::fs::create_dir_all(folder);
                            let name = wide(name);
                            fo.MoveItem(&shell_item(data)?, &shell_item(folder)?, PCWSTR(name.as_ptr()), None)?;
                        }
                    }
                    FileOp::Purge { paths } => fo.DeleteItems(&shell_items(paths)?)?,
                    FileOp::EmptyBin => return Ok(()),
                }
                fo.PerformOperations()
            })();
            if init.is_ok() {
                CoUninitialize();
            }
            match result {
                Err(err) if !is_cancel(err.code()) => Err(err.message()),
                _ => Ok(()),
            }
        }
    }

    /// Paths of a file list (`CF_HDROP`), maybe empty.
    pub(super) fn hdrop_paths(hdrop: HDROP) -> Vec<PathBuf> {
        unsafe {
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            (0..count)
                .filter_map(|i| {
                    let len = DragQueryFileW(hdrop, i, None) as usize;
                    let mut buffer = vec![0u16; len + 1];
                    let got = DragQueryFileW(hdrop, i, Some(&mut buffer)) as usize;
                    (got > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..got])))
                })
                .collect()
        }
    }

    /// The files a dragged data object carries; empty for anything else (text, zip
    /// contents, mail attachments…).
    pub fn data_object_paths(data: &IDataObject) -> Vec<PathBuf> {
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        unsafe {
            let Ok(mut medium) = data.GetData(&format) else { return Vec::new() };
            let paths = if medium.u.hGlobal.is_invalid() { Vec::new() } else { hdrop_paths(HDROP(medium.u.hGlobal.0)) };
            ReleaseStgMedium(&mut medium);
            paths
        }
    }

    /// Explorer's multi-item Properties sheet (total size, attributes of all of them).
    pub fn show_properties_of(paths: &[PathBuf]) {
        unsafe {
            if let Ok(items) = shell_items(paths)
                && let Ok(data) = items.BindToHandler::<_, IDataObject>(None, &windows::Win32::UI::Shell::BHID_DataObject)
            {
                let _ = SHMultiFileProperties(&data, 0);
            }
        }
    }

    /// Opens the clipboard, retrying for a moment while another app holds it.
    fn open_clipboard(owner: Option<HWND>) -> bool {
        for _ in 0..10 {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        false
    }

    fn preferred_effect_format() -> u32 {
        unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) }
    }

    /// A file list (`CF_HDROP`): a `DROPFILES` header, then the paths, each ending with
    /// a null, and one more null.
    pub(super) fn hdrop_memory(paths: &[PathBuf]) -> Option<HGLOBAL> {
        let mut names: Vec<u16> = Vec::new();
        for p in paths {
            names.extend(p.as_os_str().encode_wide());
            names.push(0);
        }
        names.push(0);
        unsafe {
            let header = size_of::<DROPFILES>();
            let files = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, header + names.len() * 2).ok()?;
            let ptr = GlobalLock(files) as *mut u8;
            if ptr.is_null() {
                let _ = GlobalFree(Some(files));
                return None;
            }
            (ptr as *mut DROPFILES).write(DROPFILES { pFiles: header as u32, fWide: true.into(), ..Default::default() });
            std::ptr::copy_nonoverlapping(names.as_ptr(), ptr.add(header) as *mut u16, names.len());
            let _ = GlobalUnlock(files);
            Some(files)
        }
    }

    /// Puts `paths` on the clipboard as Explorer does for Copy (or Cut: pasting moves
    /// them). `owner` must be a window of the app. Returns the clipboard's sequence
    /// number afterwards, to tell later whether it still holds these files.
    pub fn set_clipboard_files(paths: &[PathBuf], cut: bool, owner: isize) -> Option<u32> {
        let files = hdrop_memory(paths)?;
        unsafe {
            let effect = global_u32(if cut { DROPEFFECT_MOVE.0 } else { DROPEFFECT_COPY.0 });
            if !open_clipboard(Some(HWND(owner as _))) {
                let _ = GlobalFree(Some(files));
                if let Some(effect) = effect {
                    let _ = GlobalFree(Some(effect));
                }
                return None;
            }
            let _ = EmptyClipboard();
            // The clipboard owns the memory once `SetClipboardData` succeeds.
            if SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(files.0))).is_err() {
                let _ = GlobalFree(Some(files));
            }
            if let Some(effect) = effect
                && SetClipboardData(preferred_effect_format(), Some(HANDLE(effect.0))).is_err()
            {
                let _ = GlobalFree(Some(effect));
            }
            let _ = CloseClipboard();
            Some(GetClipboardSequenceNumber())
        }
    }

    fn global_u32(value: u32) -> Option<HGLOBAL> {
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, 4).ok()?;
            let ptr = GlobalLock(memory) as *mut u32;
            if ptr.is_null() {
                let _ = GlobalFree(Some(memory));
                return None;
            }
            ptr.write(value);
            let _ = GlobalUnlock(memory);
            Some(memory)
        }
    }

    pub fn clipboard_has_files() -> bool {
        unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() }
    }

    /// Files on the clipboard, and whether they were cut (pasting moves them).
    pub fn clipboard_files() -> Option<(Vec<PathBuf>, bool)> {
        if !clipboard_has_files() || !open_clipboard(None) {
            return None;
        }
        unsafe {
            let paths = GetClipboardData(CF_HDROP.0 as u32).ok().map(|h| hdrop_paths(HDROP(h.0))).unwrap_or_default();
            let cut = GetClipboardData(preferred_effect_format()).ok().is_some_and(|h| {
                let ptr = GlobalLock(HGLOBAL(h.0)) as *const u32;
                let cut = !ptr.is_null() && ptr.read() & DROPEFFECT_MOVE.0 != 0;
                if !ptr.is_null() {
                    let _ = GlobalUnlock(HGLOBAL(h.0));
                }
                cut
            });
            let _ = CloseClipboard();
            (!paths.is_empty()).then_some((paths, cut))
        }
    }

    pub fn clipboard_sequence() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    /// Empties the clipboard if it still holds what was there at `sequence`: files that
    /// were cut and have been moved can't be pasted again.
    pub fn clear_clipboard_if(sequence: u32, owner: isize) {
        unsafe {
            if GetClipboardSequenceNumber() == sequence && open_clipboard(Some(HWND(owner as _))) {
                let _ = EmptyClipboard();
                let _ = CloseClipboard();
            }
        }
    }
}

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub(crate) fn perform(_op: &FileOp, _owner: isize) -> Result<(), String> {
        Err(String::new())
    }
    pub fn show_properties_of(_paths: &[PathBuf]) {}
    pub fn set_clipboard_files(_paths: &[PathBuf], _cut: bool, _owner: isize) -> Option<u32> {
        None
    }
    pub fn clipboard_has_files() -> bool {
        false
    }
    pub fn clipboard_files() -> Option<(Vec<PathBuf>, bool)> {
        None
    }
    pub fn clipboard_sequence() -> u32 {
        0
    }
    pub fn clear_clipboard_if(_sequence: u32, _owner: isize) {}
}
#[cfg(not(windows))]
pub use fallback::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn file_lists_round_trip() {
        use windows::Win32::Foundation::GlobalFree;
        use windows::Win32::UI::Shell::HDROP;
        let paths = vec![PathBuf::from(r"C:\a\x.txt"), PathBuf::from(r"D:\ünï cödé\folder")];
        let memory = win::hdrop_memory(&paths).unwrap();
        assert_eq!(win::hdrop_paths(HDROP(memory.0)), paths);
        unsafe {
            let _ = GlobalFree(Some(memory));
        }
    }

    /// What a file drag hands over: the shell's data object, with the file list Explorer
    /// and the drop targets read, also for items from different folders (tree view).
    #[cfg(windows)]
    #[test]
    fn shell_data_objects_carry_file_lists() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IDataObject};
        use windows::Win32::UI::Shell::BHID_DataObject;
        let root = crate::fs::test_temp_dir().join(format!("fm-data-{}", std::process::id()));
        let paths = vec![root.join("a").join("x.txt"), root.join("b").join("y.txt")];
        for p in &paths {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let items = shell_items(&paths).unwrap();
            let data: IDataObject = items.BindToHandler(None, &BHID_DataObject).unwrap();
            assert_eq!(data_object_paths(&data), paths);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The real thing, through the shell, in a temporary folder (no dialog comes up:
    /// nothing is deleted and nothing collides).
    #[cfg(windows)]
    #[test]
    fn shell_copies_moves_and_renames() {
        let root = crate::fs::test_temp_dir().join(format!("fm-ops-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("x.txt"), "x").unwrap();

        perform(&FileOp::Copy { paths: vec![a.join("x.txt")], dest: b.clone() }, 0).unwrap();
        assert!(a.join("x.txt").exists() && b.join("x.txt").exists());
        perform(&FileOp::Rename { path: b.join("x.txt"), name: "y.txt".into() }, 0).unwrap();
        assert!(b.join("y.txt").exists() && !b.join("x.txt").exists());
        perform(&FileOp::Move { paths: vec![b.join("y.txt")], dest: a.clone() }, 0).unwrap();
        assert!(a.join("y.txt").exists() && !b.join("y.txt").exists());
        // A copy next to its original gets a new name.
        perform(&FileOp::Copy { paths: vec![a.join("x.txt")], dest: a.clone() }, 0).unwrap();
        assert_eq!(std::fs::read_dir(&a).unwrap().count(), 3);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn names() {
        assert!(valid_name("report.txt"));
        assert!(valid_name(".gitignore"));
        for bad in ["", "a/b", "a\\b", "what?", "x:y", "..", "trailing.", "trailing "] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert_eq!(stem_len("photo.final.jpg", false), "photo.final".len());
        assert_eq!(stem_len(".gitignore", false), ".gitignore".len());
        assert_eq!(stem_len("folder.v2", true), "folder.v2".len());
    }

    #[test]
    fn unique_names_skip_taken_ones() {
        let dir = crate::fs::test_temp_dir().join(format!("fm-unique-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("New folder")).unwrap();
        std::fs::create_dir_all(dir.join("New folder (2)")).unwrap();
        assert_eq!(unique_name(&dir, "New folder"), "New folder (3)");
        assert_eq!(unique_name(&dir, "Other"), "Other");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn results_keep_names() {
        let op = FileOp::Copy { paths: vec![r"C:\a\x.txt".into(), r"C:\a\sub".into()], dest: r"D:\b".into() };
        assert_eq!(op.results(), [PathBuf::from(r"D:\b\x.txt"), PathBuf::from(r"D:\b\sub")]);
        let op = FileOp::Rename { path: r"C:\a\x.txt".into(), name: "y.txt".into() };
        assert_eq!(op.results(), [PathBuf::from(r"C:\a\y.txt")]);
    }
}
