//! User settings, persisted as JSON in `%APPDATA%\FileManager\settings.json`.
//!
//! Settings are a GPUI global. Every field has a default and unknown or missing
//! fields are ignored, so old and new files keep loading as settings are added.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::app::ViewMode;
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
    pub preview_pane: bool,
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
            preview_pane: false,
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
    use super::{Language, Settings, ThemeChoice};

    #[test]
    fn missing_and_unknown_fields_fall_back() {
        let s: Settings = serde_json::from_str(r#"{"future_option": true}"#).unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"language": "it", "theme": "light"}"#).unwrap();
        assert_eq!(s.language, Language::Italian);
        assert_eq!(s.theme, ThemeChoice::Light);
        assert!(s.favorites_open);
    }
}
