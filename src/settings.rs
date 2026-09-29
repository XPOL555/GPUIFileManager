//! User settings, persisted as JSON in `%APPDATA%\FileManager\settings.json`.
//!
//! Settings are a GPUI global. Every field has a default and unknown or missing
//! fields are ignored, so old and new files keep loading as settings are added.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("FileManager").join("settings.json"))
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
    use super::{Language, Settings, ThemeChoice, ViewMode};
    use crate::fs::{Sort, SortKey};
    use std::path::Path;

    #[test]
    fn missing_and_unknown_fields_fall_back() {
        let s: Settings = serde_json::from_str(r#"{"future_option": true}"#).unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"language": "it", "theme": "light"}"#).unwrap();
        assert_eq!(s.language, Language::Italian);
        assert_eq!(s.theme, ThemeChoice::Light);
        assert!(s.favorites_open);
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
