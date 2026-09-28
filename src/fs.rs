//! Directory listing. Pure std, no watchers, no recursion: a listing is a
//! snapshot of one folder and costs memory proportional to its entry count.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::SharedString;

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
        let ext = if is_dir {
            SharedString::default()
        } else {
            path.extension()
                .map(|e| e.to_string_lossy().to_lowercase().into())
                .unwrap_or_default()
        };
        out.push(Entry {
            hidden: is_hidden(&name, &meta),
            name: name.into(),
            path,
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            ext,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Type,
    Modified,
    Size,
}

/// Folders always come first; `descending` only flips the order inside each group.
pub fn compare(a: &Entry, b: &Entry, key: SortKey, descending: bool) -> Ordering {
    b.is_dir.cmp(&a.is_dir).then_with(|| {
        let ord = match key {
            SortKey::Name => Ordering::Equal,
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
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
    }
}
