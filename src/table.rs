//! Table delegate: owns the current folder snapshot and the filtered/sorted view of it.
//! In tree mode it also holds the children of expanded folders.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::fs::{self, Entry, SortKey};
use crate::i18n;
use crate::icons::{self, IconSize};

/// Asks the owner to expand (`true`) or collapse a folder of the tree.
pub type ToggleFolder = Rc<dyn Fn(PathBuf, bool, &mut Window, &mut App)>;

/// A table row: entry `index` of the top level (`parent: None`) or of an expanded folder.
#[derive(Clone)]
struct Row {
    depth: u16,
    parent: Option<Arc<Path>>,
    index: u32,
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
    sort_key: SortKey,
    descending: bool,
    tree: bool,
    on_toggle: Option<ToggleFolder>,
    pub loading: bool,
    columns: Vec<Column>,
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
            sort_key: SortKey::Name,
            descending: false,
            tree: false,
            on_toggle: None,
            loading: false,
            // Names are filled in per render from the current language, see `column`.
            columns: vec![
                Column::new("name", "").width(px(380.)).min_width(px(120.)).ascending(),
                Column::new("type", "").width(px(110.)).sortable(),
                Column::new("modified", "").width(px(150.)).sortable(),
                Column::new("size", "").width(px(110.)).text_right().sortable(),
            ],
        }
    }

    pub fn set_on_toggle(&mut self, on_toggle: ToggleFolder) {
        self.on_toggle = Some(on_toggle);
    }

    /// A new folder listing; tree expansions belong to the previous one and are dropped.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.children.clear();
        self.expanding.clear();
        self.rebuild();
    }

    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_lowercase();
        self.rebuild();
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.rebuild();
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

    pub fn entry(&self, row: usize) -> Option<&Entry> {
        let row = self.rows.get(row)?;
        let list = match &row.parent {
            None => &self.entries,
            Some(p) => self.children.get(p.as_ref())?,
        };
        list.get(row.index as usize)
    }

    /// Number of rows, including the children of expanded folders.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn depth(&self, row: usize) -> usize {
        self.rows.get(row).map_or(0, |r| r.depth as usize)
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

    fn rebuild(&mut self) {
        let mut rows = Vec::with_capacity(self.entries.len());
        self.push_level(&mut rows, None, 0);
        self.rows = rows;
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
            .filter(|&i| self.show_hidden || !list[i as usize].hidden)
            .collect();
        let (key, desc) = (self.sort_key, self.descending);
        order.sort_by(|&a, &b| fs::compare(&list[a as usize], &list[b as usize], key, desc));

        let mut any = false;
        for i in order {
            let e = &list[i as usize];
            let mark = rows.len();
            rows.push(Row { depth, parent: parent.clone(), index: i });
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
            .child(div().truncate().child(e.name.clone()))
            .into_any_element()
    }
}

impl TableDelegate for FileTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, cx: &App) -> Column {
        let s = i18n::t(cx);
        let mut column = self.columns[col_ix].clone();
        column.name = match COLUMNS[col_ix] {
            SortKey::Name => s.col_name,
            SortKey::Type => s.col_type,
            SortKey::Modified => s.col_modified,
            SortKey::Size => s.col_size,
        }
        .into();
        column
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.sort_key = COLUMNS[col_ix];
        self.descending = matches!(sort, ColumnSort::Descending);
        if matches!(sort, ColumnSort::Default) {
            self.sort_key = SortKey::Name;
        }
        self.rebuild();
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
        }
    }

    fn names(t: &FileTable) -> Vec<(usize, String)> {
        (0..t.len()).map(|r| (t.depth(r), t.entry(r).unwrap().name.to_string())).collect()
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
}

/// "Folder", "File" or "PDF file", in the current language.
pub fn type_label(e: &Entry, cx: &App) -> String {
    let s = i18n::t(cx);
    if e.is_dir {
        s.folder.to_string()
    } else if e.ext.is_empty() {
        s.file.to_string()
    } else {
        (s.file_of_type)(&e.ext.to_uppercase())
    }
}
