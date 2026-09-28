//! UI strings. The language lives in `Settings`, which views read while rendering,
//! so `set_language` re-translates every open window in place.

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::shell::KnownFolder;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "it")]
    Italian,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::Italian];

    /// The language's own name, shown untranslated in the picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Italian => "Italiano",
        }
    }

    /// Locale code for gpui-component's built-in strings.
    fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Italian => "it",
        }
    }

    fn strings(self) -> &'static Strings {
        match self {
            Language::English => &EN,
            Language::Italian => &IT,
        }
    }
}

pub fn language(cx: &App) -> Language {
    cx.try_global::<Settings>().map(|s| s.language).unwrap_or_default()
}

/// Applies the language stored in `Settings` to gpui-component's own strings. Call once at startup.
pub fn init(cx: &App) {
    gpui_kit::component::set_locale(language(cx).code());
}

pub fn set_language(language: Language, cx: &mut App) {
    gpui_kit::component::set_locale(language.code());
    Settings::update(cx, |s| s.language = language);
}

/// Strings for the current language.
pub fn t(cx: &App) -> &'static Strings {
    language(cx).strings()
}

pub struct Strings {
    pub places: &'static str,
    pub drives: &'static str,
    pub filter: &'static str,
    pub col_name: &'static str,
    pub col_type: &'static str,
    pub col_modified: &'static str,
    pub col_size: &'static str,
    pub folder: &'static str,
    pub file: &'static str,
    /// Type column for a file with an extension, e.g. "PDF file".
    pub file_of_type: fn(&str) -> String,
    /// Status bar: folder and file counts.
    pub counts: fn(usize, usize) -> String,
    pub selected: &'static str,
    pub settings: &'static str,
    pub language: &'static str,
    pub open: &'static str,
    pub open_new_tab: &'static str,
    pub open_new_window: &'static str,
    pub copy_path: &'static str,
    pub properties: &'static str,
    pub refresh: &'static str,
    pub show_hidden: &'static str,
    pub more_options: &'static str,
    known_folders: [&'static str; KnownFolder::COUNT],
}

impl Strings {
    pub fn known_folder(&self, folder: KnownFolder) -> &'static str {
        self.known_folders[folder as usize]
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

static EN: Strings = Strings {
    places: "PLACES",
    drives: "DRIVES",
    filter: "Filter…",
    col_name: "Name",
    col_type: "Type",
    col_modified: "Modified",
    col_size: "Size",
    folder: "Folder",
    file: "File",
    file_of_type: |ext| format!("{ext} file"),
    counts: |dirs, files| format!("{}, {}", plural(dirs, "folder", "folders"), plural(files, "file", "files")),
    selected: "selected",
    settings: "Settings",
    language: "Language",
    open: "Open",
    open_new_tab: "Open in new tab",
    open_new_window: "Open in new window",
    copy_path: "Copy path",
    properties: "Properties",
    refresh: "Refresh",
    show_hidden: "Show hidden files",
    more_options: "Show more options",
    known_folders: ["Home", "Desktop", "Downloads", "Documents", "Pictures", "Music", "Videos"],
};

static IT: Strings = Strings {
    places: "RISORSE",
    drives: "UNITÀ",
    filter: "Filtra…",
    col_name: "Nome",
    col_type: "Tipo",
    col_modified: "Modificato",
    col_size: "Dimensione",
    folder: "Cartella",
    file: "File",
    file_of_type: |ext| format!("File {ext}"),
    counts: |dirs, files| format!("{}, {} file", plural(dirs, "cartella", "cartelle"), files),
    selected: "selezionato",
    settings: "Impostazioni",
    language: "Lingua",
    open: "Apri",
    open_new_tab: "Apri in una nuova scheda",
    open_new_window: "Apri in una nuova finestra",
    copy_path: "Copia percorso",
    properties: "Proprietà",
    refresh: "Aggiorna",
    show_hidden: "Mostra file nascosti",
    more_options: "Mostra altre opzioni",
    known_folders: ["Home", "Desktop", "Download", "Documenti", "Immagini", "Musica", "Video"],
};
