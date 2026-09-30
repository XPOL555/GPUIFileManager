//! The file view's selection and what is done with it: multiple selection with the
//! mouse and the keyboard (the same in the table and in the grid views), renaming in
//! place, delete, cut / copy / paste, new folders, dragging files out (to Explorer or
//! another window) and dropping them in. Operations run through the shell
//! (`file_ops`); the listing follows the folder's change notifications (`hooks`).

use std::time::Duration;

use gpui_kit::component::input::{self, Escape, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, ElementExt as _, Size};
use gpui_kit::prelude::FluentBuilder as _;

use super::views::{GRID_GAP, GRID_PAD};
use super::*;
use crate::dnd::{self, DropKind, SpotKey, Target};
use crate::file_ops::{self, CutFiles, FileOp};
use crate::fs::Entry;

/// How long an error stays in the status bar.
const ERROR_TIMEOUT: Duration = Duration::from_secs(8);

/// Height of the table's rows and of its header (the table is `small`).
fn row_height() -> Pixels {
    Size::Small.table_row_height()
}

/// An item being renamed in place.
pub(super) struct Rename {
    path: PathBuf,
    input: Entity<InputState>,
    /// The extension the editor doesn't show (hidden extensions, shortcuts); it stays.
    suffix: String,
    _subscription: Subscription,
}

/// A rubber band being dragged over the file view's background.
pub(super) struct Marquee {
    /// Where it started, in content coordinates: it scrolls with the items.
    origin: Point<Pixels>,
    /// The cursor, in window coordinates.
    current: Point<Pixels>,
    /// Rows selected before, which stay when Ctrl or Shift was held.
    base: Vec<usize>,
}

/// What to select once the folder has been listed again.
pub(super) enum Reselect {
    /// The same items, wherever they are now.
    Keep,
    /// These items (new ones: pasted, dropped, renamed).
    Paths(Vec<PathBuf>),
    /// The row at this position (after deleting what was there).
    Row(usize),
}

/// The app's cut files stop being faded once something else is on the clipboard.
pub(super) fn forget_stale_cut(cx: &mut App) {
    if cx.try_global::<CutFiles>().is_some_and(|cut| cut.sequence != file_ops::clipboard_sequence()) {
        cx.remove_global::<CutFiles>();
    }
}

impl FileManager {
    // ---- the view -------------------------------------------------------------

    /// The file view around the table or the grid: keyboard focus and actions, clicks
    /// on the background, the drag start, the drop zone of the listed folder.
    pub(super) fn render_files(&self, content: AnyElement, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.files_bounds.clone();
        let drop_here = self.drop_hover == Some(SpotKey::View);
        let theme = cx.theme();
        let (drop_target, drag_border) = (theme.drop_target, theme.drag_border);
        div()
            .id("files")
            .relative()
            .size_full()
            .track_focus(&self.files_focus)
            .key_context(FILES_CONTEXT)
            .on_action(cx.listener(|this, _: &CursorUp, window, cx| this.cursor_step(-1, true, window, cx)))
            .on_action(cx.listener(|this, _: &CursorDown, window, cx| this.cursor_step(1, true, window, cx)))
            .on_action(cx.listener(|this, _: &CursorLeft, window, cx| this.cursor_side(-1, window, cx)))
            .on_action(cx.listener(|this, _: &CursorRight, window, cx| this.cursor_side(1, window, cx)))
            .on_action(cx.listener(|this, _: &CursorHome, window, cx| this.move_cursor(|_, _| Some(0), window, cx)))
            .on_action(cx.listener(|this, _: &CursorEnd, window, cx| this.move_cursor(|_, n| Some(n - 1), window, cx)))
            .on_action(cx.listener(|this, _: &CursorPageUp, window, cx| {
                let page = this.page_size(cx) as isize;
                this.cursor_step(-page, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CursorPageDown, window, cx| {
                let page = this.page_size(cx) as isize;
                this.cursor_step(page, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.update_selection(cx, |d| d.select_all())))
            .on_action(cx.listener(|this, _: &ToggleCursorItem, _, cx| {
                if let Some(row) = this.table.read(cx).delegate().cursor() {
                    this.update_selection(cx, |d| d.toggle(row));
                }
            }))
            .on_action(cx.listener(Self::rename_selected))
            .on_action(cx.listener(|this, _: &DeleteSelected, window, cx| this.delete(true, window, cx)))
            .on_action(cx.listener(|this, _: &DeletePermanently, window, cx| this.delete(false, window, cx)))
            .on_action(cx.listener(|this, _: &CopySelected, _, cx| this.put_on_clipboard(false, cx)))
            .on_action(cx.listener(|this, _: &CutSelected, _, cx| this.put_on_clipboard(true, cx)))
            .on_action(cx.listener(|this, _: &PasteFiles, window, cx| {
                let dest = this.tab().path.clone();
                this.paste_into(dest, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowProperties, _, cx| {
                // Of the selection, or of the folder.
                let mut paths = this.table.read(cx).delegate().selected_paths();
                if paths.is_empty() && !this.in_bin() {
                    paths.push(this.tab().path.clone());
                }
                if !this.in_bin() {
                    menu::show_properties(&paths);
                }
            }))
            // Escape in the rename editor cancels it.
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.rename.is_some() {
                    cx.stop_propagation();
                    this.cancel_rename(window, cx);
                }
            }))
            // The rename editor lets Enter, Backspace at its start and Left / Right at
            // its ends bubble up. Handled here, they stop: otherwise the next bindings
            // of the key would run too (open the item, go up, move the selection).
            .on_action(|_: &input::Enter, _, _| {})
            .on_action(|_: &input::Backspace, _, _| {})
            .on_action(|_: &input::MoveLeft, _, _| {})
            .on_action(|_: &input::MoveRight, _, _| {})
            // Items stop their presses: these only see the background.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                this.background_press(ev, window, cx)
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                this.background_menu(ev, window, cx)
            }))
            .on_prepaint(move |b, _, _| bounds.set(b))
            .on_prepaint(dnd::zone(&self.drop_zones, SpotKey::View, self.tab().path.clone(), 0))
            .child(self.drag_listener(cx))
            .child(content)
            .children(self.render_marquee(cx))
            .when(drop_here, |d| {
                d.child(div().absolute().inset_0().border_2().border_color(drag_border).bg(drop_target.opacity(0.3)))
            })
    }

    /// Starts dragging the selection once the mouse moves far enough from where an item
    /// was pressed. A raw listener: it must see moves outside the window too.
    fn drag_listener(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (press, marquee) = (self.press.clone(), self.marquee_active.clone());
        let this = cx.entity().downgrade();
        let distance = shell::drag_distance() as f64;
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (press_up, marquee_up, this_up) = (press.clone(), marquee.clone(), this.clone());
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    if marquee.get() {
                        let pressed = e.pressed_button == Some(MouseButton::Left);
                        this.update(cx, |fm, cx| {
                            if pressed { fm.marquee_moved(e.position, cx) } else { fm.end_marquee(cx) }
                        })
                        .ok();
                        return;
                    }
                    let Some(origin) = press.get() else { return };
                    if e.pressed_button != Some(MouseButton::Left) {
                        press.set(None);
                    } else if (e.position - origin).magnitude() >= distance {
                        press.set(None);
                        this.update(cx, |fm, cx| fm.start_file_drag(cx)).ok();
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                    press_up.set(None);
                    if marquee_up.get() {
                        this_up.update(cx, |fm, cx| fm.end_marquee(cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }

    // ---- selection ------------------------------------------------------------

    pub(super) fn update_selection(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut FileTable)) {
        self.table.update(cx, |t, cx| {
            f(t.delegate_mut());
            cx.notify();
        });
        cx.notify();
    }

    /// Scrolls just enough to show `row` (`TableState::scroll_to_row` would put it on top).
    pub(super) fn scroll_to_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.view().is_table() {
            self.table.update(cx, |t, cx| {
                t.vertical_scroll_handle.scroll_to_item(row, ScrollStrategy::Nearest);
                cx.notify();
            });
        } else {
            self.scroll_grid_to(row);
        }
    }

    /// After the rows moved (new sorting or view): the cursor stays in sight.
    pub(super) fn scroll_to_cursor(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.table.read(cx).delegate().cursor() {
            self.scroll_to_row(row, cx);
        }
    }

    /// Mouse input on a table row.
    pub(super) fn on_row(&mut self, event: RowEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            RowEvent::Press { row, modifiers } => {
                let position = window.mouse_position();
                self.item_press(row, modifiers, position, window, cx);
            }
            RowEvent::Click { row, modifiers, count } => self.item_click(row, modifiers, count, window, cx),
            RowEvent::Menu { row, position, shift } => self.item_menu(row, position, shift, window, cx),
            RowEvent::Middle { row } => self.item_middle(row, window, cx),
        }
    }

    /// Left button down on an item. A plain press on an unselected item selects it at
    /// once, so it can be dragged; the rest waits for the click (the button released
    /// without dragging), so a selection can be dragged as a whole.
    pub(super) fn item_press(
        &mut self,
        row: usize,
        modifiers: Modifiers,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_view(window, cx);
        self.press.set(Some(position));
        if !modifiers.control && !modifiers.shift && !self.table.read(cx).delegate().is_selected(row) {
            self.update_selection(cx, |d| d.select_only(row));
        }
    }

    pub(super) fn item_click(
        &mut self,
        row: usize,
        modifiers: Modifiers,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (modifiers.control, modifiers.shift) {
            (_, true) => self.update_selection(cx, |d| d.select_range(row, modifiers.control)),
            (true, false) => self.update_selection(cx, |d| d.toggle(row)),
            _ if count >= 2 => {
                if let Some(e) = self.table.read(cx).delegate().entry(row).cloned() {
                    self.open_entry(e, window, cx);
                }
            }
            _ => self.update_selection(cx, |d| d.select_only(row)),
        }
    }

    /// Right button down on an item: the menu for the selection, which becomes the item
    /// alone unless it was part of it.
    pub(super) fn item_menu(
        &mut self,
        row: usize,
        position: Point<Pixels>,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_view(window, cx);
        // The selection shows what the menu is about; not the table's own right-click mark.
        self.table.update(cx, |t, cx| t.set_right_clicked_row(None, cx));
        if !self.table.read(cx).delegate().is_selected(row) {
            self.update_selection(cx, |d| d.select_only(row));
        }
        let entries = self.table.read(cx).delegate().selected_entries();
        self.show_menu(menu::MenuTarget::items(entries), position, shift, window, cx);
    }

    /// Middle button on a folder opens it in a new tab.
    pub(super) fn item_middle(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = self.table.read(cx).delegate().entry(row).filter(|e| e.is_dir).cloned() {
            self.open_tab(e.path, window, cx);
        }
    }

    /// Whether a press at `position` is on the table's header or scroll bar, which are
    /// not background.
    fn on_table_chrome(&self, position: Point<Pixels>) -> bool {
        let bounds = self.table_bounds.get();
        self.view().is_table() && (position.y < bounds.top() + row_height() || position.x > bounds.right() - px(14.))
    }

    /// A press on the background clears the selection (unless Ctrl or Shift is held)
    /// and starts a rubber band.
    fn background_press(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_view(window, cx);
        if self.on_table_chrome(ev.position) {
            return;
        }
        let keep = ev.modifiers.control || ev.modifiers.shift;
        if !keep {
            self.update_selection(cx, |d| d.clear_selection());
        }
        let base = if keep { self.table.read(cx).delegate().selected_rows() } else { Vec::new() };
        let origin = ev.position - self.content_origin(cx);
        self.marquee = Some(Marquee { origin, current: ev.position, base });
        self.marquee_active.set(true);
    }

    // ---- rubber band ----------------------------------------------------------

    /// Window position of the top left corner of the items, as scrolled now.
    fn content_origin(&self, cx: &App) -> Point<Pixels> {
        if self.view().is_table() {
            let bounds = self.table_bounds.get();
            let offset = self.table.read(cx).vertical_scroll_handle.0.borrow().base_handle.offset();
            point(bounds.left(), bounds.top() + row_height() + offset.y)
        } else {
            let bounds = self.files_bounds.get();
            let offset = self.grid_scroll.0.borrow().base_handle.offset();
            point(bounds.left() + px(GRID_PAD), bounds.top() + px(GRID_PAD / 2.) + offset.y)
        }
    }

    /// The rubber band in content coordinates.
    fn marquee_rect(&self, marquee: &Marquee, cx: &App) -> Bounds<Pixels> {
        let current = marquee.current - self.content_origin(cx);
        let a = marquee.origin;
        Bounds::from_corners(point(a.x.min(current.x), a.y.min(current.y)), point(a.x.max(current.x), a.y.max(current.y)))
    }

    fn marquee_moved(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(marquee) = self.marquee.as_mut() else { return };
        marquee.current = position;
        let marquee = self.marquee.as_ref().expect("just set");
        let rows = self.rows_in(self.marquee_rect(marquee, cx), cx);
        let base = marquee.base.clone();
        self.update_selection(cx, |d| d.select_rows(base.into_iter().chain(rows)));
    }

    fn end_marquee(&mut self, cx: &mut Context<Self>) {
        self.marquee_active.set(false);
        if self.marquee.take().is_some() {
            cx.notify();
        }
    }

    /// Rows whose item meets `rect` (content coordinates): whole rows in the table,
    /// tiles in the grid.
    fn rows_in(&self, rect: Bounds<Pixels>, cx: &App) -> Vec<usize> {
        let len = self.table.read(cx).delegate().len();
        let (left, top, right, bottom) =
            (f32::from(rect.left()), f32::from(rect.top()), f32::from(rect.right()), f32::from(rect.bottom()));
        if len == 0 || right < 0. || bottom < 0. {
            return Vec::new();
        }
        if self.view().is_table() {
            let h = f32::from(row_height());
            let (first, last) = ((top.max(0.) / h) as usize, ((bottom / h) as usize).min(len - 1));
            return (first..=last).collect();
        }
        let tile = self.view().tile();
        let (column_w, line_h) = (tile.w + GRID_GAP, tile.h + GRID_GAP);
        let columns = self.grid_columns.max(1);
        let lines = len.div_ceil(columns);
        let (first_line, last_line) = ((top.max(0.) / line_h) as usize, ((bottom / line_h) as usize).min(lines - 1));
        let (first_col, last_col) = ((left.max(0.) / column_w) as usize, ((right / column_w) as usize).min(columns - 1));
        let mut rows = Vec::new();
        for line in first_line..=last_line {
            for col in first_col..=last_col {
                let ix = line * columns + col;
                let item = Bounds::new(
                    point(px(col as f32 * column_w), px(line as f32 * line_h)),
                    size(px(tile.w), px(tile.h)),
                );
                if ix < len && item.intersects(&rect) {
                    rows.push(ix);
                }
            }
        }
        rows
    }

    /// The rubber band, drawn over the items.
    fn render_marquee(&self, cx: &App) -> Option<impl IntoElement> {
        let marquee = self.marquee.as_ref()?;
        let rect = self.marquee_rect(marquee, cx);
        // Back to coordinates inside the file view.
        let origin = self.content_origin(cx) - self.files_bounds.get().origin;
        let theme = cx.theme();
        Some(
            div()
                .absolute()
                .left(rect.left() + origin.x)
                .top(rect.top() + origin.y)
                .w(rect.size.width)
                .h(rect.size.height)
                .border_1()
                .border_color(theme.primary.opacity(0.7))
                .bg(theme.primary.opacity(0.15)),
        )
    }

    fn background_menu(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_view(window, cx);
        self.update_selection(cx, |d| d.clear_selection());
        self.show_menu(menu::MenuTarget::background(), ev.position, ev.modifiers.shift, window, cx);
    }

    /// Enter: opens the selection. Folders open in new tabs when several are selected.
    pub(super) fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        let mut entries = self.table.read(cx).delegate().selected_entries();
        if entries.len() == 1 {
            return self.open_entry(entries.remove(0), window, cx);
        }
        let mut first_tab = None;
        for e in entries {
            if e.is_dir {
                self.tabs.push(Tab::new(e.path, cx));
                first_tab.get_or_insert(self.tabs.len() - 1);
            } else {
                shell::open(&e.path);
            }
        }
        if let Some(ix) = first_tab {
            self.switch_tab(ix, window, cx);
        }
    }

    // ---- keyboard -------------------------------------------------------------

    /// Moves the cursor to the row `target` picks from the current one and the row
    /// count: alone it selects that row, with Shift it extends the selection, with Ctrl
    /// it leaves the selection alone.
    pub(super) fn move_cursor(
        &mut self,
        target: impl FnOnce(Option<usize>, usize) -> Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = window.modifiers();
        let (cursor, len) = {
            let d = self.table.read(cx).delegate();
            (d.cursor(), d.len())
        };
        if len == 0 {
            return;
        }
        let Some(row) = target(cursor, len).map(|r| r.min(len - 1)) else { return };
        self.update_selection(cx, |d| match (modifiers.control, modifiers.shift) {
            (_, true) => d.select_range(row, modifiers.control),
            (true, false) => d.move_cursor(row),
            _ => d.select_only(row),
        });
        self.scroll_to_row(row, cx);
    }

    /// Up / Down (`lines` = ±1, a grid line in the grid) and Page Up / Down.
    fn cursor_step(&mut self, lines: isize, per_line: bool, window: &mut Window, cx: &mut Context<Self>) {
        let step = if per_line && !self.view().is_table() { lines * self.grid_columns as isize } else { lines };
        self.move_cursor(
            |cursor, _| match cursor {
                None => Some(0),
                Some(row) => Some((row as isize + step).max(0) as usize),
            },
            window,
            cx,
        );
    }

    /// Left / Right: the next item in the grid; in the tree, collapse or go to the
    /// parent folder, expand or go into the folder.
    fn cursor_side(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        match self.view() {
            ViewMode::Details => {}
            ViewMode::Tree => {
                let d = self.table.read(cx).delegate();
                let Some(row) = d.cursor() else { return };
                let Some(e) = d.entry(row).cloned() else { return };
                let expanded = e.is_dir && d.is_expanded(&e.path);
                let (parent, has_children) = (d.parent_row(row), d.depth(row + 1) > d.depth(row));
                match (delta < 0, expanded) {
                    (true, true) => self.toggle_folder(e.path, false, window, cx),
                    (true, false) => {
                        if let Some(parent) = parent {
                            self.move_cursor(|_, _| Some(parent), window, cx);
                        }
                    }
                    (false, false) if e.is_dir => self.toggle_folder(e.path, true, window, cx),
                    (false, true) if has_children => self.move_cursor(|_, _| Some(row + 1), window, cx),
                    _ => {}
                }
            }
            _ => self.cursor_step(delta, false, window, cx),
        }
    }

    /// Rows (or items, in the grid) one page holds.
    fn page_size(&self, cx: &App) -> usize {
        let height = f32::from(self.files_bounds.get().size.height);
        if self.view().is_table() {
            self.table.read(cx).visible_range().rows().len().saturating_sub(1).max(1)
        } else {
            let lines = (height / self.view().tile_height()).floor().max(1.) as usize;
            lines * self.grid_columns.max(1)
        }
    }

    // ---- dragging files -------------------------------------------------------

    /// Drags the selection out as Explorer does: to other apps, other windows of this
    /// one, or folders of this window. Windows runs the drag (see `hooks`); the end
    /// comes back as `HookEvent::FileDragEnded`.
    fn start_file_drag(&mut self, cx: &mut Context<Self>) {
        let paths = self.table.read(cx).delegate().selected_paths();
        // The bin's own files ("$R…") are not the items; they only leave by a restore.
        if !paths.is_empty() && self.hwnd != 0 && self.rename.is_none() && !self.in_bin() {
            hooks::start_file_drag(self.hwnd, paths);
        }
    }

    pub(super) fn set_drop_hover(&mut self, spot: Option<SpotKey>, cx: &mut Context<Self>) {
        let folder = match &spot {
            Some(SpotKey::Item(path)) => Some(path.clone()),
            _ => None,
        };
        self.drop_hover = spot;
        self.table.update(cx, |t, cx| {
            t.delegate_mut().drop_hover = folder;
            cx.notify();
        });
        cx.notify();
    }

    /// Files dropped on this window, from anywhere.
    pub(super) fn drop_files(
        &mut self,
        paths: Vec<PathBuf>,
        target: Target,
        kind: DropKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let op = match (kind, target) {
            (DropKind::Favorite, _) => {
                for folder in paths.into_iter().filter(|p| p.is_dir()) {
                    self.add_favorite(folder, cx);
                }
                Settings::update(cx, |s| s.favorites_open = true);
                return;
            }
            (DropKind::Recycle, _) => FileOp::Delete { paths, recycle: true },
            (DropKind::Copy, Target::Folder(dest)) => FileOp::Copy { paths, dest },
            (DropKind::Move, Target::Folder(dest)) => FileOp::Move { paths, dest },
            _ => return,
        };
        let reselect = self.reselect_results(&op);
        self.run_op(op, reselect, window, cx).detach();
    }

    // ---- renaming -------------------------------------------------------------

    /// F2: renames the item with the cursor if it is selected, else the only selected one.
    pub(super) fn rename_selected(&mut self, _: &RenameSelected, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() || self.in_bin() {
            return;
        }
        let d = self.table.read(cx).delegate();
        let entry = d
            .cursor()
            .filter(|&row| d.is_selected(row))
            .and_then(|row| d.entry(row))
            .or_else(|| d.single_selected())
            .cloned();
        if let Some(entry) = entry {
            self.start_rename(entry, window, cx);
        }
    }

    /// Edits the name in place, with the name selected without its extension. A hidden
    /// extension isn't shown and stays as it is.
    pub(super) fn start_rename(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        let name = entry.display_name(Settings::get(cx).show_extensions).to_string();
        let suffix = entry.name[name.len()..].to_string();
        let stem = if suffix.is_empty() { file_ops::stem_len(&name, entry.is_dir) } else { name.len() };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |i, cx| i.set_selected_range(0..stem, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } => this.commit_rename(true, window, cx),
            InputEvent::Blur => this.commit_rename(false, window, cx),
            _ => {}
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        let path = entry.path.clone();
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.renaming = Some((path.clone(), input.clone()));
            d.select_paths(std::slice::from_ref(&path));
            cx.notify();
        });
        self.scroll_to_cursor(cx);
        self.rename = Some(Rename { path, input, suffix, _subscription: subscription });
        cx.notify();
    }

    /// Enter or leaving the editor. An invalid name keeps the editor open on Enter.
    fn commit_rename(&mut self, keep_on_error: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.as_ref() else { return };
        let typed = rename.input.read(cx).value().trim().to_string();
        let name = format!("{typed}{}", rename.suffix);
        let path = rename.path.clone();
        let unchanged = typed.is_empty() || path.file_name().is_some_and(|old| old.to_string_lossy() == name);
        if !unchanged && !file_ops::valid_name(&name) {
            self.show_error(i18n::t(cx).invalid_name, cx);
            if keep_on_error {
                return;
            }
        }
        self.cancel_rename(window, cx);
        if unchanged || !file_ops::valid_name(&name) {
            return;
        }
        let op = FileOp::Rename { path, name };
        let reselect = Reselect::Paths(op.results());
        self.run_op(op, reselect, window, cx).detach();
    }

    pub(super) fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename = None;
        self.table.update(cx, |t, cx| {
            t.delegate_mut().renaming = None;
            cx.notify();
        });
        self.focus_view(window, cx);
        cx.notify();
    }

    /// The rename editor of `path`, if it is being renamed.
    pub(super) fn renaming(&self, path: &Path) -> Option<&Entity<InputState>> {
        self.rename.as_ref().filter(|r| r.path == path).map(|r| &r.input)
    }

    // ---- operations -----------------------------------------------------------

    /// Delete (to the Recycle Bin) or Shift + Delete (for good). The shell asks first
    /// when Explorer would.
    pub(super) fn delete(&mut self, recycle: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            return;
        }
        if self.in_bin() {
            return self.purge_selected(window, cx);
        }
        let d = self.table.read(cx).delegate();
        let paths = d.selected_paths();
        let row = d.selected_rows().first().copied().unwrap_or(0);
        if !paths.is_empty() {
            self.run_op(FileOp::Delete { paths, recycle }, Reselect::Row(row), window, cx).detach();
        }
    }

    /// Ctrl + C / Ctrl + X: the selection on the clipboard, as Explorer puts it there.
    pub(super) fn put_on_clipboard(&mut self, cut: bool, cx: &mut Context<Self>) {
        let paths = self.table.read(cx).delegate().selected_paths();
        if paths.is_empty() || self.in_bin() {
            return;
        }
        match file_ops::set_clipboard_files(&paths, cut, self.hwnd) {
            Some(sequence) if cut => cx.set_global(CutFiles { sequence, paths: paths.into_iter().collect() }),
            Some(_) => {
                if cx.has_global::<CutFiles>() {
                    cx.remove_global::<CutFiles>();
                }
            }
            None => self.show_error(i18n::t(cx).clipboard_failed, cx),
        }
        cx.refresh_windows();
    }

    /// Ctrl + V: the files on the clipboard, copied, or moved if they were cut.
    pub(super) fn paste_into(&mut self, dest: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if fs::is_recycle_bin(&dest) {
            return;
        }
        let Some((paths, cut)) = file_ops::clipboard_files() else { return };
        if cut && paths.iter().all(|p| p.parent() == Some(dest.as_path())) {
            return;
        }
        let sequence = file_ops::clipboard_sequence();
        let op = if cut { FileOp::Move { paths, dest } } else { FileOp::Copy { paths, dest } };
        let reselect = self.reselect_results(&op);
        let done = self.run_op(op, reselect, window, cx);
        let owner = self.hwnd;
        cx.spawn(async move |_, cx| {
            // Moved files can't be pasted again.
            if done.await && cut {
                file_ops::clear_clipboard_if(sequence, owner);
                cx.update(|cx| {
                    if cx.has_global::<CutFiles>() {
                        cx.remove_global::<CutFiles>();
                    }
                    cx.refresh_windows();
                });
            }
        })
        .detach();
    }

    /// Ctrl + Shift + N: a new folder in the listed one, ready to be named.
    pub(super) fn new_folder(&mut self, _: &NewFolder, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_bin() {
            return;
        }
        let dir = self.tab().path.clone();
        let base = i18n::t(cx).new_folder;
        let created = cx.background_spawn(async move {
            let path = dir.join(file_ops::unique_name(&dir, base));
            std::fs::create_dir(&path).map(|_| path)
        });
        cx.spawn_in(window, async move |this, cx| {
            let created = created.await;
            let listed = this.update_in(cx, |fm, window, cx| match &created {
                Ok(path) => Some(fm.reload(Reselect::Paths(vec![path.clone()]), window, cx)),
                Err(err) => {
                    fm.show_error(err.to_string(), cx);
                    None
                }
            });
            if let (Ok(path), Ok(Some(listed))) = (created, listed) {
                listed.await;
                this.update_in(cx, |fm, window, cx| {
                    let d = fm.table.read(cx).delegate();
                    if let Some(entry) = d.single_selected().filter(|e| e.path == path).cloned() {
                        fm.start_rename(entry, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Items pasted or dropped into the listed folder get selected, unless they are
    /// copies next to their originals (which get new names).
    fn reselect_results(&self, op: &FileOp) -> Reselect {
        match op {
            FileOp::Copy { paths, dest } if paths.iter().any(|p| p.parent() == Some(dest.as_path())) => Reselect::Keep,
            FileOp::Copy { dest, .. } | FileOp::Move { dest, .. } if *dest == self.tab().path => {
                Reselect::Paths(op.results())
            }
            _ => Reselect::Keep,
        }
    }

    /// Runs `op` through the shell, then lists the folder again. The task tells whether
    /// it went through (a cancel counts as done).
    pub(super) fn run_op(
        &mut self,
        op: FileOp,
        reselect: Reselect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let touches_bin = op.touches_bin();
        let done = file_ops::run(op, self.hwnd);
        cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let ok = result.is_ok();
            this.update_in(cx, |fm, window, cx| {
                if let Err(err) = result {
                    let message = if err.is_empty() { i18n::t(cx).operation_failed.to_string() } else { err };
                    fm.show_error(message, cx);
                }
                if touches_bin {
                    super::bin::check_bin(cx);
                }
                fm.reload(reselect, window, cx).detach();
            })
            .ok();
            ok
        })
    }

    /// Lists the folder (and the expanded ones) again in place: the filter, the scroll
    /// position and, unless told otherwise, the selection stay.
    pub(super) fn reload(&mut self, reselect: Reselect, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        let path = self.tab().path.clone();
        let expanded = self.table.read(cx).delegate().expanded();
        let generation = self.load_generation;
        let listing = cx.background_spawn(async move {
            let children: Vec<(PathBuf, Vec<Entry>)> =
                expanded.into_iter().filter_map(|p| fs::list(&p).ok().map(|list| (p, list))).collect();
            (super::list_folder(&path), children)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (entries, children) = listing.await;
            this.update_in(cx, |fm, _, cx| {
                if fm.load_generation != generation {
                    return;
                }
                let entries = entries.unwrap_or_else(|err| {
                    fm.error = Some(err.to_string().into());
                    Vec::new()
                });
                let row = fm.table.update(cx, |t, cx| {
                    let d = t.delegate_mut();
                    d.replace_listing(entries, children);
                    let row = match reselect {
                        Reselect::Keep => None,
                        Reselect::Paths(paths) => d.select_paths(&paths),
                        Reselect::Row(row) if d.len() > 0 => {
                            let row = row.min(d.len() - 1);
                            d.select_only(row);
                            Some(row)
                        }
                        Reselect::Row(_) => None,
                    };
                    t.refresh(cx);
                    row
                });
                if let Some(row) = row {
                    fm.scroll_to_row(row, cx);
                }
                cx.notify();
            })
            .ok();
        })
    }

    /// The listed folder changed. The shell already gathers changes for about a second;
    /// a copy still sends a few in a row: list the folder again once they settle a
    /// little, longer for big folders.
    pub(super) fn folder_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reload_timer.is_some() {
            return;
        }
        let items = self.table.read(cx).delegate().len() as u64;
        let delay = Duration::from_millis((100 + items / 400).min(2000));
        self.reload_timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update_in(cx, |fm, window, cx| {
                fm.reload_timer = None;
                fm.reload(Reselect::Keep, window, cx).detach();
            })
            .ok();
        }));
    }

    /// Shows `message` in the status bar for a while.
    pub(super) fn show_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        let message = message.into();
        self.error = Some(message.clone());
        self.error_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ERROR_TIMEOUT).await;
            this.update(cx, |fm, cx| {
                if fm.error.as_ref() == Some(&message) {
                    fm.error = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }
}
