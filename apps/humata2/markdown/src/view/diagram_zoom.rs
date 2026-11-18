//! Zooming into and moving around a diagram: in place in the document, or
//! opened large in place of it.
//!
//! In place, pinch or ⌘-scroll (Ctrl-scroll elsewhere) zooms about the
//! pointer and a drag pans; a plain scroll still scrolls the document. A
//! hover or focus shows zoom out, zoom in, Fit and Open; a double-click opens
//! too. Opened, the diagram fills the view with the same zoom and pan, plain
//! scroll pans, and Escape returns. `+`, `-` and `0` zoom a focused diagram.
//!
//! Zooming only repaints: no layout is measured again. Once a zoom has
//! settled the diagram is drawn again at a size that is sharp at it, and the
//! nearest sharper picture stands in until then.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Bounds, ClickEvent, DispatchPhase, Div, FocusHandle, ImageSource, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement,
    PinchEvent, Pixels, Point, RenderImage, Role, ScrollEvent, StatefulInteractiveElement, Styled,
    StyledImage as _, Window, canvas, div, img, px, relative,
};

use super::{Shared, read};
use crate::mermaid::{self, DiagramBlock, DiagramEntry, Drawn};
use crate::theme::MarkdownTheme;
use crate::zoom::{self, BASE_RASTER, KeyCommand, STEP, Vec2, Viewport};

/// How long a zoom must rest before the diagram is drawn again for it.
const SETTLE: Duration = Duration::from_millis(150);
/// The opened diagram's height when the view sizes to its content.
const OPEN_HEIGHT: f32 = 520.;
/// The pixels a wheel line scrolls.
const LINE: f32 = 20.;

/// Where a diagram is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Place {
    InPlace,
    Open,
}

/// What a zoomable frame is: an element with an id (plain, or observed
/// for tests) that takes children, style and pointer handlers.
pub(super) trait Frame: StatefulInteractiveElement + ParentElement + Styled + IntoElement {}
impl<E: StatefulInteractiveElement + ParentElement + Styled + IntoElement> Frame for E {}

/// A diagram in a place: the first byte of its block, and the place.
type Key = (usize, Place);

/// One diagram's zoom in one place.
struct DiagramZoom {
    /// None while fitted.
    viewport: Option<Viewport>,
    /// The frame the diagram is shown in, in window coordinates, from the
    /// last paint.
    frame: Option<Bounds<Pixels>>,
    /// The diagram's size at 100%.
    content: Vec2,
    /// Where the pointer was at the last drag step.
    drag: Option<Point<Pixels>>,
    hovered: bool,
    focus: FocusHandle,
    /// Bumped by every change of scale, so only the last one settles.
    generation: u64,
    settled: bool,
}

impl DiagramZoom {
    /// The zoom shown, once the frame is known.
    fn current(&self) -> Option<(Viewport, Vec2)> {
        let frame = size(&self.frame?);
        let viewport = self.viewport.unwrap_or_else(|| Viewport::fit(frame, self.content));
        Some((viewport.clamped(frame, self.content), frame))
    }
}

/// A diagram opened large.
#[derive(Clone)]
pub(super) struct OpenDiagram {
    start: usize,
    source: Arc<str>,
    name: String,
}

/// What a frame is drawn from: a copy of its zoom, taken under the lock.
struct Shown {
    viewport: Option<Viewport>,
    frame: Option<Vec2>,
    scale: Option<f32>,
    hovered: bool,
    dragging: bool,
    settled: bool,
    focus: FocusHandle,
}

/// Every diagram's zoom, shared with the text view's callbacks.
#[derive(Default)]
pub(super) struct Zooms {
    diagrams: Mutex<HashMap<Key, DiagramZoom>>,
    open: Mutex<Option<OpenDiagram>>,
}

/// A poisoned lock still holds whole data here; keep using it.
fn lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn size(bounds: &Bounds<Pixels>) -> Vec2 {
    Vec2::new(f32::from(bounds.size.width), f32::from(bounds.size.height))
}

fn at(point: Point<Pixels>, frame: &Bounds<Pixels>) -> Vec2 {
    Vec2::new(
        f32::from(point.x) - f32::from(frame.origin.x),
        f32::from(point.y) - f32::from(frame.origin.y),
    )
}

/// A picture's size at 100%: it is drawn at [`BASE_RASTER`] times that.
fn natural(image: &RenderImage) -> Vec2 {
    let size = image.size(0);
    let raster = f32::from(BASE_RASTER);
    Vec2::new(size.width.0 as f32 / raster, size.height.0 as f32 / raster)
}

impl Zooms {
    /// The diagram's zoom, made fitted the first time it is shown.
    fn shown(&self, key: Key, content: Vec2, cx: &App) -> Shown {
        let mut diagrams = lock(&self.diagrams);
        let zoom = diagrams.entry(key).or_insert_with(|| DiagramZoom {
            viewport: None,
            frame: None,
            content,
            drag: None,
            hovered: false,
            focus: cx.focus_handle(),
            generation: 0,
            settled: true,
        });
        if zoom.content != content {
            zoom.content = content;
            zoom.viewport = None;
        }
        let current = zoom.current();
        Shown {
            viewport: zoom.viewport,
            frame: current.map(|(_, frame)| frame),
            scale: current.map(|(viewport, _)| viewport.scale),
            hovered: zoom.hovered,
            dragging: zoom.drag.is_some(),
            settled: zoom.settled,
            focus: zoom.focus.clone(),
        }
    }

    /// Note where the frame was painted. True when its size changed while
    /// that matters, so the frame should be drawn again.
    fn measured(&self, key: Key, frame: Bounds<Pixels>) -> bool {
        let mut diagrams = lock(&self.diagrams);
        let Some(zoom) = diagrams.get_mut(&key) else { return false };
        let resized = zoom.frame.map(|old| old.size) != Some(frame.size);
        zoom.frame = Some(frame);
        resized && (key.1 == Place::Open || zoom.viewport.is_some())
    }

    /// Change the zoom. Returns the generation to settle when the scale
    /// changed, or None when nothing did.
    fn change(&self, key: Key, change: impl FnOnce(Viewport, Vec2, &Bounds<Pixels>, Vec2) -> Viewport) -> Option<Option<u64>> {
        let mut diagrams = lock(&self.diagrams);
        let zoom = diagrams.get_mut(&key)?;
        let bounds = zoom.frame?;
        let (before, frame) = zoom.current()?;
        let after = change(before, frame, &bounds, zoom.content).clamped(frame, zoom.content);
        if after == before {
            return None;
        }
        zoom.viewport = (!after.is_fit(frame, zoom.content)).then_some(after);
        if after.scale == before.scale {
            return Some(None);
        }
        zoom.generation += 1;
        zoom.settled = false;
        Some(Some(zoom.generation))
    }

    /// Mark the zoom settled if nothing changed it since `generation`.
    fn settle(&self, key: Key, generation: u64) -> bool {
        let mut diagrams = lock(&self.diagrams);
        match diagrams.get_mut(&key) {
            Some(zoom) if zoom.generation == generation && !zoom.settled => {
                zoom.settled = true;
                true
            }
            _ => false,
        }
    }

    fn hover(&self, key: Key, hovered: bool) -> bool {
        let mut diagrams = lock(&self.diagrams);
        let Some(zoom) = diagrams.get_mut(&key) else { return false };
        let changed = zoom.hovered != hovered;
        zoom.hovered = hovered;
        changed
    }

    /// Start a drag at `point`; false when the diagram isn't zoomed, so
    /// there is nothing to move.
    fn grab(&self, key: Key, point: Point<Pixels>) -> bool {
        let mut diagrams = lock(&self.diagrams);
        let Some(zoom) = diagrams.get_mut(&key) else { return false };
        let movable = key.1 == Place::Open || zoom.viewport.is_some();
        if movable {
            zoom.drag = Some(point);
        }
        movable
    }

    /// The pointer's move since the last drag step.
    fn dragged(&self, key: Key, point: Point<Pixels>) -> Option<Vec2> {
        let mut diagrams = lock(&self.diagrams);
        let zoom = diagrams.get_mut(&key)?;
        let last = zoom.drag.replace(point)?;
        Some(Vec2::new(f32::from(point.x - last.x), f32::from(point.y - last.y)))
    }

    fn release(&self, key: Key) -> bool {
        lock(&self.diagrams).get_mut(&key).is_some_and(|zoom| zoom.drag.take().is_some())
    }

    fn focus(&self, key: Key) -> Option<FocusHandle> {
        lock(&self.diagrams).get(&key).map(|zoom| zoom.focus.clone())
    }

    /// The diagram opened large, if one is.
    pub(super) fn opened(&self) -> Option<OpenDiagram> {
        lock(&self.open).clone()
    }

    /// Forget the zoom of every diagram whose block is gone, and close the
    /// opened one if its block is. True when the opened one closed.
    pub(super) fn retain(&self, starts: &HashSet<usize>) -> bool {
        lock(&self.diagrams).retain(|(start, _), _| starts.contains(start));
        let mut open = lock(&self.open);
        let gone = open.as_ref().is_some_and(|open| !starts.contains(&open.start));
        if gone {
            *open = None;
        }
        gone
    }
}

impl Shared {
    fn redraw(&self, cx: &mut App) {
        // The view may be gone; nothing to draw then.
        let _ = self.view.update(cx, |_, cx| cx.notify());
    }

    /// Apply a zoom change, redraw, and draw it sharp once it rests.
    fn zoom(self: &Arc<Self>, key: Key, cx: &mut App, change: impl FnOnce(Viewport, Vec2, &Bounds<Pixels>, Vec2) -> Viewport) {
        let Some(rescaled) = self.zooms.change(key, change) else { return };
        if let Some(generation) = rescaled {
            let shared = self.clone();
            let timer = cx.background_executor().timer(SETTLE);
            cx.spawn(async move |cx| {
                timer.await;
                if shared.zooms.settle(key, generation) {
                    cx.update(|cx| shared.redraw(cx));
                }
            })
            .detach();
        }
        self.redraw(cx);
    }

    fn zoom_by(self: &Arc<Self>, key: Key, factor: f32, cx: &mut App) {
        self.zoom(key, cx, |viewport, frame, _, content| viewport.zoomed(factor, frame, content));
    }

    fn fit(self: &Arc<Self>, key: Key, cx: &mut App) {
        self.zoom(key, cx, |_, frame, _, content| Viewport::fit(frame, content));
    }

    fn actual_size(self: &Arc<Self>, key: Key, cx: &mut App) {
        self.zoom(key, cx, |_, frame, _, content| Viewport::actual(frame, content));
    }

    /// The sharpest picture there is for showing the diagram at `scale`,
    /// drawing a sharper one when the zoom has settled.
    fn picture(&self, source: &Arc<str>, drawn: &Drawn, scale: Option<f32>, settled: bool, window: &Window, cx: &mut App) -> Arc<RenderImage> {
        let Some(scale) = scale else { return drawn.image.clone() };
        let theme = read(&self.look).mermaid_key;
        let wanted = zoom::raster_for(scale, window.scale_factor(), natural(&drawn.image));
        if wanted == BASE_RASTER {
            return drawn.image.clone();
        }
        if settled && self.diagrams.begin(source, theme, wanted) {
            self.start_raster(source.clone(), theme, wanted, drawn.markup.clone(), cx);
        }
        for raster in zoom::rasters_down_from(wanted) {
            if raster == BASE_RASTER {
                break;
            }
            if let Some(DiagramEntry::Ready(sharper)) = self.diagrams.get(source, theme, raster) {
                return sharper.image;
            }
        }
        drawn.image.clone()
    }

    fn start_raster(&self, source: Arc<str>, theme: u64, raster: u8, markup: Arc<str>, cx: &mut App) {
        let svg = cx.svg_renderer();
        let drawing = {
            let markup = markup.clone();
            cx.background_executor()
                .spawn(async move { mermaid::raster(&markup, raster, &svg) })
        };
        let cache = self.diagrams.clone();
        let view = self.view.clone();
        cx.spawn(async move |cx| {
            let entry = match drawing.await {
                Ok(image) => DiagramEntry::Ready(Drawn { image, markup }),
                Err(error) => DiagramEntry::Failed(Arc::new(error)),
            };
            let evicted = cache.finish(source, theme, raster, entry);
            cx.update(|cx| {
                for image in evicted {
                    cx.drop_image(image, None);
                }
                let _ = view.update(cx, |_, cx| cx.notify());
            });
        })
        .detach();
    }

    /// Measure a frame's first child, the diagram's frame, as it is painted.
    fn measure(self: &Arc<Self>, key: Key, frame: Div) -> Div {
        let shared = self.clone();
        frame.on_children_prepainted(move |children, _, cx| {
            let Some(bounds) = children.first() else { return };
            if shared.zooms.measured(key, *bounds) {
                let shared = shared.clone();
                cx.defer(move |cx| shared.redraw(cx));
            }
        })
    }

    /// The pointer and key handling every zoomable frame has.
    fn zoomable<E: Frame>(self: &Arc<Self>, key: Key, shown: &Shown, element: E) -> E {
        let open = key.1 == Place::Open;
        let movable = open || shown.viewport.is_some();
        element
            .track_focus(&shown.focus)
            .when(movable, |frame| if shown.dragging { frame.cursor_grabbing() } else { frame.cursor_grab() })
            .on_scroll({
                let shared = self.clone();
                move |event: &ScrollEvent, _, cx| {
                    let delta = event.delta.pixel_delta(px(LINE));
                    if event.modifiers.platform || event.modifiers.control {
                        let factor = zoom::scroll_factor(f32::from(delta.y));
                        shared.zoom(key, cx, |viewport, frame, bounds, content| {
                            viewport.zoomed_about(at(event.position, bounds), factor, frame, content)
                        });
                        cx.stop_propagation();
                    } else if open {
                        let delta = Vec2::new(f32::from(delta.x), f32::from(delta.y));
                        shared.zoom(key, cx, |viewport, frame, _, content| viewport.panned(delta, frame, content));
                        cx.stop_propagation();
                    }
                }
            })
            .on_pinch({
                let shared = self.clone();
                move |event: &PinchEvent, _, cx| {
                    let factor = zoom::pinch_factor(event.delta);
                    shared.zoom(key, cx, |viewport, frame, bounds, content| {
                        viewport.zoomed_about(at(event.position, bounds), factor, frame, content)
                    });
                    cx.stop_propagation();
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let shared = self.clone();
                let focus = shown.focus.clone();
                move |event: &MouseDownEvent, window, cx| {
                    // After the text view under it has handled the press,
                    // which takes focus for its selection.
                    let focus = focus.clone();
                    window.defer(cx, move |window, cx| window.focus(&focus, cx));
                    if shared.zooms.grab(key, event.position) {
                        // A drag on a zoomed diagram moves it, rather than
                        // selecting text.
                        cx.stop_propagation();
                        shared.redraw(cx);
                    }
                }
            })
            .on_key_down({
                let shared = self.clone();
                move |event: &KeyDownEvent, window, cx| {
                    let keystroke = &event.keystroke;
                    let modifiers = keystroke.modifiers;
                    let chord = modifiers.platform || modifiers.control || modifiers.alt;
                    match zoom::key_command(&keystroke.key, keystroke.key_char.as_deref(), chord) {
                        Some(KeyCommand::ZoomIn) => shared.zoom_by(key, STEP, cx),
                        Some(KeyCommand::ZoomOut) => shared.zoom_by(key, 1. / STEP, cx),
                        Some(KeyCommand::Fit) => shared.fit(key, cx),
                        Some(KeyCommand::Close) if open => shared.close(window, cx),
                        Some(KeyCommand::Close) | None => return,
                    }
                    cx.stop_propagation();
                }
            })
            .children(shown.dragging.then(|| self.drag_listener(key)))
    }

    /// While a drag lasts, follow the pointer anywhere in the window.
    fn drag_listener(self: &Arc<Self>, key: Key) -> AnyElement {
        let shared = self.clone();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                window.on_mouse_event({
                    let shared = shared.clone();
                    move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        if event.pressed_button != Some(MouseButton::Left) {
                            if shared.zooms.release(key) {
                                shared.redraw(cx);
                            }
                            return;
                        }
                        if let Some(delta) = shared.zooms.dragged(key, event.position) {
                            shared.zoom(key, cx, |viewport, frame, _, content| viewport.panned(delta, frame, content));
                        }
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && event.button == MouseButton::Left && shared.zooms.release(key) {
                        shared.redraw(cx);
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    /// The diagram positioned at its zoom inside a frame of `frame`'s size.
    fn zoomed_picture(image: Arc<RenderImage>, viewport: Viewport, content: Vec2) -> impl IntoElement {
        img(ImageSource::Render(image))
            .absolute()
            .left(px(viewport.offset.x))
            .top(px(viewport.offset.y))
            .w(px(content.x * viewport.scale))
            .h(px(content.y * viewport.scale))
    }

    /// A ready diagram in the document: as drawn until zoomed, then clipped
    /// to the same frame at its zoom.
    pub(super) fn render_zoomable<F: Frame>(
        self: &Arc<Self>,
        block: &DiagramBlock,
        name: &str,
        drawn: &Drawn,
        frame: impl FnOnce(Div) -> F,
        window: &mut Window,
        cx: &mut App,
    ) -> F {
        let (start, source) = (block.bytes.start, &block.source);
        let key = (start, Place::InPlace);
        let content = natural(&drawn.image);
        let shown = self.zooms.shown(key, content, cx);
        let theme = read(&self.look).theme.clone();
        let body = match (shown.viewport, shown.frame) {
            (Some(viewport), Some(size)) => {
                let image = self.picture(source, drawn, shown.scale, shown.settled, window, cx);
                let viewport = viewport.clamped(size, content);
                div()
                    .relative()
                    .overflow_hidden()
                    .w(px(size.x))
                    .h(px(size.y))
                    .max_w(relative(1.))
                    .child(Self::zoomed_picture(image, viewport, content))
                    .into_any_element()
            }
            // Drawn at twice the pixels; shown at its own size or narrower.
            _ => img(ImageSource::Render(drawn.image.clone()))
                .w(px(content.x))
                .h(px(content.y))
                .max_w(relative(1.))
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
        };
        let controls = (shown.hovered || shown.focus.is_focused(window)).then(|| {
            div()
                .absolute()
                .top_1()
                .right_1()
                .flex()
                .gap_1()
                .p(px(2.))
                .rounded_sm()
                .bg(theme.background.opacity(0.85))
                .child(self.button(key, "md-diagram-zoom-out", "Zoom out", "−", &theme, |shared, key, _, cx| {
                    shared.zoom_by(key, 1. / STEP, cx)
                }))
                .child(self.button(key, "md-diagram-zoom-in", "Zoom in", "+", &theme, |shared, key, _, cx| {
                    shared.zoom_by(key, STEP, cx)
                }))
                .child(self.button(key, "md-diagram-fit", "Fit", "Fit", &theme, |shared, key, _, cx| shared.fit(key, cx)))
                .child(self.button(key, "md-diagram-open", "Open", "Open", &theme, {
                    let source = source.clone();
                    let name = name.to_owned();
                    move |shared, key, window, cx| shared.open(key.0, source.clone(), name.clone(), window, cx)
                }))
        });
        let element = frame(self.measure(key, div()))
            .relative()
            .flex()
            .justify_center()
            .child(body)
            .children(controls)
            .on_hover({
                let shared = self.clone();
                move |hovered, _, cx| {
                    if shared.zooms.hover(key, *hovered) {
                        shared.redraw(cx);
                    }
                }
            })
            .on_click({
                let shared = self.clone();
                let source = source.clone();
                let name = name.to_owned();
                move |event: &ClickEvent, window, cx| {
                    if event.click_count() == 2 {
                        shared.open(start, source.clone(), name.clone(), window, cx);
                    }
                }
            });
        self.zoomable(key, &shown, element)
    }

    /// A quiet text button for a diagram at `start`. Its press never
    /// reaches the diagram under it.
    fn button(
        self: &Arc<Self>,
        key: Key,
        id: &'static str,
        label: &'static str,
        glyph: &'static str,
        theme: &MarkdownTheme,
        press: impl Fn(&Arc<Self>, Key, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let shared = self.clone();
        div()
            .id((id, key.0))
            .test_support()
            .role(Role::Button)
            .aria_label(label)
            .px_1()
            .rounded_sm()
            .text_size(px(11.))
            .text_color(theme.muted)
            .hover({
                let (foreground, surface) = (theme.foreground, theme.surface);
                move |button| button.text_color(foreground).bg(surface)
            })
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_: &ClickEvent, window, cx| press(&shared, key, window, cx))
            .child(glyph)
    }

    /// Open the diagram at `start` large, in place of the document.
    fn open(self: &Arc<Self>, start: usize, source: Arc<str>, name: String, window: &mut Window, cx: &mut App) {
        let key = (start, Place::Open);
        *lock(&self.zooms.open) = Some(OpenDiagram { start, source, name });
        // Made here so it can take focus before the view is drawn.
        let focus = cx.focus_handle();
        let mut diagrams = lock(&self.zooms.diagrams);
        let content = match diagrams.get_mut(&(start, Place::InPlace)) {
            Some(in_place) => {
                // Its frame isn't drawn while open, so it won't hear the
                // pointer leave.
                in_place.hovered = false;
                in_place.drag = None;
                in_place.content
            }
            None => Vec2::default(),
        };
        diagrams.insert(
            key,
            DiagramZoom {
                viewport: None,
                frame: None,
                content,
                drag: None,
                hovered: false,
                focus: focus.clone(),
                generation: 0,
                settled: true,
            },
        );
        drop(diagrams);
        // Deferred, so it comes after the focus the press under it deferred.
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        self.redraw(cx);
    }

    /// Back to the document, with the diagram that was open focused.
    fn close(self: &Arc<Self>, window: &mut Window, cx: &mut App) {
        let Some(open) = lock(&self.zooms.open).take() else { return };
        lock(&self.zooms.diagrams).remove(&(open.start, Place::Open));
        if let Some(focus) = self.zooms.focus((open.start, Place::InPlace)) {
            window.focus(&focus, cx);
        }
        self.redraw(cx);
    }

    /// The opened diagram, filling the view: a toolbar over it, and the
    /// same zoom and pan as in place.
    pub(super) fn render_open(self: &Arc<Self>, open: &OpenDiagram, fill: bool, window: &mut Window, cx: &mut App) -> AnyElement {
        let key = (open.start, Place::Open);
        let (theme, theme_key, mermaid) = {
            let look = read(&self.look);
            (look.theme.clone(), look.mermaid_key, look.mermaid.clone())
        };
        let drawn = match self.diagrams.get(&open.source, theme_key, BASE_RASTER) {
            Some(DiagramEntry::Ready(drawn)) => Some(drawn),
            Some(DiagramEntry::Rendering) => None,
            _ => {
                // Dropped from the cache since it was opened; draw it again.
                if self.diagrams.begin(&open.source, theme_key, BASE_RASTER) {
                    self.start_diagram(open.source.clone(), mermaid, theme_key, cx);
                }
                None
            }
        };
        let content = drawn.as_ref().map(|drawn| natural(&drawn.image)).unwrap_or_default();
        let shown = self.zooms.shown(key, content, cx);
        let percent = shown.scale.map(|scale| format!("{:.0}%", scale * 100.)).unwrap_or_default();
        let start = open.start;
        let toolbar = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(theme.muted)
                    .child(open.name.clone())
                    .test_text(open.name.clone()),
            )
            .child(div().text_size(px(11.)).text_color(theme.muted).child(percent.clone()).test_text(percent))
            .child(self.button(key, "md-diagram-open-zoom-out", "Zoom out", "−", &theme, |shared, key, _, cx| {
                shared.zoom_by(key, 1. / STEP, cx)
            }))
            .child(self.button(key, "md-diagram-open-zoom-in", "Zoom in", "+", &theme, |shared, key, _, cx| {
                shared.zoom_by(key, STEP, cx)
            }))
            .child(self.button(key, "md-diagram-open-fit", "Fit", "Fit", &theme, |shared, key, _, cx| shared.fit(key, cx)))
            .child(self.button(key, "md-diagram-open-actual", "Actual size", "100%", &theme, |shared, key, _, cx| {
                shared.actual_size(key, cx)
            }))
            .child(self.button(key, "md-diagram-open-close", "Close", "Close", &theme, |shared, _, window, cx| {
                shared.close(window, cx)
            }));
        let picture = match (&drawn, shown.frame, shown.viewport) {
            (Some(drawn), Some(size), viewport) => {
                let viewport = viewport.unwrap_or_else(|| Viewport::fit(size, content)).clamped(size, content);
                let image = self.picture(&open.source, drawn, shown.scale, shown.settled, window, cx);
                Self::zoomed_picture(image, viewport, content).into_any_element()
            }
            // Before the area is measured, fitted.
            (Some(drawn), None, _) => img(ImageSource::Render(drawn.image.clone()))
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            (None, _, _) => {
                let shown = format!("Drawing diagram: {}", open.name);
                div().p_2().text_color(theme.muted).child(shown.clone()).test_text(shown).into_any_element()
            }
        };
        let area = self
            .measure(key, div())
            .id(("md-diagram-open-area", start))
            .test_support()
            .role(Role::Image)
            .aria_label(format!("Diagram: {}", open.name))
            .relative()
            .w_full()
            .when(fill, |area| area.flex_1().min_h(px(120.)))
            .when(!fill, |area| area.h(px(OPEN_HEIGHT)))
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .child(div().absolute().top_0().left_0().size_full().overflow_hidden().child(picture));
        div()
            .id(("md-diagram-opened", start))
            .test_support()
            .role(Role::Group)
            .aria_label(format!("Opened diagram: {}", open.name))
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .when(fill, |root| root.size_full())
            .when(!fill, |root| root.w_full())
            .child(toolbar)
            .child(self.zoomable(key, &shown, area))
            .into_any_element()
    }
}
