//! The Recycle Bin, read straight from the disk. Every drive keeps the current user's
//! deleted items in `X:\$Recycle.Bin\<SID>`, each as a pair of files with the same
//! suffix: `$I…` says where the item was and when it was deleted, `$R…` is the item.
//! Restoring moves `$R…` back and drops `$I…`, as Explorer does; both go through the
//! shell (`file_ops`), so conflicts and progress show its dialogs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::SharedString;

use crate::fs::Entry;

/// Whether the bin held something when last checked, for its icon.
static FULL: AtomicBool = AtomicBool::new(false);

pub fn looks_full() -> bool {
    FULL.load(Ordering::Relaxed)
}

/// Looks at the bins again; returns whether the empty / full state changed.
pub fn check() -> bool {
    let full = !is_empty();
    FULL.swap(full, Ordering::Relaxed) != full
}

/// What a `$I…` file says about a deleted item.
#[derive(Debug, PartialEq)]
pub struct Info {
    pub original: PathBuf,
    /// Of the file, or of everything in the folder.
    pub size: u64,
    pub deleted: Option<SystemTime>,
}

/// Reads a `$I…` file: version 1 (Vista to 8.1) has a fixed 260-character path,
/// version 2 (Windows 10 and later) its length first.
pub fn parse_info(bytes: &[u8]) -> Option<Info> {
    let u64_at = |at: usize| bytes.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")));
    let (version, size, time) = (u64_at(0)?, u64_at(8)?, u64_at(16)?);
    let name = match version {
        1 => bytes.get(24..24 + 520)?,
        2 => {
            let chars = u32::from_le_bytes(bytes.get(24..28)?.try_into().ok()?) as usize;
            bytes.get(28..28 + chars * 2)?
        }
        _ => return None,
    };
    let wide: Vec<u16> = name.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    if wide.is_empty() {
        return None;
    }
    Some(Info { original: PathBuf::from(String::from_utf16_lossy(&wide)), size, deleted: filetime(time) })
}

/// A Win32 `FILETIME`: 100 ns ticks since 1601.
fn filetime(ticks: u64) -> Option<SystemTime> {
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    let since_epoch = ticks.checked_sub(UNIX_EPOCH_TICKS)?;
    UNIX_EPOCH.checked_add(Duration::from_nanos(since_epoch.checked_mul(100)?))
}

/// The `$I…` file that goes with an item's `$R…` file.
pub fn info_file(data: &Path) -> Option<PathBuf> {
    let name = data.file_name()?.to_str()?;
    let suffix = name.strip_prefix("$R")?;
    Some(data.with_file_name(format!("$I{suffix}")))
}

/// After a restore or a permanent delete: the `$I…` files of the items that are gone.
pub fn forget_gone(data: &[PathBuf]) {
    for item in data {
        if item.symlink_metadata().is_err()
            && let Some(info) = info_file(item)
        {
            let _ = std::fs::remove_file(info);
        }
    }
}

/// Every deleted item of the current user, on every drive.
pub fn list() -> Vec<Entry> {
    let mut out = Vec::new();
    for bin in bins() {
        let Ok(items) = std::fs::read_dir(&bin) else { continue };
        for item in items.flatten() {
            let name = item.file_name();
            let Some(suffix) = name.to_str().and_then(|n| n.strip_prefix("$I")) else { continue };
            let Some(info) = std::fs::read(item.path()).ok().and_then(|bytes| parse_info(&bytes)) else { continue };
            let data = bin.join(format!("$R{suffix}"));
            let Ok(meta) = data.symlink_metadata() else { continue };
            out.push(entry(info, data, &meta));
        }
    }
    out
}

fn entry(info: Info, data: PathBuf, meta: &std::fs::Metadata) -> Entry {
    let is_dir = meta.is_dir();
    let name = info
        .original
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| info.original.to_string_lossy().into_owned());
    let ext: SharedString = if is_dir {
        SharedString::default()
    } else {
        Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase().into()).unwrap_or_default()
    };
    Entry {
        name: name.into(),
        path: data,
        is_dir,
        hidden: false,
        size: info.size,
        modified: info.deleted,
        ext,
        attrs: attributes(meta),
        origin: Some(Arc::from(info.original.parent().unwrap_or(Path::new("")))),
    }
}

#[cfg(windows)]
fn attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    // The bin marks nothing hidden that the user had not.
    meta.file_attributes() & !0x6
}

#[cfg(not(windows))]
fn attributes(_meta: &std::fs::Metadata) -> u32 {
    0
}

/// Whether no drive holds anything deleted by the current user.
pub fn is_empty() -> bool {
    bins().iter().all(|bin| {
        std::fs::read_dir(bin).map_or(true, |mut items| {
            !items.any(|item| item.is_ok_and(|i| i.file_name().to_str().is_some_and(|n| n.starts_with("$I"))))
        })
    })
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::OnceLock;

    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, HWND, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::Win32::UI::Shell::SHEmptyRecycleBinW;

    // Not in the `windows` crate's metadata.
    windows_core::link!("shell32.dll" "system" fn SHUpdateRecycleBinIcon());
    use windows::core::{PCWSTR, PWSTR};

    /// The current user's SID ("S-1-5-21-…"), which names their bin folders.
    fn user_sid() -> Option<&'static str> {
        static SID: OnceLock<Option<String>> = OnceLock::new();
        SID.get_or_init(|| unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
            let mut len = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            // u64s keep the buffer aligned for TOKEN_USER.
            let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
            let read = GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr() as _), len, &mut len);
            let _ = CloseHandle(token);
            read.ok()?;
            let user = &*(buffer.as_ptr() as *const TOKEN_USER);
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
            let sid = text.to_string().ok();
            let _ = LocalFree(Some(HLOCAL(text.0 as _)));
            sid
        })
        .as_deref()
    }

    /// The user's bin folder on every local drive that has one (not network drives or
    /// optical discs, which have no bin).
    pub fn bins() -> Vec<PathBuf> {
        const DRIVE_REMOVABLE: u32 = 2;
        const DRIVE_FIXED: u32 = 3;
        const DRIVE_RAMDISK: u32 = 6;
        let Some(sid) = user_sid() else { return Vec::new() };
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .filter_map(|i| {
                let root = format!("{}:\\", (b'A' + i) as char);
                let wide: Vec<u16> = std::ffi::OsStr::new(&root).encode_wide().chain(std::iter::once(0)).collect();
                let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
                matches!(kind, DRIVE_REMOVABLE | DRIVE_FIXED | DRIVE_RAMDISK)
                    .then(|| PathBuf::from(root).join("$Recycle.Bin").join(sid))
                    .filter(|bin| bin.is_dir())
            })
            .collect()
    }

    /// Empties the bin after the shell's own confirmation. Blocks until it is done:
    /// call it off the UI thread.
    pub fn empty(owner: isize) {
        unsafe {
            let _ = SHEmptyRecycleBinW(Some(HWND(owner as _)), PCWSTR::null(), 0);
            SHUpdateRecycleBinIcon();
        }
    }

    /// The bin's icon on the desktop and in Explorer follows what the app did.
    pub fn update_icon() {
        unsafe { SHUpdateRecycleBinIcon() };
    }
}

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub fn bins() -> Vec<PathBuf> {
        Vec::new()
    }
    pub fn empty(_owner: isize) {}
    pub fn update_icon() {}
}
#[cfg(not(windows))]
pub use fallback::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn info_v2(path: &str, size: u64, ticks: u64) -> Vec<u8> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut bytes = Vec::new();
        bytes.extend(2u64.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(ticks.to_le_bytes());
        bytes.extend((wide.len() as u32).to_le_bytes());
        bytes.extend(wide.iter().flat_map(|c| c.to_le_bytes()));
        bytes
    }

    #[test]
    fn info_files_of_both_versions() {
        // 2024-01-01 00:00:00 UTC.
        let ticks = 133_485_408_000_000_000;
        let info = parse_info(&info_v2(r"C:\Users\me\Desktop\città.txt", 1234, ticks)).unwrap();
        assert_eq!(info.original, PathBuf::from(r"C:\Users\me\Desktop\città.txt"));
        assert_eq!(info.size, 1234);
        assert_eq!(info.deleted, Some(UNIX_EPOCH + Duration::from_secs(1_704_067_200)));

        let mut v1 = Vec::new();
        v1.extend(1u64.to_le_bytes());
        v1.extend(7u64.to_le_bytes());
        v1.extend(ticks.to_le_bytes());
        let mut path = [0u16; 260];
        for (slot, c) in path.iter_mut().zip(r"D:\old.bin".encode_utf16()) {
            *slot = c;
        }
        v1.extend(path.iter().flat_map(|c| c.to_le_bytes()));
        assert_eq!(parse_info(&v1).unwrap().original, PathBuf::from(r"D:\old.bin"));

        // Cut short, or of an unknown version.
        assert_eq!(parse_info(&info_v2("C:\\x", 1, ticks)[..30]), None);
        let mut v3 = info_v2("C:\\x", 1, ticks);
        v3[0] = 3;
        assert_eq!(parse_info(&v3), None);
    }

    #[test]
    fn data_and_info_files_pair_up() {
        assert_eq!(info_file(Path::new(r"C:\$Recycle.Bin\S-1\$RAB12CD.txt")), Some(PathBuf::from(r"C:\$Recycle.Bin\S-1\$IAB12CD.txt")));
        assert_eq!(info_file(Path::new(r"C:\x\file.txt")), None);
    }

    /// The real thing: a file sent to the bin is listed with where it came from, and
    /// comes back where it was.
    #[cfg(windows)]
    #[test]
    fn deleted_files_are_listed_and_restored() {
        use crate::file_ops::{FileOp, perform};
        let root = crate::fs::test_temp_dir().join(format!("fm-bin-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join(format!("fm recycle test {}.txt", std::process::id()));
        std::fs::write(&file, "bin").unwrap();

        perform(&FileOp::Delete { paths: vec![file.clone()], recycle: true }, 0).unwrap();
        assert!(!file.exists());
        let listed = list().into_iter().find(|e| e.name.as_ref() == file.file_name().unwrap().to_str().unwrap());
        let listed = listed.expect("the deleted file is in the bin");
        assert_eq!(listed.origin.as_deref(), Some(root.as_path()));
        assert_eq!(listed.size, 3);
        assert!(!is_empty());

        perform(&FileOp::Restore { items: vec![(listed.path.clone(), file.clone())] }, 0).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "bin");
        assert!(!listed.path.exists() && !info_file(&listed.path).unwrap().exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
