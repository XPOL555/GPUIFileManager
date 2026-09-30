//! Cover Flow, as in the old Finder: the items as covers in perspective, the current
//! one facing the viewer, over the details list. Dragging on the covers (or the wheel,
//! or the bar under them) runs through them quickly; covers and list share the
//! selection. A filter keeps only photos, or photos and videos (`Settings::media_filter`).
//!
//! GPUI draws images only upright, so a cover turned away is drawn as thin vertical
//! strips, each scaled as perspective wants; the strips hidden behind the next cover
//! toward the middle are skipped, which is most of them. Reflections are the lower
//! part of the images flipped and faded, made once per image.
//!
//! The covers take `Settings::flow_height` of the view; the handle across their edge
//! with the list resizes it.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;

use super::*;
use crate::fs::MediaFilter;
use crate::icons::{self, IconSize};

/// Seconds for the glide to cover about two thirds of the way left.
const GLIDE: f32 = 0.09;
/// How far covers aside are turned (radians, about 70°).
const TURN: f32 = 1.22;
/// Pointer travel per cover while dragging, in pixels.
const DRAG_PER_COVER: f32 = 36.;
/// The part of an image mirrored under it.
const REFLECTION: f32 = 0.4;
/// Covers on each side whose thumbnails are asked for, and drawn once they fit.
const SIDE_COVERS: isize = 24;
/// Memory for reflections, in bytes.
const REFLECTION_BUDGET: usize = 32 << 20;
/// The covers' height, at least, and at most as a part of the view.
const MIN_HEIGHT: f32 = 180.;
const MAX_SHARE: f32 = 0.75;
/// The resize handle, across the edge between the covers and the list.
const HANDLE: f32 = 8.;

/// Where each cover went on screen, the topmost last.
type Hits = Rc<RefCell<Vec<(usize, Bounds<Pixels>)>>>;

#[derive(Default)]
pub(super) struct CoverFlow {
    /// The cover in the middle; fractional while gliding or dragging.
    pos: f32,
    last_frame: Option<Instant>,
    drag: Option<Drag>,
    /// Covers as drawn in the last frame: for clicks.
    hits: Hits,
    /// Where the bar under the covers was drawn.
    bar: Rc<Cell<Bounds<Pixels>>>,
    /// Wheel travel not yet turned into covers (touchpads send pixels).
    wheel: f32,
    /// The listing it was drawn for: a new one starts at its selection.
    generation: u64,
    /// Where the covers were drawn.
    area: Rc<Cell<Bounds<Pixels>>>,
    /// The height while the handle is dragged; `Settings` otherwise.
    height: Option<f32>,
    resizing: bool,
}

/// Drag payload of the resize handle.
pub(super) struct FlowResize;

struct Drag {
    start: Point<Pixels>,
    start_pos: f32,
    moved: bool,
    /// On the bar under the covers: the position follows the pointer along it.
    on_bar: bool,
    /// Recent positions, for the fling on release.
    samples: VecDeque<(Instant, f32)>,
}

/// A cover to draw.
struct Cover {
    index: usize,
    image: Arc<RenderImage>,
    reflection: Option<Arc<RenderImage>>,
    /// The empty rows under the picture, as a part of its height (icons have some):
    /// the picture itself stands on the floor.
    margin: f32,
    /// An icon (not a picture): drawn smaller.
    icon: bool,
    /// Nothing shows through it: it hides what is behind.
    opaque: bool,
    /// Width over height.
    aspect: f32,
}

/// Where a cover `d` covers away from the middle one goes, for covers `size` wide.
#[derive(Clone, Copy, Debug)]
struct Placement {
    /// From the middle of the view to the cover's middle, in pixels.
    x: f32,
    /// Turn around its vertical axis: covers aside face the middle.
    angle: f32,
    /// How far behind the middle cover's plane it stands.
    depth: f32,
    /// 0 in the middle, 1 once aside.
    aside: f32,
}

fn placement(d: f32, size: f32) -> Placement {
    let aside = d.abs().min(1.);
    // Aside, each cover shows a slice of itself next to the one before.
    let (gap, spacing) = (size * 0.62, size * 0.14);
    let x = d.signum() * (aside * gap + (d.abs() - 1.).max(0.) * spacing);
    Placement { x, angle: d.signum() * TURN * aside, depth: aside * size * 0.5, aside }
}

/// A cover seen from a camera `camera` pixels in front of the middle of the view
/// (`center`, on screen): screen columns and cover columns (`local`, from the cover's
/// middle), and the scale there. Covers aside turn their outer edge toward the viewer.
#[derive(Clone, Copy)]
struct Projection {
    center: f32,
    at: Placement,
    camera: f32,
}

impl Projection {
    fn scale(&self, local: f32) -> f32 {
        self.camera / (self.camera + self.at.depth - local * self.at.angle.sin())
    }

    fn screen_x(&self, local: f32) -> f32 {
        self.center + (self.at.x + local * self.at.angle.cos()) * self.scale(local)
    }

    fn local_x(&self, screen: f32) -> f32 {
        let q = screen - self.center;
        let (sin, cos) = self.at.angle.sin_cos();
        (q * (self.camera + self.at.depth) - self.camera * self.at.x) / (self.camera * cos + q * sin)
    }
}

/// An image's reflection, with what was found out about the image making it.
#[derive(Clone)]
struct Reflection {
    image: Arc<RenderImage>,
    /// The empty rows under the picture, as a part of its height.
    margin: f32,
    /// Every pixel of the image is opaque.
    opaque: bool,
}

/// Reflections of the images drawn, by image; the oldest go past the budget.
#[derive(Default)]
struct Reflections {
    images: HashMap<ImageId, (Reflection, u64, usize)>,
    tick: u64,
    bytes: usize,
}

impl Global for Reflections {}

/// The lower part of `image` upside down, fading out.
fn reflection_of(image: &Arc<RenderImage>, cx: &mut App) -> Option<Reflection> {
    let reflections = cx.default_global::<Reflections>();
    reflections.tick += 1;
    let tick = reflections.tick;
    if let Some((reflection, used, _)) = reflections.images.get_mut(&image.id) {
        *used = tick;
        return Some(reflection.clone());
    }
    let size = image.size(0);
    let (w, h) = (size.width.0.max(0) as usize, size.height.0.max(0) as usize);
    let pixels = image.as_bytes(0)?;
    if w == 0 || h == 0 || pixels.len() < w * h * 4 {
        return None;
    }
    // The picture's last row with anything in it.
    let last = (0..h).rev().find(|&y| pixels[y * w * 4..(y + 1) * w * 4].chunks_exact(4).any(|px| px[3] > 8))?;
    let margin = (h - 1 - last) as f32 / h as f32;
    let opaque = pixels[..w * h * 4].chunks_exact(4).all(|px| px[3] == 255);
    let rows = ((h as f32 * REFLECTION).ceil() as usize).min(last + 1);
    let mut out = vec![0u8; w * rows * 4];
    for r in 0..rows {
        let fade = 0.4 * (1. - r as f32 / rows as f32).powf(1.7);
        let (src, dst) = ((last - r) * w * 4, r * w * 4);
        for x in 0..w {
            let (s, d) = (src + x * 4, dst + x * 4);
            out[d..d + 3].copy_from_slice(&pixels[s..s + 3]);
            out[d + 3] = (pixels[s + 3] as f32 * fade) as u8;
        }
    }
    let bytes = out.len();
    let buffer = image::RgbaImage::from_raw(w as u32, rows as u32, out)?;
    let reflection = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    let reflection = Reflection { image: reflection, margin, opaque };
    let reflections = cx.global_mut::<Reflections>();
    reflections.images.insert(image.id, (reflection.clone(), tick, bytes));
    reflections.bytes += bytes;
    if reflections.bytes > REFLECTION_BUDGET {
        let mut by_age: Vec<(u64, ImageId)> = reflections.images.iter().map(|(id, (_, used, _))| (*used, *id)).collect();
        by_age.sort_unstable();
        let mut dropped = Vec::new();
        for (_, id) in by_age {
            if reflections.bytes <= REFLECTION_BUDGET / 4 * 3 {
                break;
            }
            if let Some((old, _, bytes)) = reflections.images.remove(&id) {
                reflections.bytes -= bytes;
                dropped.push(old.image);
            }
        }
        for old in dropped {
            cx.drop_image(old, None);
        }
    }
    Some(reflection)
}

impl FileManager {
    /// The row the covers head for: the cursor's.
    fn flow_target(&self, cx: &App) -> f32 {
        let d = self.table.read(cx).delegate();
        d.cursor().unwrap_or(0).min(d.len().saturating_sub(1)) as f32
    }

    pub(super) fn render_cover_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.table.read(cx).delegate().len();
        let target = self.flow_target(cx);
        if self.flow.generation != self.load_generation {
            self.flow.generation = self.load_generation;
            self.flow.pos = target;
            self.flow.drag = None;
        }
        // Glide toward the cursor, frame after frame.
        let now = Instant::now();
        if self.flow.drag.is_none() {
            let dt = self.flow.last_frame.map_or(0., |t| (now - t).as_secs_f32().min(0.05));
            self.flow.pos += (target - self.flow.pos) * (1. - (-dt / GLIDE).exp());
            if (target - self.flow.pos).abs() < 0.002 {
                self.flow.pos = target;
            }
        }
        self.flow.pos = self.flow.pos.clamp(0., count.saturating_sub(1) as f32);
        let gliding = self.flow.drag.is_none() && self.flow.pos != target;
        self.flow.last_frame = gliding.then_some(now);
        if gliding {
            window.request_animation_frame();
        }
        let pos = self.flow.pos;

        // Covers from the farthest to the middle one, which is drawn last, on top.
        // Thumbnails are served newest request first: the middle one is asked for last.
        let thumb = if window.scale_factor() > 1.25 { 512 } else { 256 };
        let center = pos.round() as isize;
        let mut covers = Vec::new();
        let around = (1..=SIDE_COVERS).rev().flat_map(|d| [center - d, center + d]).chain([center]);
        for index in around.filter(|&i| i >= 0 && i < count as isize) {
            let Some(e) = self.table.read(cx).delegate().entry(index as usize).cloned() else { continue };
            let (image, icon) = match icons::thumbnail_image(&e, thumb, cx) {
                Some(Some(image)) => (Some(image), false),
                Some(None) => (icons::icon_image(&e, IconSize::Jumbo, cx), true),
                None => (None, false),
            };
            let Some(image) = image else { continue };
            let size = image.size(0);
            let aspect = size.width.0.max(1) as f32 / size.height.0.max(1) as f32;
            let (reflection, margin, opaque) = match reflection_of(&image, cx) {
                Some(r) => (Some(r.image), r.margin, r.opaque),
                None => (None, 0., false),
            };
            covers.push(Cover { index: index as usize, image, reflection, margin, icon, opaque, aspect });
        }
        covers.sort_by(|a, b| (b.index as f32 - pos).abs().total_cmp(&(a.index as f32 - pos).abs()));

        let theme = cx.theme();
        let (top, bottom) =
            if theme.mode.is_dark() { (hsla(0., 0., 0.02, 1.), theme.background) } else { (theme.background, theme.muted) };
        let border = theme.border;
        let hits = self.flow.hits.clone();
        let flow = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| paint_covers(bounds, &covers, pos, bottom, &hits, window),
        )
        .absolute()
        .size_full();

        let this = cx.entity().downgrade();
        let dragging = self.flow.drag.is_some();
        let height = self.flow.height.unwrap_or(Settings::get(cx).flow_height);
        let area = self.flow.area.clone();
        div()
            .id("cover-flow")
            .relative()
            .w_full()
            .h(px(height))
            .min_h(px(MIN_HEIGHT))
            .max_h(relative(MAX_SHARE))
            .flex_shrink_0()
            .overflow_hidden()
            .bg(linear_gradient(180., linear_color_stop(top, 0.), linear_color_stop(bottom, 1.)))
            .border_b_1()
            .border_color(border)
            .on_prepaint(move |bounds, _, _| area.set(bounds))
            .child(flow)
            .children(self.render_flow_caption(pos, count, cx))
            .child(self.render_flow_bar(pos, count, cx))
            .child(self.render_media_filter(cx))
            // Covers take their own presses: no rubber band, no file drag.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.flow_press(ev, window, cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.focus_view(window, cx);
                match this.flow_hit(ev.position) {
                    Some(row) => this.item_menu(row, ev.position, ev.modifiers.shift, window, cx),
                    None => this.show_menu(menu::MenuTarget::background(), ev.position, ev.modifiers.shift, window, cx),
                }
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                cx.stop_propagation();
                this.flow_wheel(ev.delta, window, cx);
            }))
            // While dragging, moves and the release count anywhere in the window.
            .when(dragging, |d| {
                d.child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            let (moves, ups) = (this.clone(), this.clone());
                            window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble {
                                    moves.update(cx, |fm, cx| fm.flow_dragged(e, cx)).ok();
                                }
                            });
                            window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                                    ups.update(cx, |fm, cx| fm.flow_released(e.position, cx)).ok();
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
    }

    /// The middle cover's name, and where it is among them.
    fn render_flow_caption(&self, pos: f32, count: usize, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let index = pos.round() as usize;
        let e = self.table.read(cx).delegate().entry(index)?.clone();
        let s = i18n::t(cx);
        let theme = cx.theme();
        Some(
            v_flex()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(22.))
                .items_center()
                .gap_0p5()
                .child(
                    div()
                        .max_w(relative(0.6))
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(e.shown_name(Settings::get(cx).show_extensions)),
                )
                .child(div().text_xs().text_color(theme.muted_foreground).child((s.item_of)(index + 1, count))),
        )
    }

    /// The bar under the covers: its thumb is where the middle cover is among them all.
    fn render_flow_bar(&self, pos: f32, count: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let bar = self.flow.bar.clone();
        let fraction = if count > 1 { pos / (count - 1) as f32 } else { 0. };
        let thumb = (1. / count.max(1) as f32).max(0.06);
        div()
            .absolute()
            .bottom(px(8.))
            .left(relative(0.3))
            .w(relative(0.4))
            .h(px(6.))
            .rounded_full()
            .bg(theme.muted_foreground.opacity(0.2))
            .when(count > 1, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(fraction * (1. - thumb)))
                        .w(relative(thumb))
                        .rounded_full()
                        .bg(theme.muted_foreground.opacity(0.6)),
                )
            })
            .on_prepaint(move |bounds, _, _| bar.set(bounds))
    }

    /// All files, photos and videos, or photos only.
    fn render_media_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = i18n::t(cx);
        let current = Settings::get(cx).media_filter;
        h_flex()
            .absolute()
            .top_2()
            .right_2()
            .gap_1()
            // Not a press on the covers.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(MediaFilter::ALL.map(|filter| {
                Button::new(("media-filter", filter as usize))
                    .ghost()
                    .xsmall()
                    .label(s.media_filter(filter))
                    .selected(filter == current)
                    .on_click(move |_, _, cx| Settings::update(cx, |s| s.media_filter = filter))
            }))
    }

    /// The cover under `position`, the topmost one.
    fn flow_hit(&self, position: Point<Pixels>) -> Option<usize> {
        self.flow.hits.borrow().iter().rev().find(|(_, bounds)| bounds.contains(&position)).map(|(ix, _)| *ix)
    }

    /// The handle between the covers and the list, for the top of the list: half of it
    /// is over the covers, half over the list. Dragging it resizes the covers, a double
    /// click gives them their height back.
    pub(super) fn render_flow_handle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let color = cx.theme().drag_border;
        let resizing = self.flow.resizing;
        div()
            .id("cover-flow-resize")
            .group("cover-flow-resize")
            .absolute()
            .top(px(-HANDLE / 2.))
            .left_0()
            .right_0()
            .h(px(HANDLE))
            .flex()
            .items_center()
            .cursor_row_resize()
            // Not a press on the covers, nor on the column headers.
            .occlude()
            .child(
                div()
                    .w_full()
                    .h(px(3.))
                    .when(resizing, |d| d.bg(color))
                    .group_hover("cover-flow-resize", |d| d.bg(color.opacity(0.6))),
            )
            .on_drag(FlowResize, |_, _, _, cx| cx.new(|_| super::sidebar::DragGhost))
            .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                if ev.click_count() == 2 {
                    this.flow.height = None;
                    let default = Settings::default().flow_height;
                    Settings::update(cx, |s| s.flow_height = default);
                }
            }))
    }

    /// The handle follows the pointer, within limits that leave the list some rows.
    pub(super) fn flow_resize_moved(&mut self, ev: &DragMoveEvent<FlowResize>, _: &mut Window, cx: &mut Context<Self>) {
        self.flow.resizing = true;
        let area = self.flow.area.get();
        let available = f32::from(area.size.height + self.table_bounds.get().size.height);
        let max = (available * MAX_SHARE).max(MIN_HEIGHT);
        self.flow.height = Some(f32::from(ev.event.position.y - area.top()).clamp(MIN_HEIGHT, max));
        cx.notify();
    }

    /// Mouse released: keep the new height for the next windows and runs.
    pub(super) fn end_flow_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.flow.resizing)
            && let Some(height) = self.flow.height.take()
        {
            Settings::update(cx, |s| s.flow_height = height);
        }
    }

    fn flow_press(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The resize handle's.
        if ev.position.y > self.flow.area.get().bottom() - px(HANDLE / 2.) {
            return;
        }
        self.focus_view(window, cx);
        let on_bar = self.flow.bar.get().dilate(px(6.)).contains(&ev.position);
        if ev.click_count >= 2 && !on_bar {
            if let Some(e) = self.flow_hit(ev.position).and_then(|ix| self.table.read(cx).delegate().entry(ix).cloned()) {
                self.open_entry(e, window, cx);
            }
            return;
        }
        self.flow.drag =
            Some(Drag { start: ev.position, start_pos: self.flow.pos, moved: false, on_bar, samples: VecDeque::new() });
        if on_bar {
            self.flow_dragged_to(ev.position, cx);
        }
        cx.notify();
    }

    fn flow_dragged(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if ev.pressed_button != Some(MouseButton::Left) {
            return self.flow_released(ev.position, cx);
        }
        self.flow_dragged_to(ev.position, cx);
    }

    /// The covers follow the pointer, and the selection the middle cover.
    fn flow_dragged_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let count = self.table.read(cx).delegate().len();
        let Some(drag) = self.flow.drag.as_mut() else { return };
        if count == 0 {
            return;
        }
        let last = (count - 1) as f32;
        let dx = f32::from(position.x - drag.start.x);
        drag.moved |= dx.abs() > 3.;
        let pos = if drag.on_bar {
            let bar = self.flow.bar.get();
            (f32::from(position.x - bar.left()) / f32::from(bar.size.width).max(1.) * last).clamp(0., last)
        } else {
            (drag.start_pos - dx / DRAG_PER_COVER).clamp(0., last)
        };
        let now = Instant::now();
        drag.samples.push_back((now, pos));
        while drag.samples.front().is_some_and(|(t, _)| now - *t > std::time::Duration::from_millis(120)) {
            drag.samples.pop_front();
        }
        self.flow.pos = pos;
        let row = pos.round() as usize;
        if self.table.read(cx).delegate().cursor() != Some(row) {
            self.update_selection(cx, |d| d.select_only(row));
            self.scroll_to_row(row, cx);
        }
        cx.notify();
    }

    /// Released: a click picks the cover under it; a drag goes on a little with the
    /// speed it had, and stops on a cover.
    fn flow_released(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.flow.drag.take() else { return };
        let count = self.table.read(cx).delegate().len();
        if count == 0 {
            return cx.notify();
        }
        let last = (count - 1) as f32;
        let row = if !drag.moved && !drag.on_bar {
            self.flow_hit(position)
        } else {
            let fling = match (drag.samples.front(), drag.samples.back()) {
                (Some((t0, p0)), Some((t1, p1))) if !drag.on_bar && *t1 > *t0 => (p1 - p0) / (*t1 - *t0).as_secs_f32(),
                _ => 0.,
            };
            Some((self.flow.pos + fling * 0.3).round().clamp(0., last) as usize)
        };
        if let Some(row) = row {
            self.update_selection(cx, |d| d.select_only(row));
            self.scroll_to_row(row, cx);
        }
        cx.notify();
    }

    /// The wheel runs through the covers, one per notch.
    fn flow_wheel(&mut self, delta: ScrollDelta, window: &mut Window, cx: &mut Context<Self>) {
        let steps = match delta {
            ScrollDelta::Lines(lines) => {
                let lines = if lines.x.abs() > lines.y.abs() { lines.x } else { lines.y };
                -lines.signum() as isize
            }
            ScrollDelta::Pixels(pixels) => {
                let travel = if pixels.x.abs() > pixels.y.abs() { pixels.x } else { pixels.y };
                self.flow.wheel -= f32::from(travel);
                let steps = (self.flow.wheel / 40.).trunc();
                self.flow.wheel -= steps * 40.;
                steps as isize
            }
        };
        if steps != 0 {
            self.move_cursor(
                move |cursor, _| Some((cursor.unwrap_or(0) as isize + steps).max(0) as usize),
                window,
                cx,
            );
        }
    }
}

/// A cover's outline: its edges from the horizon at scale 1 (on screen, each column
/// scales them by its own perspective), and where it goes across.
struct Shape {
    projection: Projection,
    w: f32,
    h: f32,
    top: f32,
    /// The picture's bottom edge; the picture (not its empty margin) stands there.
    floor: f32,
    /// The reflection's bottom edge (the picture's without one).
    bottom: f32,
    left: f32,
    right: f32,
}

impl Shape {
    fn new(cover: &Cover, pos: f32, size: f32, center: f32, camera: f32, bounds: Bounds<Pixels>) -> Option<Self> {
        let at = placement(cover.index as f32 - pos, size);
        let projection = Projection { center, at, camera };
        let box_size = if cover.icon { size * 0.62 } else { size };
        let (w, h) =
            if cover.aspect >= 1. { (box_size, box_size / cover.aspect) } else { (box_size * cover.aspect, box_size) };
        let (left, right) = (projection.screen_x(-w / 2.), projection.screen_x(w / 2.));
        if right < f32::from(bounds.left()) || left > f32::from(bounds.right()) || right <= left {
            return None;
        }
        let floor = size / 2. + cover.margin * h;
        let reflection_top = floor - cover.margin * h + 2.;
        let bottom = if cover.reflection.is_some() { (reflection_top + h * REFLECTION).max(floor) } else { floor };
        Some(Self { projection, w, h, top: floor - h, floor, bottom, left, right })
    }

    /// Top and bottom on screen of the column at `x`.
    fn edges(&self, horizon: f32, x: f32) -> (f32, f32) {
        let s = self.projection.scale(self.projection.local_x(x));
        (horizon + self.top * s, horizon + self.bottom * s)
    }

    /// Whether the strip `x0..x1`, from `top` to `bottom` on screen, is all behind this.
    fn hides(&self, horizon: f32, x0: f32, x1: f32, top: f32, bottom: f32) -> bool {
        if self.left > x0 || x1 > self.right {
            return false;
        }
        let (mine_top, mine_bottom) = self.edges(horizon, (x0 + x1) / 2.);
        mine_top <= top + 0.5 && mine_bottom >= bottom - 0.5
    }
}

/// Draws the covers, farthest first, and notes where each one went.
fn paint_covers(
    bounds: Bounds<Pixels>,
    covers: &[Cover],
    pos: f32,
    background: Hsla,
    hits: &Hits,
    window: &mut Window,
) {
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let size = (height * 0.58).min(width * 0.3).max(40.);
    // The covers' middle, with room under them for reflections and the caption.
    let horizon = f32::from(bounds.top()) + height * 0.4;
    let center = f32::from(bounds.center().x);
    let camera = size * 2.4;
    // Strips two device pixels wide: their edges never fall between pixels.
    let step = 2. / window.scale_factor();
    let shapes: Vec<Option<Shape>> = covers.iter().map(|c| Shape::new(c, pos, size, center, camera, bounds)).collect();
    let mut drawn = Vec::with_capacity(covers.len());
    for (cover, shape) in covers.iter().zip(&shapes) {
        let Some(shape) = shape else { continue };
        let Shape { projection, w, h, floor, left, right, .. } = *shape;
        let at = projection.at;
        // The next cover toward the middle is drawn over this one: what it hides of
        // this one is not drawn at all. Only a picture with nothing showing through
        // hides anything.
        let d = cover.index as f32 - pos;
        let next = (cover.index as isize - d.signum() as isize).max(0) as usize;
        let occluder = ((next as f32 - pos).abs() < d.abs())
            .then(|| covers.iter().position(|c| c.index == next && c.opaque && !c.icon))
            .flatten()
            .and_then(|ix| shapes[ix].as_ref());
        // Covers aside sink into the background, the more toward the edges.
        let edge = (((left + right) / 2. - center).abs() - width * 0.3).max(0.) / (width * 0.2);
        let fade = (0.3 * at.aside + edge).min(1.);
        let shade = (fade > 0.01).then(|| background.opacity(fade));
        let mut top_most = f32::MAX;
        let mut strip = |x0: f32, x1: f32, window: &mut Window| {
            let (l0, l1) = (projection.local_x(x0), projection.local_x(x1));
            let s = projection.scale(projection.local_x((x0 + x1) / 2.));
            let (u0, u1) = (((l0 + w / 2.) / w).clamp(0., 1.), ((l1 + w / 2.) / w).clamp(0., 1.));
            if u1 - u0 <= f32::EPSILON {
                return;
            }
            let (top, bottom) = (horizon + (floor - h) * s, horizon + floor * s);
            // The reflection starts just under the picture, empty margin or not.
            let r_top = bottom - cover.margin * h * s + 2. * s;
            let r_bottom = r_top + h * REFLECTION * s;
            let lowest = if cover.reflection.is_some() { bottom.max(r_bottom) } else { bottom };
            if occluder.is_some_and(|o| o.hides(horizon, x0, x1, top, lowest)) {
                return;
            }
            top_most = top_most.min(top);
            let full = (x1 - x0) / (u1 - u0);
            let rect = Bounds::from_corners(point(px(x0), px(top)), point(px(x1), px(bottom)));
            let image = Bounds::new(point(px(x0 - u0 * full), px(top)), size_px(full, bottom - top));
            let _ = window.paint_image(rect, image, Corners::default(), cover.image.clone(), 0, false);
            if let Some(reflection) = &cover.reflection {
                let rect = Bounds::from_corners(point(px(x0), px(r_top)), point(px(x1), px(r_bottom)));
                let image = Bounds::new(point(px(x0 - u0 * full), px(r_top)), size_px(full, r_bottom - r_top));
                let _ = window.paint_image(rect, image, Corners::default(), reflection.clone(), 0, false);
            }
            if let Some(shade) = shade {
                let rect = Bounds::from_corners(point(px(x0), px(top)), point(px(x1), px(lowest)));
                window.paint_quad(fill(rect, shade));
            }
        };
        if at.angle.abs() < 0.01 {
            strip(left, right, window);
        } else {
            // Only what is in view.
            let (from, to) = (left.max(f32::from(bounds.left())), right.min(f32::from(bounds.right())));
            for (x0, x1) in strips(from, to, step) {
                strip(x0, x1, window);
            }
        }
        if top_most < f32::MAX {
            let bottom = horizon + (floor - cover.margin * h) * projection.scale(0.);
            drawn.push((cover.index, Bounds::from_corners(point(px(left), px(top_most)), point(px(right), px(bottom)))));
        }
    }
    *hits.borrow_mut() = drawn;
}

/// The strips `step` wide that make `left..right`, their edges on multiples of `step`:
/// the first and the last one are cut. Counted, not added up: sums of floats may stop
/// moving.
fn strips(left: f32, right: f32, step: f32) -> impl Iterator<Item = (f32, f32)> {
    let sound = left.is_finite() && right.is_finite() && step > 0. && right > left;
    // One more on each side: a division may round an edge to the wrong side of a multiple.
    let (first, last) = if sound { ((left / step).floor() as i64 - 1, (right / step).ceil() as i64 + 1) } else { (0, 0) };
    (first..last).filter_map(move |k| {
        let (x0, x1) = ((k as f32 * step).max(left), ((k + 1) as f32 * step).min(right));
        (x1 > x0).then_some((x0, x1))
    })
}

fn size_px(width: f32, height: f32) -> Size<Pixels> {
    size(px(width), px(height))
}

#[cfg(test)]
mod tests {
    use super::{Projection, placement, strips};

    #[test]
    fn strips_make_the_whole_range_and_end() {
        for scale in [1., 1.25, 1.5, 1.75, 2., 2.5, 3.] {
            let step = 2. / scale;
            // Edges on multiples of the step too: adding steps up got stuck on some.
            for k in -400..1500 {
                for (left, width) in [(k as f32 * step, 90.), (k as f32 * step + 0.3, 41.7), (k as f32 * 0.77, 3.)] {
                    let right = left + width;
                    let all: Vec<(f32, f32)> = strips(left, right, step).take(10_000).collect();
                    assert!(all.len() < 200, "{scale} {left}: {} strips", all.len());
                    assert_eq!((all[0].0, all[all.len() - 1].1), (left, right));
                    assert!(all.windows(2).all(|pair| pair[0].1 == pair[1].0), "{scale} {left}: gaps");
                    assert!(all.iter().all(|(x0, x1)| x1 > x0 && x1 - x0 <= step * 1.001), "{scale} {left}: widths");
                }
            }
        }
        assert_eq!(strips(f32::NAN, 10., 2.).count(), 0);
        assert_eq!(strips(0., f32::INFINITY, 2.).count(), 0);
        assert_eq!(strips(10., 10., 2.).count(), 0);
    }

    #[test]
    fn covers_line_up_around_the_middle_one() {
        let size = 200.;
        let middle = placement(0., size);
        assert_eq!((middle.x, middle.angle, middle.depth), (0., 0., 0.));
        let xs: Vec<f32> = [-3., -2., -1., -0.5, 0., 0.5, 1., 2., 3.].iter().map(|&d| placement(d, size).x).collect();
        assert!(xs.windows(2).all(|pair| pair[0] < pair[1]), "in order: {xs:?}");
        // Aside, they all turn as much and stand as far back.
        assert_eq!(placement(2., size).angle, placement(5., size).angle);
        assert_eq!(placement(-2., size).depth, placement(2., size).depth);
    }

    #[test]
    fn perspective_maps_both_ways_and_brings_outer_edges_forward() {
        for d in [-3., -1., -0.4, 0.7, 2.] {
            let projection = Projection { center: 500., at: placement(d, 200.), camera: 480. };
            for local in [-100., -37., 0., 12., 100.] {
                let back = projection.local_x(projection.screen_x(local));
                assert!((back - local).abs() < 1e-2, "{d}: {local} came back as {back}");
            }
            // The edge away from the middle is nearer, so taller.
            let (outer, inner) = if d > 0. { (100., -100.) } else { (-100., 100.) };
            assert!(projection.scale(outer) > projection.scale(inner));
        }
    }
}
