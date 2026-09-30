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

/// Applies the language stored in `Settings` to gpui-component's own strings, and to
/// the text under dragged files. Call once at startup.
pub fn init(cx: &App) {
    apply(language(cx));
}

pub fn set_language(language: Language, cx: &mut App) {
    apply(language);
    Settings::update(cx, |s| s.language = language);
}

fn apply(language: Language) {
    gpui_kit::component::set_locale(language.code());
    let s = language.strings();
    crate::dnd::set_labels(crate::dnd::DropLabels {
        move_to: s.drop_move,
        copy_to: s.drop_copy,
        add_to: s.drop_add,
        favorites: s.favorites_name,
        recycle_bin: s.recycle_bin,
    });
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
    pub hide_hidden: &'static str,
    pub keep_sort: &'static str,
    pub forget_sort: &'static str,
    pub sort_kept: &'static str,
    pub sort_tip: &'static str,
    pub sort_tip_hint: &'static str,
    pub keep_view: &'static str,
    pub forget_view: &'static str,
    pub view_kept: &'static str,
    pub view_tip: &'static str,
    pub view_tip_hint: &'static str,
    /// Snackbar button that keeps the sorting or view for the folder.
    pub keep: &'static str,
    pub dismiss: &'static str,
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
    pub menu: &'static str,
    pub about: &'static str,
    /// "Version 0.1.0".
    pub version: fn(&str) -> String,
    pub about_description: &'static str,
    pub check_updates: &'static str,
    pub checking: &'static str,
    pub up_to_date: &'static str,
    /// "Version 0.2.0 is available".
    pub update_available: fn(&str) -> String,
    pub open_release: &'static str,
    pub update_failed: &'static str,
    pub cut: &'static str,
    pub copy: &'static str,
    pub paste: &'static str,
    pub paste_into: &'static str,
    pub rename: &'static str,
    pub delete: &'static str,
    /// Also the name of new folders, as Explorer names them.
    pub new_folder: &'static str,
    /// "3 items selected".
    pub items_selected: fn(usize) -> String,
    pub invalid_name: &'static str,
    pub clipboard_failed: &'static str,
    pub operation_failed: &'static str,
    /// Under dragged files; `%1` is the folder.
    pub drop_move: &'static str,
    pub drop_copy: &'static str,
    /// Under folders dragged onto the favorites; `%1` is `favorites_name`.
    pub drop_add: &'static str,
    pub favorites_name: &'static str,
    /// On the favorites heading while folders are dragged.
    pub favorites_drop: &'static str,
    pub shortcut: &'static str,
    pub internet_shortcut: &'static str,
    pub open_file_location: &'static str,
    pub files_section: &'static str,
    pub show_extensions: &'static str,
    pub recycle_bin: &'static str,
    pub restore: &'static str,
    pub restore_all: &'static str,
    pub empty_bin: &'static str,
    pub delete_permanently: &'static str,
    pub col_origin: &'static str,
    pub col_deleted: &'static str,
    pub purge_title: &'static str,
    /// "3 items will be deleted for good."
    pub purge_message: fn(usize) -> String,
    pub cancel: &'static str,
    pub bin_empty: &'static str,
    pub view: &'static str,
    pub sort_by: &'static str,
    pub ascending: &'static str,
    pub descending: &'static str,
    /// Cover flow: "12 of 340".
    pub item_of: fn(usize, usize) -> String,
    pub no_media: &'static str,
    /// Key names as printed on keyboards.
    pub key_enter: &'static str,
    pub key_delete: &'static str,
    media_filters: [&'static str; 3],
    view_modes: [&'static str; 7],
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

    pub fn media_filter(&self, filter: crate::fs::MediaFilter) -> &'static str {
        self.media_filters[filter as usize]
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
    hide_hidden: "Hide hidden files",
    keep_sort: "Keep this sorting for this folder",
    forget_sort: "Forget this folder's sorting",
    sort_kept: "Sorting kept for this folder",
    sort_tip: "Keep this sorting for this folder?",
    sort_tip_hint: "You can always do it by right-clicking a column header.",
    keep_view: "Keep this view for this folder",
    forget_view: "Forget this folder's view",
    view_kept: "View kept for this folder",
    view_tip: "Keep this view for this folder?",
    view_tip_hint: "You can always do it by right-clicking the view button, bottom right.",
    keep: "Keep",
    dismiss: "Dismiss",
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
    menu: "Menu",
    about: "About",
    version: |v| format!("Version {v}"),
    about_description: "A lightweight, fast file manager for Windows",
    check_updates: "Check for updates",
    checking: "Checking…",
    up_to_date: "You're on the latest version",
    update_available: |v| format!("Version {v} is available"),
    open_release: "Open release page",
    update_failed: "Couldn't reach GitHub",
    cut: "Cut",
    copy: "Copy",
    paste: "Paste",
    paste_into: "Paste into folder",
    rename: "Rename",
    delete: "Delete",
    new_folder: "New folder",
    items_selected: |n| format!("{n} items selected"),
    invalid_name: "A name can't be empty or contain any of these characters: \\ / : * ? \" < > |",
    clipboard_failed: "Couldn't use the clipboard",
    operation_failed: "The operation failed",
    drop_move: "Move to %1",
    drop_copy: "Copy to %1",
    drop_add: "Add to %1",
    favorites_name: "Favorites",
    favorites_drop: "Drop here to add",
    shortcut: "Shortcut",
    internet_shortcut: "Internet shortcut",
    open_file_location: "Open file location",
    files_section: "Files",
    show_extensions: "Show file extensions",
    recycle_bin: "Recycle Bin",
    restore: "Restore",
    restore_all: "Restore all items",
    empty_bin: "Empty Recycle Bin",
    delete_permanently: "Delete permanently",
    col_origin: "Original location",
    col_deleted: "Date deleted",
    purge_title: "Delete permanently?",
    purge_message: |n| if n == 1 { "This item will be deleted for good.".into() } else { format!("These {n} items will be deleted for good.") },
    cancel: "Cancel",
    bin_empty: "The Recycle Bin is empty",
    view: "View",
    sort_by: "Sort by",
    ascending: "Ascending",
    descending: "Descending",
    item_of: |i, n| format!("{i} of {n}"),
    no_media: "No photos or videos here",
    key_enter: "Enter",
    key_delete: "Del",
    media_filters: ["All files", "Photos and videos", "Photos"],
    view_modes: ["Details", "Tree", "List", "M icons", "L icons", "XL icons", "Cover Flow"],
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
    hide_hidden: "Nascondi file nascosti",
    keep_sort: "Mantieni questo ordinamento per questa cartella",
    forget_sort: "Dimentica l'ordinamento di questa cartella",
    sort_kept: "Ordinamento mantenuto per questa cartella",
    sort_tip: "Mantenere questo ordinamento per questa cartella?",
    sort_tip_hint: "Puoi farlo sempre con il tasto destro sull'intestazione di una colonna.",
    keep_view: "Mantieni questa vista per questa cartella",
    forget_view: "Dimentica la vista di questa cartella",
    view_kept: "Vista mantenuta per questa cartella",
    view_tip: "Mantenere questa vista per questa cartella?",
    view_tip_hint: "Puoi farlo sempre con il tasto destro sul pulsante della vista, in basso a destra.",
    keep: "Mantieni",
    dismiss: "Chiudi",
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
    menu: "Menu",
    about: "Informazioni",
    version: |v| format!("Versione {v}"),
    about_description: "Un file manager leggero e veloce per Windows",
    check_updates: "Controlla aggiornamenti",
    checking: "Controllo in corso…",
    up_to_date: "Hai già l'ultima versione",
    update_available: |v| format!("È disponibile la versione {v}"),
    open_release: "Apri pagina della release",
    update_failed: "Impossibile contattare GitHub",
    cut: "Taglia",
    copy: "Copia",
    paste: "Incolla",
    paste_into: "Incolla nella cartella",
    rename: "Rinomina",
    delete: "Elimina",
    new_folder: "Nuova cartella",
    items_selected: |n| format!("{n} elementi selezionati"),
    invalid_name: "Un nome non può essere vuoto né contenere i caratteri: \\ / : * ? \" < > |",
    clipboard_failed: "Impossibile usare gli appunti",
    operation_failed: "L'operazione non è riuscita",
    drop_move: "Sposta in %1",
    drop_copy: "Copia in %1",
    drop_add: "Aggiungi a %1",
    favorites_name: "Preferiti",
    favorites_drop: "Rilascia qui per aggiungere",
    shortcut: "Collegamento",
    internet_shortcut: "Collegamento Internet",
    open_file_location: "Apri percorso file",
    files_section: "File",
    show_extensions: "Mostra estensioni dei file",
    recycle_bin: "Cestino",
    restore: "Ripristina",
    restore_all: "Ripristina tutti gli elementi",
    empty_bin: "Svuota cestino",
    delete_permanently: "Elimina definitivamente",
    col_origin: "Percorso originale",
    col_deleted: "Data eliminazione",
    purge_title: "Eliminare definitivamente?",
    purge_message: |n| if n == 1 { "L'elemento verrà eliminato definitivamente.".into() } else { format!("{n} elementi verranno eliminati definitivamente.") },
    cancel: "Annulla",
    bin_empty: "Il cestino è vuoto",
    view: "Visualizza",
    sort_by: "Ordina per",
    ascending: "Crescente",
    descending: "Decrescente",
    item_of: |i, n| format!("{i} di {n}"),
    no_media: "Nessuna foto o video qui",
    key_enter: "Invio",
    key_delete: "Canc",
    media_filters: ["Tutti i file", "Foto e video", "Solo foto"],
    view_modes: ["Dettagli", "Albero", "Elenco", "Icone M", "Icone L", "Icone XL", "Cover Flow"],
    themes: ["Scuro", "Attenuato", "Chiaro"],
    accents: ["Blu", "Viola", "Verde acqua", "Ambra", "Rosa"],
    known_folders: ["Home", "Desktop", "Download", "Documenti", "Immagini", "Musica", "Video"],
};
