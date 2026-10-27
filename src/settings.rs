//! User settings, persisted as JSON in `%APPDATA%\FileManager\settings.json`, or in
//! `settings.json` next to the exe in the portable build (see `Location`).
//!
//! Settings are a GPUI global. Every field has a default and unknown or missing
//! fields are ignored, so old and new files keep loading as settings are added.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::app::ViewMode;
use crate::fs::Sort;
use crate::i18n::Language;
use crate::theme::{Accent, ThemeChoice};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub theme: ThemeChoice,
    /// Accent of the dimmed theme.
    pub accent: Accent,
    /// Sidebar favorites, in order. `None` until the user changes them: the defaults
    /// (the known folders) are then resolved at startup.
    pub favorites: Option<Vec<PathBuf>>,
    pub favorites_open: bool,
    pub drives_open: bool,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    /// View mode of new tabs: the last one picked.
    pub view_mode: ViewMode,
    /// Sorting of new tabs: the last one picked.
    pub sort: Sort,
    pub preview_pane: bool,
    /// Hidden and system files are listed (dimmed).
    pub show_hidden: bool,
    /// File names end with their extension (shortcuts never show theirs, as in Explorer).
    pub show_extensions: bool,
    /// The size column shows folder sizes, computed on demand for the listed folder's
    /// subfolders (`folder_sizes`).
    pub folder_sizes: bool,
    /// Names of folders the size walks leave out (build caches, `.git`…); a size that
    /// skipped one is shown as "≥".
    pub size_exclusions: Vec<String>,
    /// What the cover flow view shows.
    pub media_filter: crate::fs::MediaFilter,
    /// Height of the covers over the list, in pixels.
    pub flow_height: f32,
    /// View and sorting kept for single folders, by `folder_key`. It only grows
    /// through an explicit "Keep for this folder".
    pub folders: BTreeMap<String, FolderPrefs>,
    /// The one-time tips offering to keep a sorting or a view for the folder were shown.
    pub sort_tip_shown: bool,
    pub view_tip_shown: bool,
}

/// What is kept for one folder; `None` follows the tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderPrefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Sort>,
}

/// Folders the size walks skip unless the user says otherwise: build and package
/// caches that are huge and nobody wants counted.
pub const DEFAULT_EXCLUSIONS: [&str; 4] = ["Intermediate", "DerivedDataCache", ".git", "node_modules"];

/// Windows paths are case-insensitive and may come with a trailing separator.
fn folder_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('/', "\\").to_lowercase();
    key.trim_end_matches('\\').to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::default(),
            theme: ThemeChoice::default(),
            accent: Accent::default(),
            favorites: None,
            favorites_open: true,
            drives_open: true,
            sidebar_width: 230.,
            sidebar_collapsed: false,
            view_mode: ViewMode::default(),
            sort: Sort::default(),
            preview_pane: false,
            show_hidden: true,
            show_extensions: true,
            folder_sizes: false,
            size_exclusions: DEFAULT_EXCLUSIONS.iter().map(|s| s.to_string()).collect(),
            media_filter: crate::fs::MediaFilter::default(),
            flow_height: 330.,
            folders: BTreeMap::new(),
            sort_tip_shown: false,
            view_tip_shown: false,
        }
    }
}

impl Global for Settings {}

/// Saves run on background threads: only the newest one writes, one at a time.
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: Mutex<()> = Mutex::new(());

/// The portable build (`--features portable`) keeps its settings next to the exe. The
/// release ships it as its own exe and zip; the installers ship the other build.
pub const PORTABLE: bool = cfg!(feature = "portable");

/// Where the settings file is, chosen once per process.
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub path: PathBuf,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// `%APPDATA%\FileManager`.
    User,
    /// The portable build: the exe's folder.
    Portable,
    /// The portable build in a folder that can't be written: back to `%APPDATA%`.
    PortableReadOnly,
}

impl Location {
    pub fn get() -> Option<&'static Self> {
        static LOCATION: OnceLock<Option<Location>> = OnceLock::new();
        LOCATION
            .get_or_init(|| {
                let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
                let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
                Self::choose(PORTABLE, exe_dir.as_deref(), appdata.as_deref(), writable)
            })
            .as_ref()
    }

    fn choose(
        portable: bool,
        exe_dir: Option<&Path>,
        appdata: Option<&Path>,
        writable: impl Fn(&Path) -> bool,
    ) -> Option<Self> {
        let user = |mode| appdata.map(|dir| Self { path: dir.join("FileManager").join("settings.json"), mode });
        if !portable {
            return user(Mode::User);
        }
        match exe_dir {
            Some(dir) if writable(dir) => Some(Self { path: dir.join("settings.json"), mode: Mode::Portable }),
            _ => user(Mode::PortableReadOnly),
        }
    }
}

/// Whether files can be created in `dir` (a read-only medium, a folder under Program
/// Files without admin rights…). The attributes don't say: only trying does.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".filemanager-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        Location::get().map(|l| l.path.clone())
    }

    /// Reads the settings file, falling back to defaults when it is missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// What is kept for `folder`.
    pub fn folder(&self, folder: &Path) -> FolderPrefs {
        self.folders.get(&folder_key(folder)).copied().unwrap_or_default()
    }

    /// Changes what is kept for `folder`; a folder left with nothing kept is dropped.
    pub fn edit_folder(&mut self, folder: &Path, f: impl FnOnce(&mut FolderPrefs)) {
        let key = folder_key(folder);
        let mut prefs = self.folders.get(&key).copied().unwrap_or_default();
        f(&mut prefs);
        if prefs == FolderPrefs::default() {
            self.folders.remove(&key);
        } else {
            self.folders.insert(key, prefs);
        }
    }

    /// Applies `f`, saves the result on a background thread and redraws every window.
    /// Observe with `cx.observe_global_in::<Settings>` to update state that caches settings.
    pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
        let mut settings = cx.global::<Self>().clone();
        f(&mut settings);
        if &settings == cx.global::<Self>() {
            return;
        }
        cx.set_global(settings.clone());
        let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        cx.background_spawn(async move {
            let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if SAVE_GENERATION.load(Ordering::SeqCst) == generation
                && let Err(err) = settings.save()
            {
                eprintln!("settings: {err}");
            }
        })
        .detach();
        cx.refresh_windows();
    }

    /// Writes to a temporary file first so a crash never leaves a truncated file behind.
    fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that glob includes gpui_kit's `test` macro, which shadows `#[test]`.
    use super::{Language, Location, Mode, Settings, ThemeChoice, ViewMode, writable};
    use crate::fs::{Sort, SortKey};
    use std::path::Path;

    #[test]
    fn portable_build_picks_the_exe_folder() {
        let root = std::env::temp_dir().join(format!("fm-portable-{}", std::process::id()));
        let exe_dir = root.join("app");
        let appdata = root.join("appdata");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let user = Some(Location { path: appdata.join("FileManager").join("settings.json"), mode: Mode::User });

        // The installed build: %APPDATA%, even in a writable folder.
        assert_eq!(Location::choose(false, Some(&exe_dir), Some(&appdata), |_| true), user);

        assert_eq!(
            Location::choose(true, Some(&exe_dir), Some(&appdata), writable),
            Some(Location { path: exe_dir.join("settings.json"), mode: Mode::Portable })
        );
        // The probe doesn't stay behind.
        assert_eq!(std::fs::read_dir(&exe_dir).unwrap().count(), 0);
        // A folder that can't be written: back to %APPDATA%.
        let fallback =
            Some(Location { path: appdata.join("FileManager").join("settings.json"), mode: Mode::PortableReadOnly });
        assert_eq!(Location::choose(true, Some(&exe_dir), Some(&appdata), |_| false), fallback);
        assert_eq!(Location::choose(true, None, Some(&appdata), |_| true), fallback);
        assert_eq!(Location::choose(true, Some(&exe_dir), None, |_| false), None);

        assert!(!writable(&root.join("missing")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_and_unknown_fields_fall_back() {
        let s: Settings = serde_json::from_str(r#"{"future_option": true}"#).unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"language": "it", "theme": "light"}"#).unwrap();
        assert_eq!(s.language, Language::Italian);
        assert_eq!(s.theme, ThemeChoice::Light);
        assert!(s.favorites_open);
        assert!(s.show_hidden && s.show_extensions);
    }

    #[test]
    fn folder_prefs_ignore_case_and_trailing_separator() {
        let mut s = Settings::default();
        let sort = Sort { key: SortKey::Modified, descending: true };
        s.edit_folder(Path::new(r"C:\Users\Me\"), |p| p.sort = Some(sort));
        s.edit_folder(Path::new(r"c:\users\me"), |p| p.view = Some(ViewMode::List));
        let prefs = s.folder(Path::new(r"C:\USERS\ME"));
        assert_eq!((prefs.sort, prefs.view), (Some(sort), Some(ViewMode::List)));
        assert_eq!(s.folders.len(), 1);

        // Forgetting both drops the folder.
        s.edit_folder(Path::new(r"C:\Users\Me"), |p| *p = Default::default());
        assert!(s.folders.is_empty());

        s.edit_folder(Path::new(r"C:\Users\Me"), |p| p.sort = Some(sort));
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
    }
}
