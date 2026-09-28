//! UI strings. The language is a GPUI global that views read while rendering,
//! so `set_language` re-translates every open window in place.

use gpui_kit::*;

use crate::shell::KnownFolder;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
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

/// Observe it with `cx.observe_global_in` to update state that caches strings (input placeholders).
pub struct CurrentLanguage(Language);

impl Global for CurrentLanguage {}

pub fn language(cx: &App) -> Language {
    cx.try_global::<CurrentLanguage>().map(|l| l.0).unwrap_or_default()
}

pub fn set_language(language: Language, cx: &mut App) {
    cx.set_global(CurrentLanguage(language));
    gpui_kit::component::set_locale(language.code());
    cx.refresh_windows();
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
    known_folders: ["Home", "Desktop", "Download", "Documenti", "Immagini", "Musica", "Video"],
};
