//! Table delegate: owns the current folder snapshot and the filtered/sorted view of it.

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::fs::{self, Entry, SortKey};

pub struct FileTable {
    entries: Vec<Entry>,
    /// Indices into `entries` after filter + sort; row `i` of the table is `entries[rows[i]]`.
    rows: Vec<usize>,
    filter: String,
    show_hidden: bool,
    sort_key: SortKey,
    descending: bool,
    pub loading: bool,
    columns: Vec<Column>,
}

const COLUMNS: [SortKey; 4] = [SortKey::Name, SortKey::Type, SortKey::Modified, SortKey::Size];

impl FileTable {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            rows: Vec::new(),
            filter: String::new(),
            show_hidden: true,
            sort_key: SortKey::Name,
            descending: false,
            loading: false,
            columns: vec![
                Column::new("name", "Nome").width(px(380.)).min_width(px(120.)).ascending(),
                Column::new("type", "Tipo").width(px(110.)).sortable(),
                Column::new("modified", "Modificato").width(px(150.)).sortable(),
                Column::new("size", "Dimensione").width(px(110.)).text_right().sortable(),
            ],
        }
    }

    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.rebuild();
    }

    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_lowercase();
        self.rebuild();
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.rebuild();
    }

    pub fn entry(&self, row: usize) -> Option<&Entry> {
        self.rows.get(row).map(|&i| &self.entries[i])
    }

    pub fn row_of(&self, name: &str) -> Option<usize> {
        self.rows.iter().position(|&i| self.entries[i].name.as_ref() == name)
    }

    pub fn summary(&self) -> (usize, usize) {
        let dirs = self.rows.iter().filter(|&&i| self.entries[i].is_dir).count();
        (dirs, self.rows.len() - dirs)
    }

    fn rebuild(&mut self) {
        let f = &self.filter;
        self.rows = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                (self.show_hidden || !e.hidden) && (f.is_empty() || e.name.to_lowercase().contains(f))
            })
            .collect();
        self.sort();
    }

    fn sort(&mut self) {
        let (entries, key, desc) = (&self.entries, self.sort_key, self.descending);
        self.rows.sort_by(|&a, &b| fs::compare(&entries[a], &entries[b], key, desc));
    }
}

impl TableDelegate for FileTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
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
        self.sort();
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(e) = self.entry(row_ix) else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let text = match COLUMNS[col_ix] {
            SortKey::Name => {
                let icon = if e.is_dir { IconName::Folder } else { IconName::File };
                return h_flex()
                    .gap_2()
                    .overflow_hidden()
                    .when(e.hidden, |d| d.opacity(0.55))
                    .child(Icon::new(icon).small().text_color(if e.is_dir { cx.theme().warning } else { muted }))
                    .child(div().truncate().child(e.name.clone()))
                    .into_any_element();
            }
            SortKey::Type if e.is_dir => "Cartella".to_string(),
            SortKey::Type if e.ext.is_empty() => "File".to_string(),
            SortKey::Type => format!("File {}", e.ext.to_uppercase()),
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
