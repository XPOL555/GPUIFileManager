//! The app's context menu, in Windows 11's style: roomy rows, a row of icon buttons for
//! the commonest commands, submenus, keyboard navigation, and a short slide as it
//! opens (down, or up when it opens above the cursor). GPUI draws it inside the
//! window, over everything else.
//!
//! Entries can be filled in after the menu is shown (`Slot`, `set_image`): Explorer's
//! own entries take a moment to load (see `shell_menu`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, ElementExt as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

type LucideIcon = gpui_kit::assets::IconName;

/// What an entry or a button does when picked.
pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

actions!(context_menu, [MenuUp, MenuDown, MenuLeft, MenuRight, MenuConfirm, MenuCancel, MenuHome, MenuEnd]);

const CONTEXT: &str = "ContextMenu";

const ITEM_HEIGHT: f32 = 30.;
const TEXT_SIZE: f32 = 14.5;
const TOOLBAR_HEIGHT: f32 = 44.;
const SEPARATOR_HEIGHT: f32 = 7.;
const PADDING: f32 = 4.;
const MIN_WIDTH: f32 = 260.;
const MAX_WIDTH: f32 = 420.;
/// How long the pointer rests on an entry before its submenu opens or closes.
const HOVER_DELAY: Duration = Duration::from_millis(160);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", MenuUp, Some(CONTEXT)),
        KeyBinding::new("down", MenuDown, Some(CONTEXT)),
        KeyBinding::new("left", MenuLeft, Some(CONTEXT)),
        KeyBinding::new("right", MenuRight, Some(CONTEXT)),
        KeyBinding::new("enter", MenuConfirm, Some(CONTEXT)),
        KeyBinding::new("space", MenuConfirm, Some(CONTEXT)),
        KeyBinding::new("escape", MenuCancel, Some(CONTEXT)),
        KeyBinding::new("home", MenuHome, Some(CONTEXT)),
        KeyBinding::new("end", MenuEnd, Some(CONTEXT)),
    ]);
}

#[derive(Clone)]
pub enum MenuIcon {
    None,
    Named(LucideIcon),
    Image(Arc<RenderImage>),
}

/// An entry: a command, or a submenu when it has children.
#[derive(Clone)]
pub struct MenuEntry {
    label: SharedString,
    icon: MenuIcon,
    shortcut: Option<SharedString>,
    disabled: bool,
    checked: bool,
    handler: Option<Handler>,
    children: Vec<MenuItem>,
    /// Finds the entry again to give it an image later (`set_image`).
    tag: Option<u32>,
}

impl MenuEntry {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: MenuIcon::None,
            shortcut: None,
            disabled: false,
            checked: false,
            handler: None,
            children: Vec::new(),
            tag: None,
        }
    }

    pub fn icon(mut self, icon: LucideIcon) -> Self {
        self.icon = MenuIcon::Named(icon);
        self
    }

    pub fn image(mut self, image: Arc<RenderImage>) -> Self {
        self.icon = MenuIcon::Image(image);
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// A check mark in place of the icon.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.handler = Some(Rc::new(handler));
        self
    }

    pub fn submenu(mut self, children: Vec<MenuItem>) -> Self {
        self.children = children;
        self
    }

    pub fn tag(mut self, tag: u32) -> Self {
        self.tag = Some(tag);
        self
    }

    fn has_submenu(&self) -> bool {
        !self.children.is_empty()
    }
}

/// An icon button of the row on top.
#[derive(Clone)]
pub struct ToolButton {
    icon: LucideIcon,
    label: SharedString,
    disabled: bool,
    handler: Handler,
}

impl ToolButton {
    pub fn new(icon: LucideIcon, label: impl Into<SharedString>, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { icon, label: label.into(), disabled: false, handler: Rc::new(handler) }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Clone)]
pub enum MenuItem {
    Entry(MenuEntry),
    Separator,
    /// Icon buttons in a row (cut, copy, rename, delete…).
    Toolbar(Vec<ToolButton>),
    /// Where entries come in later (`fill_slot`); it stays, invisible, for more.
    Slot(u8),
}

impl From<MenuEntry> for MenuItem {
    fn from(entry: MenuEntry) -> Self {
        MenuItem::Entry(entry)
    }
}

impl MenuItem {
    fn entry(&self) -> Option<&MenuEntry> {
        match self {
            MenuItem::Entry(entry) => Some(entry),
            _ => None,
        }
    }

    fn height(&self) -> f32 {
        match self {
            MenuItem::Entry(_) => ITEM_HEIGHT,
            MenuItem::Separator => SEPARATOR_HEIGHT,
            MenuItem::Toolbar(_) => TOOLBAR_HEIGHT,
            MenuItem::Slot(_) => 0.,
        }
    }

    /// An entry that can be picked (from the keyboard too).
    fn selectable(&self) -> bool {
        self.entry().is_some_and(|e| !e.disabled)
    }
}

/// Indices of the items to draw: no slots, no separator first, last or twice in a row.
fn shown(items: &[MenuItem]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(items.len());
    for (ix, item) in items.iter().enumerate() {
        match item {
            MenuItem::Slot(_) => {}
            MenuItem::Separator if out.last().is_none_or(|&last| matches!(items[last], MenuItem::Separator)) => {}
            _ => out.push(ix),
        }
    }
    while out.last().is_some_and(|&last| matches!(items[last], MenuItem::Separator)) {
        out.pop();
    }
    out
}

/// Tells menus apart, so each one plays its opening animation.
static SERIAL: AtomicUsize = AtomicUsize::new(0);

pub struct ContextMenu {
    focus: FocusHandle,
    items: Vec<MenuItem>,
    /// Where it opened, in window coordinates.
    position: Point<Pixels>,
    /// It opens above the position (not enough room below).
    upward: bool,
    serial: usize,
    /// The highlighted entry of each open level (0: the menu itself).
    selected: Vec<Option<usize>>,
    /// The entries whose submenus are open: `open[0]` is in the menu, `open[1]` in its
    /// submenu… Always one shorter than `selected`.
    open: Vec<usize>,
    /// A submenu about to open or close under the pointer.
    pending: Option<Task<()>>,
    /// Where each level was drawn, for clicks outside.
    panels: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    dismissed: bool,
    /// Entries are still being read (Explorer's): a progress bar at the bottom says so.
    loading: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ContextMenu {}

impl Focusable for ContextMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ContextMenu {
    /// A menu at `position` (window coordinates). Show it by rendering the entity; it
    /// emits `DismissEvent` once done.
    pub fn new(mut items: Vec<MenuItem>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let height: f32 = shown(&items).iter().map(|&ix| items[ix].height()).sum::<f32>() + 2. * PADDING + 2.;
        let room_below = f32::from(window.viewport_size().height - position.y);
        let upward = height > room_below - 8. && f32::from(position.y) > room_below;
        // The icon row stays next to the pointer, as in Windows 11.
        if upward && matches!(items.first(), Some(MenuItem::Toolbar(_))) {
            let toolbar = items.remove(0);
            items.push(MenuItem::Separator);
            items.push(toolbar);
        }
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_blur(&focus, window, |this, _, cx| this.dismiss(cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.dismiss(cx);
                }
            }),
        ];
        Self {
            focus,
            items,
            position,
            upward,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            selected: vec![None],
            open: Vec::new(),
            pending: None,
            panels: Rc::default(),
            dismissed: false,
            loading: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.dismissed {
            self.dismissed = true;
            self.pending = None;
            cx.emit(DismissEvent);
        }
    }

    /// Shows or hides the progress bar for entries that are still loading.
    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        if self.loading != loading {
            self.loading = loading;
            cx.notify();
        }
    }

    /// Puts `entries` where `Slot(slot)` is, after what it got before. The highlight
    /// stays on its entry.
    pub fn fill_slot(&mut self, slot: u8, entries: Vec<MenuItem>, cx: &mut Context<Self>) {
        let Some(at) = self.items.iter().position(|item| matches!(item, MenuItem::Slot(s) if *s == slot)) else {
            return;
        };
        let added = entries.len();
        self.items.splice(at..at, entries);
        let shift = |ix: &mut usize| {
            if *ix >= at {
                *ix += added;
            }
        };
        if let Some(Some(ix)) = self.selected.first_mut() {
            shift(ix);
        }
        if let Some(ix) = self.open.first_mut() {
            shift(ix);
        }
        cx.notify();
    }

    /// Gives the entry tagged `tag` (anywhere) its image.
    pub fn set_image(&mut self, tag: u32, image: Arc<RenderImage>, cx: &mut Context<Self>) {
        fn find(items: &mut [MenuItem], tag: u32) -> Option<&mut MenuEntry> {
            for item in items {
                if let MenuItem::Entry(entry) = item {
                    if entry.tag == Some(tag) {
                        return Some(entry);
                    }
                    if let Some(found) = find(&mut entry.children, tag) {
                        return Some(found);
                    }
                }
            }
            None
        }
        if let Some(entry) = find(&mut self.items, tag) {
            entry.icon = MenuIcon::Image(image);
            cx.notify();
        }
    }

    /// The items of an open level.
    fn level(&self, level: usize) -> &[MenuItem] {
        let mut items = self.items.as_slice();
        for &ix in self.open.iter().take(level) {
            match items.get(ix).and_then(MenuItem::entry) {
                Some(entry) => items = &entry.children,
                None => return &[],
            }
        }
        items
    }

    fn deepest(&self) -> usize {
        self.open.len()
    }

    /// Closes the submenus below `level`.
    fn close_below(&mut self, level: usize) {
        self.open.truncate(level);
        self.selected.truncate(level + 1);
    }

    /// Opens the submenu of entry `ix` of `level`, closing any other one.
    fn open_submenu(&mut self, level: usize, ix: usize, select_first: bool) {
        self.close_below(level);
        self.selected[level] = Some(ix);
        self.open.push(ix);
        let first = select_first.then(|| self.level(level + 1).iter().position(MenuItem::selectable)).flatten();
        self.selected.push(first);
    }

    /// The pointer rests on entry `ix` of `level`: it gets the highlight at once, its
    /// submenu (or the closing of another one) after a moment, so the pointer can
    /// cross other entries on its way to an open submenu.
    fn hover(&mut self, level: usize, ix: usize, cx: &mut Context<Self>) {
        if level >= self.selected.len() {
            return;
        }
        self.selected[level] = Some(ix);
        if self.open.get(level) == Some(&ix) {
            // Back on the entry whose submenu is open.
            self.pending = None;
            self.selected.truncate(self.open.len() + 1);
            return cx.notify();
        }
        let has_submenu = self.level(level).get(ix).and_then(MenuItem::entry).is_some_and(MenuEntry::has_submenu);
        if !has_submenu && self.open.len() <= level {
            self.pending = None;
            return cx.notify();
        }
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            this.update(cx, |this, cx| {
                if this.selected.get(level).copied().flatten() != Some(ix) {
                    return;
                }
                if has_submenu {
                    this.open_submenu(level, ix, false);
                } else {
                    this.close_below(level);
                    this.selected[level] = Some(ix);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Entry `ix` of `level` picked: its submenu opens, or its command runs and the
    /// menu closes.
    fn activate(&mut self, level: usize, ix: usize, from_keyboard: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.level(level).get(ix).and_then(MenuItem::entry).filter(|e| !e.disabled) else { return };
        if entry.has_submenu() {
            self.pending = None;
            self.open_submenu(level, ix, from_keyboard);
            return cx.notify();
        }
        let handler = entry.handler.clone();
        self.dismiss(cx);
        if let Some(handler) = handler {
            handler(window, cx);
        }
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let level = self.deepest();
        let items = self.level(level);
        let order: Vec<usize> = shown(items).into_iter().filter(|&ix| items[ix].selectable()).collect();
        if order.is_empty() {
            return;
        }
        let at = self.selected[level].and_then(|ix| order.iter().position(|&o| o == ix));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => order.len() - 1,
            (Some(at), true) => (at + 1) % order.len(),
            (Some(at), false) => (at + order.len() - 1) % order.len(),
        };
        self.selected[level] = Some(order[next]);
        self.pending = None;
        cx.notify();
    }

    fn select_end(&mut self, last: bool, cx: &mut Context<Self>) {
        let level = self.deepest();
        let items = self.level(level);
        let order: Vec<usize> = shown(items).into_iter().filter(|&ix| items[ix].selectable()).collect();
        let pick = if last { order.last() } else { order.first() };
        self.selected[level] = pick.copied();
        cx.notify();
    }

    fn on_up(&mut self, _: &MenuUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn on_down(&mut self, _: &MenuDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn on_home(&mut self, _: &MenuHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_end(false, cx);
    }

    fn on_end(&mut self, _: &MenuEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_end(true, cx);
    }

    fn on_right(&mut self, _: &MenuRight, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.deepest();
        if let Some(ix) = self.selected[level]
            && self.level(level).get(ix).and_then(MenuItem::entry).is_some_and(MenuEntry::has_submenu)
        {
            self.activate(level, ix, true, window, cx);
        }
    }

    fn on_left(&mut self, _: &MenuLeft, _: &mut Window, cx: &mut Context<Self>) {
        let level = self.deepest();
        if level > 0 {
            self.close_below(level - 1);
            cx.notify();
        }
    }

    fn on_confirm(&mut self, _: &MenuConfirm, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.deepest();
        if let Some(ix) = self.selected[level] {
            self.activate(level, ix, true, window, cx);
        }
    }

    fn on_cancel(&mut self, _: &MenuCancel, _: &mut Window, cx: &mut Context<Self>) {
        let level = self.deepest();
        if level > 0 {
            self.close_below(level - 1);
            cx.notify();
        } else {
            self.dismiss(cx);
        }
    }

    // ---- drawing ------------------------------------------------------------------

    fn render_panel(&self, level: usize, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let panels = self.panels.clone();
        let items = self.level(level);
        let rows: Vec<AnyElement> =
            shown(items).into_iter().map(|ix| self.render_item(level, ix, &items[ix], window, cx)).collect();
        let theme = cx.theme();
        v_flex()
            .id(("menu-panel", level))
            .occlude()
            .relative()
            .min_w(px(MIN_WIDTH))
            .max_w(px(MAX_WIDTH))
            .max_h(window.viewport_size().height - px(16.))
            .overflow_y_scroll()
            .p(px(PADDING))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(theme.border)
            .rounded(px(8.))
            .shadow_lg()
            .text_size(px(TEXT_SIZE))
            .on_prepaint(move |bounds, _, _| {
                let mut panels = panels.borrow_mut();
                if panels.len() <= level {
                    panels.resize(level + 1, Bounds::default());
                }
                panels[level] = bounds;
            })
            .children(rows)
            .when(level == 0 && self.loading, |d| d.child(self.render_progress(cx)))
    }

    /// An indeterminate bar: a segment sliding across the track until the entries arrive.
    fn render_progress(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let segment = div().absolute().top_0().bottom_0().w(relative(0.35)).rounded_full().bg(theme.primary).with_animation(
            ("menu-progress", self.serial),
            Animation::new(Duration::from_millis(1100)).repeat().with_easing(ease_in_out),
            |segment, t| segment.left(relative(-0.35 + 1.35 * t)),
        );
        div()
            .relative()
            .flex_shrink_0()
            .h(px(3.))
            .mx(px(10.))
            .my(px(5.))
            .rounded_full()
            .overflow_hidden()
            .bg(theme.border)
            .child(segment)
    }

    fn render_item(&self, level: usize, ix: usize, item: &MenuItem, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        match item {
            MenuItem::Separator => div().h(px(1.)).mx(px(-PADDING)).my(px(3.)).bg(theme.border).into_any_element(),
            MenuItem::Slot(_) => Empty.into_any_element(),
            MenuItem::Toolbar(buttons) => h_flex()
                .h(px(TOOLBAR_HEIGHT))
                .px_1()
                .gap_1()
                .justify_around()
                .children(buttons.iter().enumerate().map(|(b, button)| self.render_tool(level, b, button, cx)))
                .into_any_element(),
            MenuItem::Entry(entry) => self.render_entry(level, ix, entry, window, cx),
        }
    }

    fn render_tool(&self, level: usize, index: usize, button: &ToolButton, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (hover, label, handler) = (theme.accent, button.label.clone(), button.handler.clone());
        div()
            .id(("menu-tool", level * 100 + index))
            .size(px(38.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .child(Icon::new(button.icon).size(px(18.)))
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .when(button.disabled, |d| d.opacity(0.35))
            .when(!button.disabled, |d| {
                d.cursor_pointer().hover(move |d| d.bg(hover)).on_click(cx.listener(move |this, _, window, cx| {
                    this.dismiss(cx);
                    handler(window, cx);
                }))
            })
            .into_any_element()
    }

    fn render_entry(&self, level: usize, ix: usize, entry: &MenuEntry, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let highlighted = self.selected.get(level).copied().flatten() == Some(ix) || self.open.get(level) == Some(&ix);
        let muted = theme.muted_foreground;
        let icon = match &entry.icon {
            MenuIcon::Named(icon) => Some(Icon::new(*icon).size(px(16.)).into_any_element()),
            MenuIcon::Image(image) => Some(img(image.clone()).size(px(16.)).into_any_element()),
            MenuIcon::None if entry.checked => Some(Icon::new(LucideIcon::Check).size(px(16.)).into_any_element()),
            MenuIcon::None => None,
        };
        let open = self.open.get(level) == Some(&ix);
        h_flex()
            .id(("menu-entry", level * 10_000 + ix))
            .relative()
            .flex_shrink_0()
            .h(px(ITEM_HEIGHT))
            .px(px(10.))
            .gap(px(12.))
            .rounded(px(5.))
            .when(highlighted && !entry.disabled, |d| d.bg(theme.accent).text_color(theme.accent_foreground))
            .when(entry.disabled, |d| d.text_color(muted))
            .child(h_flex().w(px(18.)).flex_shrink_0().justify_center().children(icon))
            .child(div().flex_1().min_w_0().truncate().child(entry.label.clone()))
            .children(entry.shortcut.clone().map(|s| div().flex_shrink_0().ml_4().text_xs().text_color(muted).child(s)))
            .when(entry.has_submenu(), |d| d.child(Icon::new(LucideIcon::ChevronRight).size(px(14.)).text_color(muted)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hover(level, ix, cx);
                }
            }))
            .when(!entry.disabled, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| this.activate(level, ix, false, window, cx)))
            })
            .when(open, |d| d.child(self.render_submenu(level + 1, window, cx)))
            .into_any_element()
    }

    /// A submenu beside its entry: on the right, or on the left without room there.
    fn render_submenu(&self, level: usize, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let parent = self.panels.borrow().get(level - 1).copied().unwrap_or_default();
        let on_left = parent.right() + px(MAX_WIDTH) > window.viewport_size().width;
        let panel = self.render_panel(level, window, cx).with_animation(
            ("menu-submenu", self.serial * 16 + level),
            Animation::new(Duration::from_millis(120)).with_easing(ease_out_quint()),
            move |panel, t| panel.opacity(t).left(px(if on_left { 6. } else { -6. } * (1. - t))),
        );
        // From the entry's top left corner (the panel's edge + border + padding), so the
        // submenu overlaps its panel by a few pixels and its first entry lines up.
        let (anchor, left) = if on_left {
            (Anchor::TopRight, px(-1.))
        } else {
            (Anchor::TopLeft, parent.size.width - px(2. * PADDING + 1.))
        };
        div().absolute().top_0().left_0().child(
            deferred(
                anchored()
                    .anchor(anchor)
                    .snap_to_window_with_margin(px(8.))
                    .child(div().top(px(-PADDING - 1.)).left(left).child(panel)),
            )
            .with_priority(4 + level),
        )
    }

    /// Clicks outside every panel close the menu; they still reach what is under them.
    fn outside_listener(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (panels, this) = (self.panels.clone(), cx.entity().downgrade());
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (panels, this) = (panels.clone(), this.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && !panels.borrow().iter().any(|b| b.contains(&e.position)) {
                        this.update(cx, |menu, cx| menu.dismiss(cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Levels that closed don't count for clicks outside.
        self.panels.borrow_mut().truncate(self.open.len() + 1);
        let upward = self.upward;
        let panel = self
            .render_panel(0, window, cx)
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_left))
            .on_action(cx.listener(Self::on_right))
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_end))
            .with_animation(
                ("context-menu", self.serial),
                Animation::new(Duration::from_millis(180)).with_easing(ease_out_quint()),
                move |panel, t| panel.opacity(t).top(px(if upward { 10. } else { -10. } * (1. - t))),
            );
        div().absolute().size_0().child(self.outside_listener(cx)).child(
            deferred(
                anchored()
                    .position(self.position)
                    .anchor(if upward { Anchor::BottomLeft } else { Anchor::TopLeft })
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(3),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{MenuEntry, MenuItem, shown};

    #[test]
    fn separators_only_between_entries() {
        let entry = || MenuItem::from(MenuEntry::new("x"));
        let items = vec![
            MenuItem::Separator,
            entry(),
            MenuItem::Separator,
            MenuItem::Slot(0),
            MenuItem::Separator,
            entry(),
            MenuItem::Separator,
            MenuItem::Slot(1),
        ];
        assert_eq!(shown(&items), [1, 2, 5]);
        assert!(shown(&[MenuItem::Separator, MenuItem::Slot(0)]).is_empty());
    }
}
