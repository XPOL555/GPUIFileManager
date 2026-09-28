//! User settings, persisted as JSON in `%APPDATA%\FileManager\settings.json`.
//!
//! Settings are a GPUI global. Every field has a default and unknown or missing
//! fields are ignored, so old and new files keep loading as settings are added.

use std::path::PathBuf;

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::i18n::Language;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
}

impl Global for Settings {}

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

    /// Applies `f`, saves the result on a background thread and redraws every window.
    /// Observe with `cx.observe_global_in::<Settings>` to update state that caches settings.
    pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
        let mut settings = cx.global::<Self>().clone();
        f(&mut settings);
        if &settings == cx.global::<Self>() {
            return;
        }
        cx.set_global(settings.clone());
        cx.background_spawn(async move {
            if let Err(err) = settings.save() {
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
    use super::{Language, Settings};

    #[test]
    fn missing_and_unknown_fields_fall_back() {
        let s: Settings = serde_json::from_str(r#"{"future_option": true}"#).unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"language": "it"}"#).unwrap();
        assert_eq!(s.language, Language::Italian);
    }
}
