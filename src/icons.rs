//! Native file icons and thumbnails, with bounded memory.
//!
//! Views ask for an entry's icon or thumbnail while rendering. A miss queues a job
//! for the worker threads (`shell_images`) and draws a fallback; results come back in
//! batches, land in byte-budgeted LRU caches and trigger a redraw. Icons are cached
//! by system image list index, so every file of a type shares one bitmap.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::*;

use crate::fs::Entry;
pub use crate::shell_images::IconSize;
use crate::shell_images::{self, Bitmap};

/// Icon bitmaps: a few hundred distinct icons at most, but jumbo ones are 256 KB each.
const ICON_BUDGET: usize = 24 << 20;
/// Thumbnails for the icon views and the preview pane.
const THUMB_BUDGET: usize = 128 << 20;
/// Jobs waiting per queue. The newest are served first; the oldest are dropped.
const QUEUE_CAP: usize = 768;

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

/// Extensions whose icon is stored in the file itself (or its target).
const OWN_ICON: &[&str] = &["exe", "lnk", "url", "ico", "cur", "ani", "scr", "msc", "cpl", "appref-ms", "website"];

/// Extensions the shell can make a thumbnail of (with the stock or common codecs).
const THUMBNAIL: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "gif", "bmp", "webp", "tif", "tiff", "ico", "heic", "heif", "avif", "jxr",
    "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf", "psd", "tga", "dds", "svg", "mp4", "m4v", "mov",
    "mkv", "avi", "wmv", "webm", "mpg", "mpeg", "ts", "m2ts", "mts", "3gp", "flv", "pdf",
];

pub fn has_thumbnail(e: &Entry) -> bool {
    !e.is_dir && THUMBNAIL.contains(&e.ext.as_ref())
}

/// What decides an entry's icon: its type, or the item itself.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum IndexKey {
    Folder,
    Ext(SharedString),
    Path(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ThumbKey {
    path: PathBuf,
    modified: Option<SystemTime>,
    px: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Pending {
    Index(IndexKey),
    Image(i32, IconSize),
    Thumb(ThumbKey),
}

enum Job {
    /// Resolve the image list index of `probe`, then load its bitmap at `size`.
    Index { key: IndexKey, probe: PathBuf, attrs: u32, from_attributes: bool, overlay: bool, size: IconSize },
    Image { index: i32, size: IconSize },
    Thumb(ThumbKey),
}

enum Done {
    Index { key: IndexKey, index: Option<i32>, size: IconSize, bitmap: Option<Bitmap> },
    Image { index: i32, size: IconSize, bitmap: Option<Bitmap> },
    Thumb { key: ThumbKey, bitmap: Option<Bitmap> },
}

impl Job {
    fn pending(&self) -> Pending {
        match self {
            Job::Index { key, .. } => Pending::Index(key.clone()),
            Job::Image { index, size } => Pending::Image(*index, *size),
            Job::Thumb(key) => Pending::Thumb(key.clone()),
        }
    }

    fn run(self) -> Done {
        match self {
            Job::Index { key, probe, attrs, from_attributes, overlay, size } => {
                let index = shell_images::icon_index(&probe, attrs, from_attributes, overlay);
                let bitmap = index.and_then(|i| shell_images::icon_bitmap(i, size));
                Done::Index { key, index, size, bitmap }
            }
            Job::Image { index, size } => Done::Image { index, size, bitmap: shell_images::icon_bitmap(index, size) },
            Job::Thumb(key) => {
                let bitmap = shell_images::thumbnail(&key.path, key.px);
                Done::Thumb { key, bitmap }
            }
        }
    }
}

/// LIFO job queue shared with the worker threads.
struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
}

impl Queue {
    fn new() -> Arc<Self> {
        Arc::new(Self { jobs: Mutex::new(VecDeque::new()), ready: Condvar::new() })
    }

    /// Queues `job`; returns the job pushed out when the queue is full.
    fn push(&self, job: Job) -> Option<Job> {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.push_back(job);
        let dropped = (jobs.len() > QUEUE_CAP).then(|| jobs.pop_front()).flatten();
        self.ready.notify_one();
        dropped
    }

    fn pop(&self) -> Job {
        let mut jobs = self.jobs.lock().unwrap();
        loop {
            if let Some(job) = jobs.pop_back() {
                return job;
            }
            jobs = self.ready.wait(jobs).unwrap();
        }
    }

    fn spawn_workers(self: &Arc<Self>, name: &str, count: usize, done: &mpsc::UnboundedSender<Done>) {
        for i in 0..count {
            let (queue, done) = (self.clone(), done.clone());
            std::thread::Builder::new()
                .name(format!("{name}-{i}"))
                .spawn(move || {
                    shell_images::init_worker_thread();
                    loop {
                        if done.unbounded_send(queue.pop().run()).is_err() {
                            return;
                        }
                    }
                })
                .expect("spawn icon worker");
        }
    }
}

/// Map with a byte budget that evicts the least recently used entries.
struct Lru<K, V> {
    map: HashMap<K, (V, u64, usize)>,
    tick: u64,
    bytes: usize,
    budget: usize,
}

impl<K: Eq + Hash + Clone, V: Clone> Lru<K, V> {
    fn new(budget: usize) -> Self {
        Self { map: HashMap::new(), tick: 0, bytes: 0, budget }
    }

    fn get(&mut self, key: &K) -> Option<V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|(v, used, _)| {
            *used = tick;
            v.clone()
        })
    }

    /// Inserts and returns what had to go to stay within budget.
    fn insert(&mut self, key: K, value: V, bytes: usize) -> Vec<V> {
        self.tick += 1;
        if let Some((_, _, old)) = self.map.insert(key, (value, self.tick, bytes)) {
            self.bytes -= old;
        }
        self.bytes += bytes;
        let mut evicted = Vec::new();
        if self.bytes > self.budget {
            // Evict down to 3/4 of the budget so this sort runs rarely.
            let mut by_age: Vec<(u64, K)> = self.map.iter().map(|(k, (_, used, _))| (*used, k.clone())).collect();
            by_age.sort_unstable_by_key(|(used, _)| *used);
            for (_, key) in by_age {
                if self.bytes <= self.budget / 4 * 3 {
                    break;
                }
                if let Some((value, _, bytes)) = self.map.remove(&key) {
                    self.bytes -= bytes;
                    evicted.push(value);
                }
            }
        }
        evicted
    }
}

pub struct Icons {
    /// Image list index per icon key; -1 when the shell had no icon.
    index: HashMap<IndexKey, i32>,
    images: Lru<(i32, IconSize), Arc<RenderImage>>,
    /// `None`: the item has no thumbnail, use its icon.
    thumbs: Lru<ThumbKey, Option<Arc<RenderImage>>>,
    pending: HashSet<Pending>,
    icon_jobs: Arc<Queue>,
    thumb_jobs: Arc<Queue>,
}

impl Global for Icons {}

/// Starts the workers and the task that files their results. Call once at startup.
pub fn init(cx: &mut App) {
    let (done_tx, mut done_rx) = mpsc::unbounded();
    let icon_jobs = Queue::new();
    let thumb_jobs = Queue::new();
    icon_jobs.spawn_workers("icons", 1, &done_tx);
    // Video thumbnails can take a while: two workers, apart from the icon one.
    thumb_jobs.spawn_workers("thumbnails", 2, &done_tx);
    cx.set_global(Icons {
        index: HashMap::new(),
        images: Lru::new(ICON_BUDGET),
        thumbs: Lru::new(THUMB_BUDGET),
        pending: HashSet::new(),
        icon_jobs,
        thumb_jobs,
    });

    cx.spawn(async move |cx| {
        while let Some(first) = done_rx.next().await {
            // Let a burst of results pile up, then redraw once.
            cx.background_executor().timer(Duration::from_millis(12)).await;
            let mut batch = vec![first];
            while let Ok(more) = done_rx.try_recv() {
                batch.push(more);
            }
            cx.update(|cx| {
                let evicted = cx.global_mut::<Icons>().file(batch);
                for image in evicted {
                    cx.drop_image(image, None);
                }
                cx.refresh_windows();
            });
        }
    })
    .detach();
}

fn render_image(b: Bitmap) -> (Arc<RenderImage>, usize) {
    let bytes = b.bytes();
    let buffer = image::RgbaImage::from_raw(b.width, b.height, b.bgra).expect("bitmap size");
    (Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])), bytes)
}

impl Icons {
    fn file(&mut self, batch: Vec<Done>) -> Vec<Arc<RenderImage>> {
        let mut evicted = Vec::new();
        for done in batch {
            match done {
                Done::Index { key, index, size, bitmap } => {
                    self.pending.remove(&Pending::Index(key.clone()));
                    let index = index.unwrap_or(-1);
                    if self.index.len() > 8192 {
                        // Per-item keys (programs, shortcuts) accumulate while browsing.
                        self.index.retain(|k, _| !matches!(k, IndexKey::Path(_)));
                    }
                    self.index.insert(key, index);
                    if let Some(b) = bitmap {
                        let (image, bytes) = render_image(b);
                        evicted.extend(self.images.insert((index, size), image, bytes));
                    }
                }
                Done::Image { index, size, bitmap } => {
                    self.pending.remove(&Pending::Image(index, size));
                    if let Some(b) = bitmap {
                        let (image, bytes) = render_image(b);
                        evicted.extend(self.images.insert((index, size), image, bytes));
                    }
                }
                Done::Thumb { key, bitmap } => {
                    self.pending.remove(&Pending::Thumb(key.clone()));
                    let (image, bytes) = match bitmap.map(render_image) {
                        Some((image, bytes)) => (Some(image), bytes),
                        None => (None, 64),
                    };
                    evicted.extend(self.thumbs.insert(key, image, bytes).into_iter().flatten());
                }
            }
        }
        evicted
    }

    fn request(&mut self, job: Job) {
        let pending = job.pending();
        if !self.pending.insert(pending) {
            return;
        }
        let queue = if matches!(job, Job::Thumb(_)) { &self.thumb_jobs } else { &self.icon_jobs };
        if let Some(dropped) = queue.push(job) {
            self.pending.remove(&dropped.pending());
        }
    }

    fn icon(&mut self, e: &Entry, size: IconSize) -> Option<Arc<RenderImage>> {
        let (key, probe, attrs, from_attributes, overlay) = index_key(e);
        match self.index.get(&key).copied() {
            Some(-1) => None,
            Some(index) => {
                let image = self.images.get(&(index, size));
                if image.is_none() {
                    self.request(Job::Image { index, size });
                }
                image
            }
            None => {
                self.request(Job::Index { key, probe, attrs, from_attributes, overlay, size });
                None
            }
        }
    }

    /// `Some(None)` when the item has no thumbnail.
    fn thumbnail(&mut self, e: &Entry, px: u32) -> Option<Option<Arc<RenderImage>>> {
        let key = ThumbKey { path: e.path.clone(), modified: e.modified, px };
        let thumb = self.thumbs.get(&key);
        if thumb.is_none() {
            self.request(Job::Thumb(key));
        }
        thumb
    }
}

fn index_key(e: &Entry) -> (IndexKey, PathBuf, u32, bool, bool) {
    let own = |overlay| (IndexKey::Path(e.path.clone()), e.path.clone(), e.attrs, false, overlay);
    if e.is_dir {
        // Drives and customized folders (desktop.ini marks them read-only or system,
        // like Explorer checks) have their own icon; every other folder shares one.
        if e.path.parent().is_none() || e.attrs & (FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_SYSTEM) != 0 {
            own(false)
        } else {
            (IndexKey::Folder, PathBuf::from("folder"), FILE_ATTRIBUTE_DIRECTORY, true, false)
        }
    } else if OWN_ICON.contains(&e.ext.as_ref()) {
        own(matches!(e.ext.as_ref(), "lnk" | "url"))
    } else {
        let probe = PathBuf::from(format!("file.{}", e.ext));
        (IndexKey::Ext(e.ext.clone()), probe, FILE_ATTRIBUTE_NORMAL, true, false)
    }
}

/// The entry's native icon drawn at `display` size, or a generic one while it loads.
pub fn entry_icon(e: &Entry, size: IconSize, display: Pixels, cx: &mut App) -> AnyElement {
    match cx.global_mut::<Icons>().icon(e, size) {
        Some(image) => img(image).size(display).flex_shrink_0().into_any_element(),
        None => fallback_icon(e, display, cx),
    }
}

/// The entry's thumbnail if it has one, otherwise its icon; `display` is the box size.
pub fn entry_thumbnail(e: &Entry, px: u32, size: IconSize, display: Pixels, cx: &mut App) -> AnyElement {
    if has_thumbnail(e) {
        match cx.global_mut::<Icons>().thumbnail(e, px) {
            Some(Some(image)) => {
                return img(image).size(display).object_fit(ObjectFit::Contain).flex_shrink_0().into_any_element();
            }
            // Still loading: keep the box empty rather than flashing the icon.
            None => return div().size(display).flex_shrink_0().into_any_element(),
            Some(None) => {}
        }
    }
    entry_icon(e, size, display, cx)
}

fn fallback_icon(e: &Entry, display: Pixels, cx: &App) -> AnyElement {
    let (icon, color) = if e.is_dir {
        (IconName::Folder, cx.theme().warning)
    } else {
        (IconName::File, cx.theme().muted_foreground)
    };
    div()
        .size(display)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).size(display * 0.85).text_color(color))
        .into_any_element()
}
