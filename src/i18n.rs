//! UI strings. The language lives in `Settings`, which views read while rendering,
//! so `set_language` re-translates every open window in place.

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::app::ViewMode;
use crate::settings::Settings;
use crate::shell::KnownFolder;
use crate::theme::{Accent, ThemeChoice};

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
    pub favorites: &'static str,
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
    pub add_favorite: &'static str,
    pub add_folder_favorite: &'static str,
    pub remove_favorite: &'static str,
    pub toggle_sidebar: &'static str,
    pub folders: &'static str,
    pub files: &'static str,
    pub preview_pane: &'static str,
    pub no_selection: &'static str,
    pub view_hint: &'static str,
    pub drag_reorder: &'static str,
    pub drag_detach: &'static str,
    pub drag_move_to: &'static str,
    pub drag_move_window: &'static str,
    pub drag_cancel: &'static str,
    pub theme: &'static str,
    pub accent: &'static str,
    view_modes: [&'static str; 6],
    themes: [&'static str; 3],
    accents: [&'static str; 5],
    known_folders: [&'static str; KnownFolder::COUNT],
}

impl Strings {
    pub fn known_folder(&self, folder: KnownFolder) -> &'static str {
        self.known_folders[folder as usize]
    }

    pub fn view_mode(&self, mode: ViewMode) -> &'static str {
        self.view_modes[mode as usize]
    }

    pub fn theme_name(&self, theme: ThemeChoice) -> &'static str {
        self.themes[theme as usize]
    }

    pub fn accent_name(&self, accent: Accent) -> &'static str {
        self.accents[accent as usize]
    }
}

static EN: Strings = Strings {
    favorites: "FAVORITES",
    drives: "DRIVES",
    filter: "Filter…",
    col_name: "Name",
    col_type: "Type",
    col_modified: "Modified",
    col_size: "Size",
    folder: "Folder",
    file: "File",
    file_of_type: |ext| format!("{ext} file"),
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
    add_favorite: "Add to favorites",
    add_folder_favorite: "Add this folder to favorites",
    remove_favorite: "Remove from favorites",
    toggle_sidebar: "Show or hide the sidebar (Ctrl+B)",
    folders: "Folders",
    files: "Files",
    preview_pane: "Preview pane (Alt+P)",
    no_selection: "Select a file to preview it",
    view_hint: "Ctrl + mouse wheel",
    drag_reorder: "Release to move the tab here",
    drag_detach: "Release to open it in a new window",
    drag_move_to: "Release to move it to this window",
    drag_move_window: "Release to move the window here",
    drag_cancel: "Drag out of the window to detach the tab",
    theme: "Theme",
    accent: "Accent color",
    view_modes: ["Details", "Tree", "List", "M icons", "L icons", "XL icons"],
    themes: ["Dark", "Dimmed", "Light"],
    accents: ["Blue", "Violet", "Teal", "Amber", "Rose"],
    known_folders: ["Home", "Desktop", "Downloads", "Documents", "Pictures", "Music", "Videos"],
};

static IT: Strings = Strings {
    favorites: "PREFERITI",
    drives: "UNITÀ",
    filter: "Filtra…",
    col_name: "Nome",
    col_type: "Tipo",
    col_modified: "Modificato",
    col_size: "Dimensione",
    folder: "Cartella",
    file: "File",
    file_of_type: |ext| format!("File {ext}"),
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
    add_favorite: "Aggiungi ai preferiti",
    add_folder_favorite: "Aggiungi questa cartella ai preferiti",
    remove_favorite: "Rimuovi dai preferiti",
    toggle_sidebar: "Mostra o nascondi la barra laterale (Ctrl+B)",
    folders: "Cartelle",
    files: "File",
    preview_pane: "Riquadro anteprima (Alt+P)",
    no_selection: "Seleziona un file per vederne l'anteprima",
    view_hint: "Ctrl + rotellina",
    drag_reorder: "Rilascia per spostare qui la scheda",
    drag_detach: "Rilascia per aprirla in una nuova finestra",
    drag_move_to: "Rilascia per spostarla in questa finestra",
    drag_move_window: "Rilascia per spostare qui la finestra",
    drag_cancel: "Trascina fuori dalla finestra per staccare la scheda",
    theme: "Tema",
    accent: "Colore di accento",
    view_modes: ["Dettagli", "Albero", "Elenco", "Icone M", "Icone L", "Icone XL"],
    themes: ["Scuro", "Attenuato", "Chiaro"],
    accents: ["Blu", "Viola", "Verde acqua", "Ambra", "Rosa"],
    known_folders: ["Home", "Desktop", "Download", "Documenti", "Immagini", "Musica", "Video"],
};
