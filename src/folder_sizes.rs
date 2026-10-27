//! Folder sizes, computed on demand. The subfolders of a listed folder are walked on
//! worker threads, one job per folder, the newest listing first. A walk is cancelled
//! as soon as its listing is left (`Cancel`), skips reparse points (junctions,
//! symlinks: they loop, and their bytes belong elsewhere) and the excluded names from
//! `Settings::size_exclusions`, and reports a running count every few hundred
//! milliseconds. Results go in a cache of bounded size, reused for a while so coming
//! back to a folder doesn't walk it again. Nothing is watched, nothing is kept per
//! file: memory follows the number of folders seen, never the trees walked.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::*;

use crate::fs::FolderSize;

const WORKERS: usize = 2;
/// A finished size is reused this long before the folder is walked again.
const FRESH: Duration = Duration::from_secs(5 * 60);
/// Folders remembered at most; the oldest half goes when it is reached.
const CACHE_CAP: usize = 8192;
const REPORT_EVERY: Duration = Duration::from_millis(300);

/// The walks of one listing, cancelled all together when the listing is left.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

struct Job {
    path: PathBuf,
    cancel: Cancel,
    /// Lowercase names of folders left out.
    exclusions: Arc<Vec<String>>,
}

struct Report {
    path: PathBuf,
    /// `None`: the walk was cancelled.
    size: Option<FolderSize>,
}

/// LIFO job queue: the folders of the listing on screen come first.
struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    wake: Condvar,
}

impl Queue {
    fn push(&self, job: Job) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.len() >= 4096 {
            jobs.retain(|j| !j.cancel.is_cancelled());
        }
        jobs.push_back(job);
        self.wake.notify_one();
    }

    fn pop(&self) -> Job {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(job) = jobs.pop_back() {
                return job;
            }
            jobs = self.wake.wait(jobs).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn clear(&self) {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Sent to whoever shows folder sizes when results arrive. `requeue`: a walk was
/// cancelled, and a listing still waiting for that folder should ask again.
pub struct SizesChanged {
    pub requeue: bool,
}

pub struct FolderSizes {
    cache: HashMap<PathBuf, (FolderSize, Instant)>,
    /// Folders queued or being walked, with the cancel of the listing that asked.
    pending: HashMap<PathBuf, Cancel>,
    queue: Arc<Queue>,
    report: mpsc::UnboundedSender<Report>,
    enabled: bool,
    exclusions: Arc<Vec<String>>,
}

impl EventEmitter<SizesChanged> for FolderSizes {}

struct Handle(Entity<FolderSizes>);

impl Global for Handle {}

pub fn entity(cx: &App) -> Entity<FolderSizes> {
    cx.global::<Handle>().0.clone()
}

/// Starts the workers and the task that files their reports. Call once at startup.
pub fn init(cx: &mut App) {
    let (report_tx, mut report_rx) = mpsc::unbounded();
    let queue = Arc::new(Queue { jobs: Mutex::new(VecDeque::new()), wake: Condvar::new() });
    for i in 0..WORKERS {
        let (queue, report) = (queue.clone(), report_tx.clone());
        std::thread::Builder::new()
            .name(format!("folder-sizes-{i}"))
            .spawn(move || {
                loop {
                    let job = queue.pop();
                    if job.cancel.is_cancelled() {
                        report.unbounded_send(Report { path: job.path, size: None }).ok();
                        continue;
                    }
                    let size = walk(&job.path, &job.exclusions, &job.cancel, &mut |running| {
                        report.unbounded_send(Report { path: job.path.clone(), size: Some(running) }).ok();
                    });
                    report.unbounded_send(Report { path: job.path, size }).ok();
                }
            })
            .expect("spawn folder size worker");
    }
    let sizes = cx.new(|_| FolderSizes {
        cache: HashMap::new(),
        pending: HashMap::new(),
        queue,
        report: report_tx,
        enabled: false,
        exclusions: Arc::new(Vec::new()),
    });
    cx.set_global(Handle(sizes.clone()));

    cx.spawn(async move |cx| {
        while let Some(first) = report_rx.next().await {
            // Let a burst of reports pile up, then redraw once.
            cx.background_executor().timer(Duration::from_millis(12)).await;
            let mut batch = vec![first];
            while let Ok(more) = report_rx.try_recv() {
                batch.push(more);
            }
            sizes.update(cx, |sizes, cx| sizes.file(batch, cx));
        }
    })
    .detach();
}

impl FolderSizes {
    fn file(&mut self, batch: Vec<Report>, cx: &mut Context<Self>) {
        let mut requeue = false;
        for Report { path, size } in batch {
            match size {
                Some(size) => {
                    if size.done {
                        self.pending.remove(&path);
                    }
                    self.cache.insert(path, (size, Instant::now()));
                }
                None => {
                    self.pending.remove(&path);
                    requeue = true;
                }
            }
        }
        if self.cache.len() > CACHE_CAP {
            let mut by_age: Vec<(Instant, PathBuf)> = self.cache.iter().map(|(p, (_, t))| (*t, p.clone())).collect();
            by_age.sort();
            for (_, path) in by_age.into_iter().take(CACHE_CAP / 2) {
                self.cache.remove(&path);
            }
        }
        cx.emit(SizesChanged { requeue });
    }

    /// Follows the settings: turning the sizes off or changing the exclusions forgets
    /// everything computed and stops the walks. Returns whether anything changed.
    pub fn configure(&mut self, enabled: bool, exclusions: &[String]) -> bool {
        let exclusions: Vec<String> = exclusions.iter().map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).collect();
        if self.enabled == enabled && *self.exclusions == exclusions {
            return false;
        }
        self.enabled = enabled;
        self.exclusions = Arc::new(exclusions);
        self.cancel_all();
        self.cache.clear();
        true
    }

    fn cancel_all(&mut self) {
        self.queue.clear();
        for cancel in self.pending.values() {
            cancel.cancel();
        }
        self.pending.clear();
    }

    /// What is known about `path`: a finished size, maybe an old one, or the running
    /// count of a walk.
    pub fn lookup(&self, path: &Path) -> Option<FolderSize> {
        self.cache.get(path).map(|(size, _)| *size)
    }

    /// Asks for the sizes of `paths` under `cancel`; folders with a fresh size (unless
    /// `force`) or already on their way are skipped.
    pub fn request(&mut self, paths: Vec<PathBuf>, cancel: &Cancel, force: bool) {
        if !self.enabled {
            return;
        }
        for path in paths {
            if self.pending.contains_key(&path) {
                continue;
            }
            if !force && self.cache.get(&path).is_some_and(|(size, at)| size.done && at.elapsed() < FRESH) {
                continue;
            }
            self.pending.insert(path.clone(), cancel.clone());
            self.queue.push(Job { path, cancel: cancel.clone(), exclusions: self.exclusions.clone() });
        }
        // Keep the channel alive for the workers even if nothing was pushed.
        let _ = &self.report;
    }
}

#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

fn is_excluded(name: &std::ffi::OsStr, exclusions: &[String]) -> bool {
    !exclusions.is_empty() && exclusions.iter().any(|x| name.to_string_lossy().eq_ignore_ascii_case(x))
}

/// Adds up the files under `root`, folder by folder (a stack, no recursion), calling
/// `progress` now and then. `None` when cancelled.
pub fn walk(
    root: &Path,
    exclusions: &[String],
    cancel: &Cancel,
    progress: &mut dyn FnMut(FolderSize),
) -> Option<FolderSize> {
    let (mut bytes, mut partial) = (0u64, false);
    let mut stack = vec![root.to_path_buf()];
    let mut last_report = Instant::now();
    let mut seen = 0u32;
    while let Some(dir) = stack.pop() {
        let Ok(items) = std::fs::read_dir(&dir) else {
            partial = true;
            continue;
        };
        for item in items {
            seen = seen.wrapping_add(1);
            if seen % 256 == 0 {
                if cancel.is_cancelled() {
                    return None;
                }
                if last_report.elapsed() >= REPORT_EVERY {
                    progress(FolderSize { bytes, partial, done: false });
                    last_report = Instant::now();
                }
            }
            let Ok(item) = item else {
                partial = true;
                continue;
            };
            // On Windows the metadata comes with the directory entry: no extra syscall,
            // and it describes the link itself, never what it points to.
            let Ok(meta) = item.metadata() else {
                partial = true;
                continue;
            };
            if is_reparse_point(&meta) {
                continue;
            }
            if meta.is_dir() {
                if is_excluded(&item.file_name(), exclusions) {
                    partial = true;
                } else {
                    stack.push(item.path());
                }
            } else {
                bytes += meta.len();
            }
        }
    }
    Some(FolderSize { bytes, partial, done: true })
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that glob includes gpui_kit's `test` macro, which shadows `#[test]`.
    use super::{Cancel, walk};
    use crate::fs::FolderSize;
    use std::path::PathBuf;

    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("fm-sizes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("a.txt"), [0u8; 100]).unwrap();
        std::fs::write(root.join("src/b.txt"), [0u8; 200]).unwrap();
        std::fs::write(root.join("src/deep/c.txt"), [0u8; 300]).unwrap();
        std::fs::write(root.join("node_modules/pkg/big.js"), [0u8; 5000]).unwrap();
        root
    }

    #[test]
    fn adds_up_the_tree_and_skips_exclusions() {
        let root = tree("walk");
        let all = walk(&root, &[], &Cancel::default(), &mut |_| {}).unwrap();
        assert_eq!(all, FolderSize { bytes: 5600, partial: false, done: true });
        let some = walk(&root, &["node_modules".into()], &Cancel::default(), &mut |_| {}).unwrap();
        assert_eq!(some, FolderSize { bytes: 600, partial: true, done: true });
        // Exclusions compare case-insensitively.
        let some = walk(&root, &["NODE_MODULES".into()], &Cancel::default(), &mut |_| {}).unwrap();
        assert_eq!(some.bytes, 600);
        let missing = walk(&root.join("nope"), &[], &Cancel::default(), &mut |_| {}).unwrap();
        assert_eq!(missing, FolderSize { bytes: 0, partial: true, done: true });
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_cancelled_walk_returns_nothing() {
        let root = tree("cancel");
        // Enough entries for the cancel flag to be looked at.
        for i in 0..600 {
            std::fs::write(root.join(format!("f{i}")), b"x").unwrap();
        }
        let cancel = Cancel::default();
        cancel.cancel();
        assert_eq!(walk(&root, &[], &cancel, &mut |_| {}), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn junctions_are_not_followed() {
        let root = tree("junction");
        // A junction back to the root would loop forever if followed.
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(root.join("src").join("loop"))
            .arg(&root)
            .output()
            .unwrap();
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
        let all = walk(&root, &[], &Cancel::default(), &mut |_| {}).unwrap();
        assert_eq!(all, FolderSize { bytes: 5600, partial: false, done: true });
        std::fs::remove_dir_all(&root).unwrap();
    }
}
