//! Directory listing. Pure std, no watchers, no recursion: a listing is a
//! snapshot of one folder and costs memory proportional to its entry count.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::SharedString;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: SharedString,
    pub path: PathBuf,
    pub is_dir: bool,
    pub hidden: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Lowercased extension without the dot, empty for folders.
    pub ext: SharedString,
    /// Win32 `FILE_ATTRIBUTE_*` bits (0 on other platforms).
    pub attrs: u32,
    /// In the Recycle Bin: the folder the item was deleted from. Its `modified` is
    /// then the time it was deleted, and its `path` the file the bin keeps it in.
    pub origin: Option<Arc<Path>>,
}

/// Where the Recycle Bin is, as the shell parses it. Tabs and favorites hold it like a
/// folder; `recycle` lists it.
pub const RECYCLE_BIN: &str = "::{645FF040-5081-101B-9F08-08002B30309D}";

pub fn is_recycle_bin(path: &Path) -> bool {
    path.as_os_str() == RECYCLE_BIN
}

/// The folder above `path`; none above a drive or the Recycle Bin.
pub fn parent(path: &Path) -> Option<&Path> {
    if is_recycle_bin(path) { None } else { path.parent().filter(|p| !p.as_os_str().is_empty()) }
}

/// Extensions Explorer never shows, whatever the setting: shortcuts.
const HIDDEN_EXTENSIONS: &[&str] = &["lnk", "url", "pif", "appref-ms"];

const PHOTOS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif", "avif", "jxr", "dng", "cr2",
    "cr3", "nef", "arw", "orf", "rw2", "raf", "srw", "pef",
];
const VIDEOS: &[&str] =
    &["mp4", "m4v", "mov", "mkv", "avi", "wmv", "webm", "mpg", "mpeg", "ts", "m2ts", "mts", "3gp", "flv"];

pub fn is_photo(e: &Entry) -> bool {
    !e.is_dir && PHOTOS.contains(&e.ext.as_ref())
}

pub fn is_video(e: &Entry) -> bool {
    !e.is_dir && VIDEOS.contains(&e.ext.as_ref())
}

/// What the cover flow view lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaFilter {
    #[default]
    All,
    PhotosAndVideos,
    Photos,
}

impl MediaFilter {
    pub const ALL: [MediaFilter; 3] = [MediaFilter::All, MediaFilter::PhotosAndVideos, MediaFilter::Photos];

    pub fn admits(self, e: &Entry) -> bool {
        match self {
            MediaFilter::All => true,
            MediaFilter::PhotosAndVideos => is_photo(e) || is_video(e),
            MediaFilter::Photos => is_photo(e),
        }
    }
}

impl Entry {
    /// A folder known only by its path (sidebar favorites, drives, tabs), without
    /// touching the disk. It is marked read-only so its icon is looked up per folder:
    /// special folders like Desktop or Downloads have their own.
    pub fn folder(path: &Path) -> Self {
        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Self {
            name: name.into(),
            path: path.to_path_buf(),
            is_dir: true,
            hidden: false,
            size: 0,
            modified: None,
            ext: SharedString::default(),
            attrs: FILE_ATTRIBUTE_READONLY,
            origin: None,
        }
    }

    /// A Windows shortcut (`.lnk`).
    pub fn is_shortcut(&self) -> bool {
        !self.is_dir && self.ext == "lnk"
    }

    /// The name as listed: without the extension when extensions are hidden, and
    /// always for shortcuts, as Explorer does.
    pub fn display_name(&self, show_extensions: bool) -> &str {
        if self.is_dir || self.ext.is_empty() || (show_extensions && !HIDDEN_EXTENSIONS.contains(&self.ext.as_ref())) {
            return &self.name;
        }
        match self.name.rfind('.') {
            Some(dot) if dot > 0 => &self.name[..dot],
            _ => &self.name,
        }
    }

    /// `display_name` as a `SharedString`, without copying when it is the whole name.
    pub fn shown_name(&self, show_extensions: bool) -> SharedString {
        let shown = self.display_name(show_extensions);
        if shown.len() == self.name.len() { self.name.clone() } else { shown.to_string().into() }
    }
}

fn extension(path: &Path) -> SharedString {
    path.extension().map(|e| e.to_string_lossy().to_lowercase().into()).unwrap_or_default()
}

#[cfg(windows)]
fn attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes()
}

#[cfg(not(windows))]
fn attributes(_meta: &std::fs::Metadata) -> u32 {
    0
}

pub fn list(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        // On Windows DirEntry::metadata comes from FindNextFileW data: no extra syscall.
        let Ok(meta) = item.metadata() else { continue };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        let ext = if is_dir { SharedString::default() } else { extension(&path) };
        out.push(Entry {
            hidden: is_hidden(&name, &meta),
            name: name.into(),
            path,
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            ext,
            attrs: attributes(&meta),
            origin: None,
        });
    }
    Ok(out)
}

#[cfg(windows)]
fn is_hidden(_name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    Name,
    Type,
    Modified,
    Size,
}

/// Sort column and direction of a listing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Sort {
    /// A click on `key`'s column header: the other direction on the sorted column,
    /// ascending on any other.
    pub fn clicked(self, key: SortKey) -> Self {
        Self { key, descending: self.key == key && !self.descending }
    }
}

/// Folders always come first; `descending` only flips the order inside each group.
pub fn compare(a: &Entry, b: &Entry, key: SortKey, descending: bool) -> Ordering {
    b.is_dir.cmp(&a.is_dir).then_with(|| {
        let ord = match key {
            SortKey::Name => Ordering::Equal,
            // In the Recycle Bin the type column shows where items were deleted from.
            SortKey::Type if a.origin.is_some() => a.origin.cmp(&b.origin),
            SortKey::Type => a.ext.cmp(&b.ext),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Size => a.size.cmp(&b.size),
        }
        .then_with(|| natural_cmp(&a.name, &b.name));
        if descending { ord.reverse() } else { ord }
    })
}

/// Case-insensitive compare where digit runs compare numerically ("file2" < "file10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_number(&mut a);
                let nb = take_number(&mut b);
                let ord = na.trim_start_matches('0').len().cmp(&nb.trim_start_matches('0').len())
                    .then_with(|| na.trim_start_matches('0').cmp(nb.trim_start_matches('0')));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}

/// Splits address-bar text into the folder to list and the name prefix to match:
/// `C:\Users\pa` → (`Some("C:\Users\")`, `"pa"`), `C:` → (`None`, `"C:"`).
pub fn split_for_completion(text: &str) -> (Option<&str>, &str) {
    match text.rfind(['\\', '/']) {
        Some(i) => (Some(&text[..=i]), &text[i + 1..]),
        None => (None, text),
    }
}

/// Names of the subfolders of `dir` starting with `prefix` (case-insensitive),
/// in natural order, at most `limit`. Reads one folder, like `list`.
pub fn complete_dirs(dir: &Path, prefix: &str, limit: usize) -> Vec<String> {
    let prefix = prefix.to_lowercase();
    let Ok(items) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = items
        .filter_map(Result::ok)
        .filter(|item| {
            item.file_type()
                .map(|t| t.is_dir() || (t.is_symlink() && item.path().is_dir()))
                .unwrap_or(false)
        })
        .map(|item| item.file_name().to_string_lossy().into_owned())
        .filter(|name| name.to_lowercase().starts_with(&prefix))
        .collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    names.truncate(limit);
    names
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

pub fn format_time(t: Option<SystemTime>) -> String {
    t.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a", "file02b"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "file1", "File2", "file02b", "file10"]);
    }

    #[test]
    fn header_clicks_flip_only_the_sorted_column() {
        let by_name = Sort::default();
        let by_size = by_name.clicked(SortKey::Size);
        assert_eq!(by_size, Sort { key: SortKey::Size, descending: false });
        assert_eq!(by_size.clicked(SortKey::Size), Sort { key: SortKey::Size, descending: true });
        assert_eq!(by_size.clicked(SortKey::Size).clicked(SortKey::Size), by_size);
        let desc = Sort { key: SortKey::Modified, descending: true };
        assert_eq!(desc.clicked(SortKey::Name), by_name);
    }

    #[test]
    fn completion_split() {
        assert_eq!(split_for_completion(r"C:\Users\pa"), (Some(r"C:\Users\"), "pa"));
        assert_eq!(split_for_completion(r"C:\Users\"), (Some(r"C:\Users\"), ""));
        assert_eq!(split_for_completion("C:"), (None, "C:"));
        assert_eq!(split_for_completion("C:/tmp/x"), (Some("C:/tmp/"), "x"));
    }

    #[test]
    fn completes_only_matching_folders() {
        let root = std::env::temp_dir().join(format!("fm-complete-{}", std::process::id()));
        for d in ["Alpha", "alps", "Beta", "alpha10", "alpha2"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("alpha.txt"), "").unwrap();
        assert_eq!(complete_dirs(&root, "AL", 10), ["Alpha", "alpha2", "alpha10", "alps"]);
        assert_eq!(complete_dirs(&root, "al", 2), ["Alpha", "alpha2"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn display_names_hide_extensions_like_explorer() {
        let file = |name: &str| {
            let mut e = Entry::folder(Path::new(name));
            e.is_dir = false;
            e.ext = extension(Path::new(name));
            e
        };
        assert_eq!(file("photo.JPG").display_name(true), "photo.JPG");
        assert_eq!(file("photo.JPG").display_name(false), "photo");
        assert_eq!(file("archive.tar.gz").display_name(false), "archive.tar");
        // Shortcuts never show theirs; names that are only an extension keep it.
        assert_eq!(file("Editor.lnk").display_name(true), "Editor");
        assert_eq!(file(".gitignore").display_name(false), ".gitignore");
        assert_eq!(file("README").display_name(false), "README");
        assert_eq!(Entry::folder(Path::new("dir.d")).display_name(false), "dir.d");
    }

    #[test]
    fn parents_stop_at_drives_and_the_recycle_bin() {
        assert_eq!(parent(Path::new(r"C:\a\b")), Some(Path::new(r"C:\a")));
        assert_eq!(parent(Path::new(r"C:\")), None);
        assert_eq!(parent(Path::new(RECYCLE_BIN)), None);
        assert!(is_recycle_bin(&PathBuf::from(RECYCLE_BIN)));
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
    }
}
