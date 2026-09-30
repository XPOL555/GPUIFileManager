//! Table delegate: owns the current folder snapshot, the filtered/sorted view of it and
//! the selection. In tree mode it also holds the children of expanded folders. The
//! grid views read their items and the selection from here too.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, ElementExt as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::dnd::{self, SharedZones, SpotKey};
use crate::file_ops::CutFiles;
use crate::fs::{self, Entry, MediaFilter, Sort, SortKey};
use crate::i18n;
use crate::icons::{self, IconSize};
use crate::settings::Settings;

/// Asks the owner to expand (`true`) or collapse a folder of the tree.
pub type ToggleFolder = Rc<dyn Fn(PathBuf, bool, &mut Window, &mut App)>;

/// A column header was clicked: sort by it, or open the header menu at a window position.
pub enum HeaderClick {
    Sort(SortKey),
    Menu(Point<Pixels>),
}

/// Handles header clicks. The owner keeps the sorting (per tab, maybe kept for the
/// folder), so the table's own sort cycle and column selection are off.
pub type OnHeader = Rc<dyn Fn(HeaderClick, &mut Window, &mut App)>;

/// Mouse input on a row. The table's own row selection is off: the owner selects,
/// the same way for the table and the grid views.
pub enum RowEvent {
    /// Left button down.
    Press { row: usize, modifiers: Modifiers },
    /// Left button released without dragging.
    Click { row: usize, modifiers: Modifiers, count: usize },
    /// Right button down, at a window position.
    Menu { row: usize, position: Point<Pixels>, shift: bool },
    Middle { row: usize },
}

pub type OnRow = Rc<dyn Fn(RowEvent, &mut Window, &mut App)>;

/// Identity of a row that survives re-sorting and filtering: the listing it comes from
/// (`None`: the listed folder, else an expanded one) and its index there.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    parent: Option<Arc<Path>>,
    index: u32,
}

#[derive(Clone)]
struct Row {
    depth: u16,
    key: Key,
}

pub struct FileTable {
    entries: Vec<Entry>,
    /// Listings of expanded folders (tree mode only). Collapsing drops them, so memory
    /// follows what is open on screen, never the whole subtree.
    children: HashMap<PathBuf, Vec<Entry>>,
    /// Folders whose listing is on its way.
    expanding: HashSet<PathBuf>,
    rows: Vec<Row>,
    filter: String,
    show_hidden: bool,
    /// Only photos and videos (the cover flow view).
    media: MediaFilter,
    sort: Sort,
    /// The listed folder, to tell whether its sorting is the one kept for it.
    folder: PathBuf,
    tree: bool,
    on_toggle: Option<ToggleFolder>,
    on_header: Option<OnHeader>,
    on_row: Option<OnRow>,
    pub loading: bool,
    columns: Vec<Column>,
    /// Selected rows; only rows on screen (filtered out or collapsed ones drop out).
    selected: HashSet<Key>,
    /// Where keyboard moves start from.
    cursor: Option<Key>,
    /// The fixed end of Shift ranges.
    anchor: Option<Key>,
    /// The item being renamed, and its editor.
    pub renaming: Option<(PathBuf, Entity<InputState>)>,
    /// The folder a file drag hovers.
    pub drop_hover: Option<PathBuf>,
    zones: Option<SharedZones>,
}

const COLUMNS: [SortKey; 4] = [SortKey::Name, SortKey::Type, SortKey::Modified, SortKey::Size];
const INDENT: f32 = 18.;

impl FileTable {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            children: HashMap::new(),
            expanding: HashSet::new(),
            rows: Vec::new(),
            filter: String::new(),
            show_hidden: true,
            media: MediaFilter::All,
            sort: Sort::default(),
            folder: PathBuf::new(),
            tree: false,
            on_toggle: None,
            on_header: None,
            on_row: None,
            loading: false,
            // Names are filled in per render from the current language, see `column`.
            // Not `sortable`: headers sort through `on_header`, see `render_th`.
            columns: vec![
                Column::new("name", "").width(px(380.)).min_width(px(120.)),
                Column::new("type", "").width(px(110.)),
                Column::new("modified", "").width(px(150.)),
                Column::new("size", "").width(px(110.)).text_right(),
            ],
            selected: HashSet::new(),
            cursor: None,
            anchor: None,
            renaming: None,
            drop_hover: None,
            zones: None,
        }
    }

    pub fn set_on_toggle(&mut self, on_toggle: ToggleFolder) {
        self.on_toggle = Some(on_toggle);
    }

    pub fn set_on_header(&mut self, on_header: OnHeader) {
        self.on_header = Some(on_header);
    }

    pub fn set_on_row(&mut self, on_row: OnRow) {
        self.on_row = Some(on_row);
    }

    /// Folder rows record where files can be dropped on them.
    pub fn set_zones(&mut self, zones: SharedZones) {
        self.zones = Some(zones);
    }

    pub fn set_folder(&mut self, folder: PathBuf) {
        self.folder = folder;
    }

    pub fn set_sort(&mut self, sort: Sort) {
        if self.sort != sort {
            self.sort = sort;
            self.rebuild();
        }
    }

    /// Widths the user dragged the columns to, so `TableState::refresh` keeps them.
    pub fn set_widths(&mut self, widths: &[Pixels]) {
        for (column, &width) in self.columns.iter_mut().zip(widths) {
            column.width = width;
        }
    }

    /// A new folder listing; tree expansions and the selection belong to the previous
    /// one and are dropped.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.children.clear();
        self.expanding.clear();
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.rebuild();
    }

    /// A fresh listing of the same folder, and of expanded folders: the selection, the
    /// cursor and the expansions stay on the items that are still there.
    pub fn replace_listing(&mut self, entries: Vec<Entry>, children: Vec<(PathBuf, Vec<Entry>)>) {
        let selected: HashSet<PathBuf> = self.selected_paths().into_iter().collect();
        let (cursor, anchor) = (self.key_path(self.cursor.as_ref()), self.key_path(self.anchor.as_ref()));
        self.entries = entries;
        for (path, list) in children {
            // Collapsed while it was listed: stays collapsed.
            if let Some(old) = self.children.get_mut(&path) {
                *old = list;
            }
        }
        // Expanded folders that are gone.
        let mut folders: HashSet<&Path> = HashSet::new();
        for list in std::iter::once(&self.entries).chain(self.children.values()) {
            folders.extend(list.iter().filter(|e| e.is_dir).map(|e| e.path.as_path()));
        }
        let gone: Vec<PathBuf> = self.children.keys().filter(|p| !folders.contains(p.as_path())).cloned().collect();
        for path in gone {
            self.children.remove(&path);
        }
        // Keys point into the old listings.
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.rebuild();
        let (mut restored, mut new_cursor, mut new_anchor) = (HashSet::new(), None, None);
        for row in &self.rows {
            let Some(e) = self.entry_of(&row.key) else { continue };
            if selected.contains(&e.path) {
                restored.insert(row.key.clone());
            }
            if cursor.as_ref() == Some(&e.path) {
                new_cursor = Some(row.key.clone());
            }
            if anchor.as_ref() == Some(&e.path) {
                new_anchor = Some(row.key.clone());
            }
        }
        self.selected = restored;
        self.cursor = new_cursor;
        self.anchor = new_anchor;
    }

    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_lowercase();
        self.rebuild();
    }

    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn media_filter(&self) -> MediaFilter {
        self.media
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        if self.show_hidden != show {
            self.show_hidden = show;
            self.rebuild();
        }
    }

    pub fn set_media_filter(&mut self, media: MediaFilter) {
        if self.media != media {
            self.media = media;
            self.rebuild();
        }
    }

    /// Tree mode shows expanders; leaving it collapses everything.
    pub fn set_tree(&mut self, tree: bool) {
        if self.tree != tree {
            self.tree = tree;
            self.children.clear();
            self.expanding.clear();
            self.rebuild();
        }
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.children.contains_key(path) || self.expanding.contains(path)
    }

    /// Expanded folders whose listing is here.
    pub fn expanded(&self) -> Vec<PathBuf> {
        self.children.keys().cloned().collect()
    }

    pub fn mark_expanding(&mut self, path: PathBuf) {
        self.expanding.insert(path);
    }

    /// Children of an expanded folder arrived. Ignored if it was collapsed meanwhile.
    pub fn set_children(&mut self, path: PathBuf, entries: Vec<Entry>) {
        if self.expanding.remove(&path) {
            self.children.insert(path, entries);
            self.rebuild();
        }
    }

    /// Collapses `path` and everything expanded below it.
    pub fn collapse(&mut self, path: &Path) {
        self.children.retain(|p, _| !p.starts_with(path));
        self.expanding.retain(|p| !p.starts_with(path));
        self.rebuild();
    }

    /// The listed folder's items, shown or not.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The Recycle Bin is listed: other columns, no folders to drop into.
    fn is_bin(&self) -> bool {
        fs::is_recycle_bin(&self.folder)
    }

    fn entry_of(&self, key: &Key) -> Option<&Entry> {
        let list = match &key.parent {
            None => &self.entries,
            Some(p) => self.children.get(p.as_ref())?,
        };
        list.get(key.index as usize)
    }

    fn key_path(&self, key: Option<&Key>) -> Option<PathBuf> {
        key.and_then(|k| self.entry_of(k)).map(|e| e.path.clone())
    }

    pub fn entry(&self, row: usize) -> Option<&Entry> {
        self.entry_of(&self.rows.get(row)?.key)
    }

    /// Number of rows, including the children of expanded folders.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn depth(&self, row: usize) -> usize {
        self.rows.get(row).map_or(0, |r| r.depth as usize)
    }

    /// The row of the folder a tree row is in; `None` at the top level.
    pub fn parent_row(&self, row: usize) -> Option<usize> {
        let parent = self.rows.get(row)?.key.parent.as_ref()?;
        (0..row).rev().find(|&r| self.entry(r).is_some_and(|e| e.path.as_path() == parent.as_ref()))
    }

    /// Top-level row with this name.
    pub fn row_of(&self, name: &str) -> Option<usize> {
        (0..self.rows.len()).find(|&r| self.rows[r].depth == 0 && self.entry(r).is_some_and(|e| e.name.as_ref() == name))
    }

    /// Visible folders and files of the listed folder (not counting expanded children).
    pub fn summary(&self) -> (usize, usize) {
        let (mut dirs, mut files) = (0, 0);
        for (r, row) in self.rows.iter().enumerate() {
            if row.depth == 0 {
                match self.entry(r) {
                    Some(e) if e.is_dir => dirs += 1,
                    Some(_) => files += 1,
                    None => {}
                }
            }
        }
        (dirs, files)
    }

    // ---- selection ----------------------------------------------------------

    fn key(&self, row: usize) -> Option<Key> {
        self.rows.get(row).map(|r| r.key.clone())
    }

    fn row_of_key(&self, key: &Key) -> Option<usize> {
        self.rows.iter().position(|r| &r.key == key)
    }

    pub fn is_selected(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(|r| self.selected.contains(&r.key))
    }

    pub fn selection_len(&self) -> usize {
        self.selected.len()
    }

    /// Selected rows, top to bottom.
    pub fn selected_rows(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            return Vec::new();
        }
        (0..self.rows.len()).filter(|&r| self.is_selected(r)).collect()
    }

    pub fn selected_entries(&self) -> Vec<Entry> {
        self.selected_rows().into_iter().filter_map(|r| self.entry(r).cloned()).collect()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_rows().into_iter().filter_map(|r| self.entry(r).map(|e| e.path.clone())).collect()
    }

    /// The only selected item.
    pub fn single_selected(&self) -> Option<&Entry> {
        match self.selected.len() {
            1 => self.selected.iter().next().and_then(|k| self.entry_of(k)),
            _ => None,
        }
    }

    /// Total size of the selected files.
    pub fn selected_size(&self) -> u64 {
        self.selected.iter().filter_map(|k| self.entry_of(k)).filter(|e| !e.is_dir).map(|e| e.size).sum()
    }

    pub fn cursor(&self) -> Option<usize> {
        self.row_of_key(self.cursor.as_ref()?)
    }

    pub fn is_cursor(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(|r| self.cursor.as_ref() == Some(&r.key))
    }

    /// Selects `row` alone; it becomes the cursor and the anchor.
    pub fn select_only(&mut self, row: usize) {
        let Some(key) = self.key(row) else { return };
        self.selected.clear();
        self.selected.insert(key.clone());
        self.cursor = Some(key.clone());
        self.anchor = Some(key);
    }

    /// Ctrl + click: adds or removes `row`.
    pub fn toggle(&mut self, row: usize) {
        let Some(key) = self.key(row) else { return };
        if !self.selected.remove(&key) {
            self.selected.insert(key.clone());
        }
        self.cursor = Some(key.clone());
        self.anchor = Some(key);
    }

    /// Shift + click or arrow: the rows from the anchor to `row`, added to the
    /// selection with Ctrl held (`additive`), else in its place.
    pub fn select_range(&mut self, row: usize, additive: bool) {
        let Some(key) = self.key(row) else { return };
        let from = self.anchor.as_ref().and_then(|a| self.row_of_key(a)).unwrap_or(row);
        if !additive {
            self.selected.clear();
        }
        let (lo, hi) = if from <= row { (from, row) } else { (row, from) };
        for r in lo..=hi {
            self.selected.insert(self.rows[r].key.clone());
        }
        if self.anchor.is_none() {
            self.anchor = Some(key.clone());
        }
        self.cursor = Some(key);
    }

    /// Ctrl + arrow: moves the cursor, leaving the selection alone.
    pub fn move_cursor(&mut self, row: usize) {
        if let Some(key) = self.key(row) {
            self.cursor = Some(key.clone());
            self.anchor = Some(key);
        }
    }

    /// Selects exactly these rows (a rubber band); the last one gets the cursor.
    pub fn select_rows(&mut self, rows: impl IntoIterator<Item = usize>) {
        self.selected.clear();
        let mut last = None;
        for row in rows {
            if let Some(key) = self.key(row) {
                self.selected.insert(key.clone());
                last = Some(key);
            }
        }
        if last.is_some() {
            self.cursor = last.clone();
            self.anchor = last;
        }
    }

    pub fn select_all(&mut self) {
        self.selected = self.rows.iter().map(|r| r.key.clone()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Selects the rows of these items (those that are on screen); the first one gets
    /// the cursor. Returns its row.
    pub fn select_paths(&mut self, paths: &[PathBuf]) -> Option<usize> {
        let wanted: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
        self.selected.clear();
        let mut first = None;
        for (r, row) in self.rows.iter().enumerate() {
            if self.entry_of(&row.key).is_some_and(|e| wanted.contains(e.path.as_path())) {
                self.selected.insert(row.key.clone());
                first.get_or_insert(r);
            }
        }
        if let Some(r) = first {
            self.cursor = self.key(r);
            self.anchor = self.cursor.clone();
        }
        first
    }

    /// Drops what is no longer on screen from the selection.
    fn prune(&mut self) {
        if self.selected.is_empty() && self.cursor.is_none() {
            return;
        }
        let shown: HashSet<&Key> = self.rows.iter().map(|r| &r.key).collect();
        self.selected.retain(|k| shown.contains(k));
        if self.cursor.as_ref().is_some_and(|k| !shown.contains(k)) {
            self.cursor = None;
        }
        if self.anchor.as_ref().is_some_and(|k| !shown.contains(k)) {
            self.anchor = None;
        }
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::with_capacity(self.entries.len());
        self.push_level(&mut rows, None, 0);
        self.rows = rows;
        self.prune();
    }

    /// Appends the sorted, filtered rows of one folder and, recursively, of its expanded
    /// subfolders. A folder that does not match the filter stays when a descendant does.
    fn push_level(&self, rows: &mut Vec<Row>, parent: Option<Arc<Path>>, depth: u16) -> bool {
        let list = match &parent {
            None => &self.entries,
            Some(p) => match self.children.get(p.as_ref()) {
                Some(list) => list,
                None => return false,
            },
        };
        let mut order: Vec<u32> = (0..list.len() as u32)
            .filter(|&i| {
                let e = &list[i as usize];
                (self.show_hidden || !e.hidden) && self.media.admits(e)
            })
            .collect();
        let (key, desc) = (self.sort.key, self.sort.descending);
        order.sort_by(|&a, &b| fs::compare(&list[a as usize], &list[b as usize], key, desc));

        let mut any = false;
        for i in order {
            let e = &list[i as usize];
            let mark = rows.len();
            rows.push(Row { depth, key: Key { parent: parent.clone(), index: i } });
            let mut keep = self.filter.is_empty() || e.name.to_lowercase().contains(&self.filter);
            if self.children.contains_key(&e.path) && self.push_level(rows, Some(Arc::from(e.path.as_path())), depth + 1) {
                keep = true;
            }
            if keep {
                any = true;
            } else {
                rows.truncate(mark);
            }
        }
        any
    }

    fn render_name(&self, row_ix: usize, e: &Entry, cx: &mut Context<TableState<Self>>) -> AnyElement {
        let depth = self.depth(row_ix);
        let expander = self.tree.then(|| {
            if !e.is_dir {
                return div().w(px(16.)).flex_shrink_0().into_any_element();
            }
            let expanded = self.is_expanded(&e.path);
            let (path, on_toggle) = (e.path.clone(), self.on_toggle.clone());
            div()
                .id(("expander", row_ix))
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size(px(16.))
                .rounded(px(3.))
                .hover(|d| d.bg(cx.theme().accent))
                .child(
                    Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                // Don't let the press select the row or start a double click.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    if let Some(toggle) = &on_toggle {
                        toggle(path.clone(), !expanded, window, cx);
                    }
                })
                .into_any_element()
        });
        h_flex()
            .gap_1p5()
            .overflow_hidden()
            .pl(px(depth as f32 * INDENT))
            .when(e.hidden, |d| d.opacity(0.55))
            .children(expander)
            .child(icons::entry_icon(e, IconSize::Small, px(16.), cx))
            .child(name_or_editor(e, self.renaming.as_ref(), Settings::get(cx).show_extensions))
            .into_any_element()
    }
}

/// The name, or the rename editor when the item is being renamed. Presses in the editor
/// stay there: they must not select rows or start drags.
pub fn name_or_editor(e: &Entry, renaming: Option<&(PathBuf, Entity<InputState>)>, show_extensions: bool) -> AnyElement {
    match renaming {
        Some((path, input)) if *path == e.path => div()
            .flex_1()
            .min_w_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(Input::new(input).xsmall())
            .into_any_element(),
        _ => div().truncate().child(e.shown_name(show_extensions)).into_any_element(),
    }
}

/// The selection, cursor and drop highlight of a row or tile, drawn under its content.
pub fn highlight(selected: bool, cursor: bool, drop: bool, radius: Pixels, cx: &App) -> Option<Div> {
    let theme = cx.theme();
    (selected || cursor || drop).then(|| {
        div()
            .absolute()
            .inset_0()
            .rounded(radius)
            .when(selected, |d| d.bg(theme.table_active))
            .when(cursor, |d| d.border_1().border_color(theme.table_active_border))
            .when(drop, |d| d.bg(theme.drop_target).border_1().border_color(theme.drag_border))
    })
}

impl TableDelegate for FileTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, cx: &App) -> Column {
        let mut column = self.columns[col_ix].clone();
        column.name = column_name(COLUMNS[col_ix], self.is_bin(), cx).into();
        column
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }

    /// An empty folder shows the table's inbox; the Recycle Bin says it is empty, and a
    /// view filtered to photos that it holds none.
    fn render_empty(&mut self, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let s = i18n::t(cx);
        let muted = cx.theme().muted_foreground;
        let (icon, text) = if self.is_bin() {
            (gpui_kit::assets::IconName::Trash, Some(s.bin_empty))
        } else if self.media != MediaFilter::All && !self.entries.is_empty() {
            (gpui_kit::assets::IconName::Images, Some(s.no_media))
        } else {
            (gpui_kit::assets::IconName::Inbox, None)
        };
        v_flex()
            .size_full()
            .gap_2()
            .items_center()
            .justify_center()
            .text_color(muted.opacity(0.7))
            .child(Icon::new(icon).size_10())
            .children(text.map(|text| div().text_sm().child(text)))
            .into_any_element()
    }

    /// The column name, with an arrow on the sorted column. A click sorts, a right click
    /// opens the header menu. Bold with a pin when the sorting is the one kept for the folder.
    fn render_th(&mut self, col_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let key = COLUMNS[col_ix];
        let sorted = self.sort.key == key;
        let kept = sorted && Settings::get(cx).folder(&self.folder).sort == Some(self.sort);
        let theme = cx.theme();
        let arrow = sorted.then(|| {
            Icon::new(if self.sort.descending { IconName::ChevronDown } else { IconName::ChevronUp })
                .xsmall()
                .text_color(if kept { theme.primary } else { theme.muted_foreground })
        });
        let pin = kept.then(|| Icon::new(gpui_kit::assets::IconName::Pin).xsmall().text_color(theme.primary));
        let (on_sort, on_menu) = (self.on_header.clone(), self.on_header.clone());
        let tip = i18n::t(cx).sort_kept;
        h_flex()
            .id(("th", col_ix))
            .size_full()
            .gap_1()
            .cursor_pointer()
            // Right-aligned column: the arrow goes left of the name.
            .when(key == SortKey::Size, |d| d.flex_row_reverse())
            .when(kept, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(theme.foreground))
            .child(div().truncate().child(column_name(key, self.is_bin(), cx)))
            .children(arrow)
            .children(pin)
            // Not a press on the view's background, which clears the selection.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                if let Some(on_header) = &on_sort {
                    on_header(HeaderClick::Sort(key), window, cx);
                }
            })
            .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
                // Not the folder background menu of the view around the table.
                cx.stop_propagation();
                if let Some(on_header) = &on_menu {
                    on_header(HeaderClick::Menu(ev.position), window, cx);
                }
            })
            .when(kept, |d| d.tooltip(move |window, cx| Tooltip::new(tip).build(window, cx)))
    }

    /// The row's selection and drop highlight, and its mouse input, which goes to the owner.
    fn render_tr(&mut self, row_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> Stateful<Div> {
        let tr = div().id(("row", row_ix));
        let Some(e) = self.entry(row_ix) else { return tr };
        // Folders take drops; deleted ones (in the Recycle Bin) don't.
        let (path, is_dir) = (e.path.clone(), e.is_dir && e.origin.is_none());
        let selected = self.is_selected(row_ix);
        // The cursor is only worth marking when it adds something to the selection.
        let cursor = self.is_cursor(row_ix) && (!selected || self.selected.len() > 1);
        let drop = is_dir && self.drop_hover.as_ref() == Some(&path);
        let cut = cx.try_global::<CutFiles>().is_some_and(|c| c.paths.contains(&path));
        let on_row = self.on_row.clone();
        let emit = move |event: RowEvent, window: &mut Window, cx: &mut App| {
            if let Some(on_row) = &on_row {
                on_row(event, window, cx);
            }
        };
        let (press, click, menu, middle) = (emit.clone(), emit.clone(), emit.clone(), emit);
        tr.when(cut, |d| d.opacity(0.5))
            .children(highlight(selected, cursor, drop, px(0.), cx))
            .when_some(self.zones.as_ref().filter(|_| is_dir), |d, zones| {
                d.on_prepaint(dnd::zone(zones, SpotKey::Item(path.clone()), path, 2))
            })
            .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                cx.stop_propagation();
                press(RowEvent::Press { row: row_ix, modifiers: ev.modifiers }, window, cx);
            })
            .on_click(move |ev, window, cx| {
                cx.stop_propagation();
                click(RowEvent::Click { row: row_ix, modifiers: ev.modifiers(), count: ev.click_count() }, window, cx);
            })
            .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
                cx.stop_propagation();
                menu(RowEvent::Menu { row: row_ix, position: ev.position, shift: ev.modifiers.shift }, window, cx);
            })
            .on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                cx.stop_propagation();
                middle(RowEvent::Middle { row: row_ix }, window, cx);
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(e) = self.entry(row_ix).cloned() else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let text = match COLUMNS[col_ix] {
            SortKey::Name => return self.render_name(row_ix, &e, cx),
            // In the Recycle Bin: where the item was deleted from.
            SortKey::Type if e.origin.is_some() => e.origin.as_deref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            SortKey::Type => type_label(&e, cx),
            SortKey::Modified => fs::format_time(e.modified),
            SortKey::Size if e.is_dir => String::new(),
            SortKey::Size => fs::format_size(e.size),
        };
        div().truncate().text_color(muted).child(text).into_any_element()
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.entry(row_ix)
            .map(|e| match COLUMNS[col_ix] {
                SortKey::Name => e.name.to_string(),
                SortKey::Type => e.ext.to_string(),
                SortKey::Modified => fs::format_time(e.modified),
                SortKey::Size => e.size.to_string(),
            })
            .unwrap_or_default()
    }
}


/// Column names; the Recycle Bin shows where and when items were deleted instead of
/// their type and their date.
fn column_name(key: SortKey, bin: bool, cx: &App) -> &'static str {
    let s = i18n::t(cx);
    match key {
        SortKey::Name => s.col_name,
        SortKey::Type if bin => s.col_origin,
        SortKey::Type => s.col_type,
        SortKey::Modified if bin => s.col_deleted,
        SortKey::Modified => s.col_modified,
        SortKey::Size => s.col_size,
    }
}

/// "Folder", "File", "Shortcut" or "PDF file", in the current language.
pub fn type_label(e: &Entry, cx: &App) -> String {
    let s = i18n::t(cx);
    if e.is_dir {
        s.folder.to_string()
    } else if e.ext == "lnk" {
        s.shortcut.to_string()
    } else if e.ext == "url" {
        s.internet_shortcut.to_string()
    } else if e.ext.is_empty() {
        s.file.to_string()
    } else {
        (s.file_of_type)(&e.ext.to_uppercase())
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that glob includes gpui_kit's `test` macro, which shadows `#[test]`.
    use super::FileTable;
    use crate::fs::Entry;
    use std::path::{Path, PathBuf};

    fn entry(path: &str, is_dir: bool) -> Entry {
        let path = PathBuf::from(path);
        Entry {
            name: path.file_name().unwrap().to_string_lossy().into_owned().into(),
            path,
            is_dir,
            hidden: false,
            size: 0,
            modified: None,
            ext: Default::default(),
            attrs: 0,
            origin: None,
        }
    }

    fn names(t: &FileTable) -> Vec<(usize, String)> {
        (0..t.len()).map(|r| (t.depth(r), t.entry(r).unwrap().name.to_string())).collect()
    }

    fn selected(t: &FileTable) -> Vec<String> {
        t.selected_entries().iter().map(|e| e.name.to_string()).collect()
    }

    #[test]
    fn tree_rows_follow_expansion_and_filter() {
        let mut t = FileTable::new();
        t.set_tree(true);
        t.set_entries(vec![entry(r"C:\r\b", true), entry(r"C:\r\a", true), entry(r"C:\r\z.txt", false)]);
        t.mark_expanding(PathBuf::from(r"C:\r\b"));
        t.set_children(PathBuf::from(r"C:\r\b"), vec![entry(r"C:\r\b\y.txt", false), entry(r"C:\r\b\x", true)]);
        let row = |d: usize, n: &str| (d, n.to_string());
        assert_eq!(names(&t), [row(0, "a"), row(0, "b"), row(1, "x"), row(1, "y.txt"), row(0, "z.txt")]);
        assert_eq!(t.summary(), (2, 1), "expanded children are not counted");
        assert_eq!(t.parent_row(3), Some(1));
        assert_eq!(t.parent_row(1), None);

        // A folder stays when only something inside it matches.
        t.set_filter("y.t");
        assert_eq!(names(&t), [row(0, "b"), row(1, "y.txt")]);

        // Collapsing drops the listing of the folder and of everything below it.
        t.set_filter("");
        t.mark_expanding(PathBuf::from(r"C:\r\b\x"));
        t.set_children(PathBuf::from(r"C:\r\b\x"), vec![entry(r"C:\r\b\x\deep", false)]);
        t.collapse(Path::new(r"C:\r\b"));
        assert_eq!(names(&t), [row(0, "a"), row(0, "b"), row(0, "z.txt")]);
        assert!(!t.is_expanded(Path::new(r"C:\r\b\x")));

        // A listing that arrives after its folder was collapsed is dropped.
        t.set_children(PathBuf::from(r"C:\r\a"), vec![entry(r"C:\r\a\late", false)]);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn selection_follows_items_across_sorting_and_filtering() {
        let mut t = FileTable::new();
        t.set_entries(vec![entry(r"C:\r\c.txt", false), entry(r"C:\r\a.txt", false), entry(r"C:\r\b.txt", false)]);
        // a, b, c
        t.select_only(0);
        t.select_range(2, false);
        assert_eq!(selected(&t), ["a.txt", "b.txt", "c.txt"]);
        t.toggle(1);
        assert_eq!(selected(&t), ["a.txt", "c.txt"]);
        assert_eq!(t.cursor(), Some(1));

        // Sorting moves rows, not the selection.
        t.set_sort(crate::fs::Sort { key: crate::fs::SortKey::Name, descending: true });
        assert_eq!(selected(&t), ["c.txt", "a.txt"]);
        assert_eq!(t.entry(t.cursor().unwrap()).unwrap().name.as_ref(), "b.txt");

        // Rows the filter hides leave the selection.
        t.set_filter("a");
        assert_eq!(selected(&t), ["a.txt"]);
        t.set_filter("");
        assert_eq!(selected(&t), ["a.txt"]);
        assert_eq!(t.cursor(), None);
    }

    #[test]
    fn shift_ranges_grow_from_the_anchor() {
        let mut t = FileTable::new();
        t.set_entries((0..6).map(|i| entry(&format!(r"C:\r\f{i}"), false)).collect());
        t.select_only(2);
        t.select_range(4, false);
        assert_eq!(selected(&t), ["f2", "f3", "f4"]);
        // The anchor stays: going back past it selects the other side.
        t.select_range(0, false);
        assert_eq!(selected(&t), ["f0", "f1", "f2"]);
        // Ctrl + Shift adds a range from a new anchor.
        t.move_cursor(5);
        t.select_range(5, true);
        assert_eq!(selected(&t), ["f0", "f1", "f2", "f5"]);
        t.select_all();
        assert_eq!(t.selection_len(), 6);
    }

    #[test]
    fn a_fresh_listing_keeps_what_is_still_there() {
        let mut t = FileTable::new();
        t.set_tree(true);
        t.set_entries(vec![entry(r"C:\r\a", true), entry(r"C:\r\b.txt", false), entry(r"C:\r\c.txt", false)]);
        t.mark_expanding(PathBuf::from(r"C:\r\a"));
        t.set_children(PathBuf::from(r"C:\r\a"), vec![entry(r"C:\r\a\in.txt", false)]);
        t.select_paths(&[PathBuf::from(r"C:\r\a\in.txt"), PathBuf::from(r"C:\r\c.txt")]);
        assert_eq!(selected(&t), ["in.txt", "c.txt"]);

        // b.txt is gone, new.txt appeared, in the other order.
        t.replace_listing(
            vec![entry(r"C:\r\new.txt", false), entry(r"C:\r\c.txt", false), entry(r"C:\r\a", true)],
            vec![(PathBuf::from(r"C:\r\a"), vec![entry(r"C:\r\a\in.txt", false), entry(r"C:\r\a\more", false)])],
        );
        assert_eq!(selected(&t), ["in.txt", "c.txt"]);
        assert_eq!(t.len(), 5);
        assert_eq!(t.entry(t.cursor().unwrap()).unwrap().name.as_ref(), "in.txt");

        // An expanded folder that disappeared takes its listing with it.
        t.replace_listing(vec![entry(r"C:\r\c.txt", false)], Vec::new());
        assert!(!t.is_expanded(Path::new(r"C:\r\a")));
        assert_eq!(selected(&t), ["c.txt"]);
    }
}
