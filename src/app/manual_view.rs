//! The manual on screen.
//!
//! A layer surface covers the screen with a dark backdrop, drawn once; the
//! page and the help row are subsurfaces of it. The page's buffer is the
//! rendered page, uploaded once: `wp_viewporter` crops and scales it to what
//! the view shows, so zooming and moving only change the viewport and the
//! subsurface's position (a frame costs the compositor a blit, the overlay
//! nothing). A zoom shows the rendering there is, scaled, and the sharp one
//! is rendered once the zoom settles. Without `wp_viewporter` the visible
//! part is cropped and scaled here instead.
//!
//! The backdrop never takes the keyboard: the panel keeps it, so the overlay
//! stays open (see app.rs).

use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::calloop::channel::{self as loop_channel, Event};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::shell::wlr_layer::Anchor;
use tiny_skia::Pixmap;
use x11rb::protocol::xproto;

use super::display::{Piece, Spec, Sub, SurfaceId};
use super::{BTN_LEFT, BTN_RIGHT, FRAME, Outcome, PointerInput, PointerKind, State};
use crate::input::{Input, Nav};
use crate::manual::{PageRequest, Rendered};
use crate::theme::Rgba;
use crate::ui::help::{Help, Prompt};
use crate::ui::viewer::{Placement, Viewer};

/// The backdrop behind the page, over everything else (opaque: nothing
/// behind it needs blending).
const BACKDROP: Rgba = Rgba(0x0000_00FF);
/// How long a zoom must have settled before the page is rendered sharp.
const SETTLE: Duration = Duration::from_millis(200);
/// How much a notch of the mouse wheel zooms.
const WHEEL_ZOOM: f32 = 1.15;
/// How far the pointer moves before a press is a drag, not a click.
const DRAG_THRESHOLD: f64 = 6.0;

/// The manual's surfaces while it is open.
pub(super) struct ManualScreen {
    backdrop: Piece,
    page: Option<Sub>,
    help: Option<Sub>,
    /// The rendering the page's buffer holds.
    shown: Option<PageRequest>,
    placement: Option<Placement>,
    /// The help row as last drawn (prompts and page number).
    prompts: Option<(Vec<Prompt>, String)>,
}

/// The manual's state across openings and its input.
#[derive(Default)]
pub(super) struct ManualState {
    pub(super) viewer: Option<Viewer>,
    pub(super) screen: Option<ManualScreen>,
    /// The directions and shoulders held, in the order pressed.
    held: Vec<Nav>,
    /// When the motion timer last ran (None: it is not running).
    moving: Option<Instant>,
    /// When a zoom by the wheel stops counting as going on.
    settle_at: Option<Instant>,
    /// Where the left button went down (screen coordinates) and whether
    /// it has dragged the page since.
    drag: Option<((f64, f64), bool)>,
}

impl State {
    /// Show the manual over everything, where it was left.
    pub(super) fn open_manual(&mut self) {
        if self.manual.screen.is_some() {
            return;
        }
        let Some(manual) = self.content.manual.as_mut() else {
            return;
        };
        if self.manual.viewer.is_none() {
            let (sender, pages) = loop_channel::channel::<Rendered>();
            let inserted = self.handle.insert_source(pages, |event, &mut (), state| {
                if let Event::Msg(rendered) = event {
                    state.page_rendered(rendered);
                }
            });
            if let Err(err) = inserted {
                log::error!("cannot watch the manual renderer: {}", err.error);
                return;
            }
            manual.start(move |rendered| {
                // the overlay may be closing: nothing to show it on then
                sender.send(rendered).ok();
            });
            let view = manual_view_size(self.dim.size, self.help.as_ref());
            self.manual.viewer = Some(Viewer::new(manual.pages(), view));
        }
        // the panel keeps the keyboard: losing it would close the overlay
        let backdrop = self.display.create_piece(Spec {
            anchor: Anchor::all(),
            size: (0, 0),
            margin: [0; 4],
            keyboard: false,
            fade: false,
        });
        self.manual.screen = Some(ManualScreen {
            page: Sub::new(&mut self.display, &backdrop, true, false),
            help: Sub::new(&mut self.display, &backdrop, false, false),
            backdrop,
            shown: None,
            placement: None,
            prompts: None,
        });
        if !self.display.has_subsurfaces() {
            log::warn!("no wl_subcompositor: the manual cannot be shown");
        }
        self.hover(None);
    }

    /// Back to the menu.
    pub(super) fn close_manual(&mut self) {
        self.manual.screen = None;
        self.manual.held.clear();
        self.manual.drag = None;
    }

    pub(super) fn manual_is_open(&self) -> bool {
        self.manual.screen.is_some()
    }

    /// Whether a surface is one of the manual's.
    pub(super) fn is_manual_surface(&self, surface: &SurfaceId) -> bool {
        self.manual.screen.as_ref().is_some_and(|screen| {
            screen.backdrop.is(surface)
                || screen.page.as_ref().is_some_and(|page| page.is(surface))
                || screen.help.as_ref().is_some_and(|help| help.is(surface))
        })
    }

    /// The backdrop has its size: draw it, once, and what is on it.
    pub(super) fn manual_configured(&mut self, surface: &SurfaceId, size: (u32, u32)) -> bool {
        let Some(screen) = self
            .manual
            .screen
            .as_mut()
            .filter(|screen| screen.backdrop.is(surface))
        else {
            return false;
        };
        let first = screen.backdrop.size == (0, 0);
        screen.backdrop.size = size;
        if first && let Some(mut pixmap) = Pixmap::new(size.0.max(1), size.1.max(1)) {
            pixmap.fill(BACKDROP.to_color());
            screen.backdrop.present(&mut self.display, &pixmap);
            self.show_page(true);
        }
        true
    }

    /// The current page's size in points.
    fn page_points(&self) -> Option<(f32, f32)> {
        let viewer = self.manual.viewer.as_ref()?;
        Some(self.content.manual.as_ref()?.page_size(viewer.page))
    }

    /// Put the page on screen as the viewer says. `sharp`: also render it
    /// at the size it is shown at (not while a zoom goes on), and the pages
    /// either side ahead.
    fn show_page(&mut self, sharp: bool) {
        let (Some(viewer), Some(manual), Some(screen)) = (
            self.manual.viewer.as_ref(),
            self.content.manual.as_mut(),
            self.manual.screen.as_mut(),
        ) else {
            return;
        };
        if screen.backdrop.size == (0, 0) {
            return;
        }
        let points = manual.page_size(viewer.page);
        let (width, height) = viewer.render_size(points);
        let wanted = PageRequest {
            page: viewer.page,
            width,
            height,
        };
        if sharp {
            manual.want(wanted);
            let ahead: Vec<PageRequest> = [viewer.page + 1, viewer.page.saturating_sub(1)]
                .into_iter()
                .filter(|page| (1..=manual.pages()).contains(page))
                .map(|page| {
                    let (width, height) = viewer.render_size(manual.page_size(page));
                    PageRequest {
                        page,
                        width,
                        height,
                    }
                })
                .collect();
            manual.ahead(&ahead);
        }
        let placement = viewer.placement(points);
        let image = manual
            .get(&wanted)
            .map(|image| (wanted, image))
            .or_else(|| manual.best(viewer.page));
        if let Some(page) = screen.page.as_mut() {
            match (image, page.viewport()) {
                (None, _) => {} // the page shown before stays until this one is ready
                (Some((request, image)), Some(viewport)) => {
                    if screen.shown != Some(request) {
                        page.show_pixels(
                            &mut self.display,
                            image.width,
                            image.height,
                            &image.pixels,
                        );
                        screen.shown = Some(request);
                        screen.placement = None;
                    }
                    if screen.placement != Some(placement) {
                        let [left, top, right, bottom] = placement.source;
                        let (image_width, image_height) =
                            (f64::from(image.width), f64::from(image.height));
                        viewport.set_source(
                            f64::from(left) * image_width,
                            f64::from(top) * image_height,
                            f64::from(right - left) * image_width,
                            f64::from(bottom - top) * image_height,
                        );
                        viewport.set_destination(placement.width as i32, placement.height as i32);
                    }
                }
                (Some((request, image)), None) => {
                    if screen.shown != Some(request) || screen.placement != Some(placement) {
                        let pixels = crop_and_scale(image.width, &image.pixels, &placement);
                        page.show_pixels(
                            &mut self.display,
                            placement.width,
                            placement.height,
                            &pixels,
                        );
                        screen.shown = Some(request);
                    }
                }
            }
            if screen.shown.is_some() && screen.placement != Some(placement) {
                page.move_to(placement.x, placement.y);
                screen.placement = Some(placement);
            }
            page.commit();
        }
        self.draw_manual_help();
        if let Some(screen) = &self.manual.screen {
            // the subsurfaces' changes take effect with their parent's commit
            screen.backdrop.commit();
        }
    }

    /// The help row under the page: what the buttons do now, and the page.
    fn draw_manual_help(&mut self) {
        let Some(viewer) = self.manual.viewer.as_ref() else {
            return;
        };
        let drawn = (
            self.manual_prompts(viewer.zoomed()),
            format!("{} / {}", viewer.page, viewer.pages),
        );
        let (Some(help), Some(screen)) = (self.help.as_mut(), self.manual.screen.as_mut()) else {
            return;
        };
        if screen.prompts.as_ref() == Some(&drawn) {
            return;
        }
        let Some(sub) = screen.help.as_mut() else {
            return;
        };
        let width = screen.backdrop.size.0 as f32 - help.left() * 2.0;
        match help.render(&mut self.content.fonts, &drawn.0, width, Some(&drawn.1)) {
            Some(pixmap) => {
                let (x, y) = help.place(pixmap.width());
                sub.move_to(x, y);
                sub.show_pixmap(&mut self.display, &pixmap);
            }
            None => sub.hide(),
        }
        sub.commit();
        screen.prompts = Some(drawn);
    }

    /// The prompts of the manual, as ES's viewer has them: the whole page
    /// is like its page list, a zoomed page like its zoomed image.
    fn manual_prompts(&self, zoomed: bool) -> Vec<Prompt> {
        let Some(help) = &self.help else {
            return Vec::new();
        };
        let texts = &self.content.translator;
        let prompt = |icon: &str, key: &str| Prompt {
            icon: icon.to_owned(),
            label: texts.text(key),
        };
        if zoomed {
            vec![
                prompt(help.back(), "help-back"),
                prompt("l", "help-zoom-out"),
                prompt("r", "help-zoom-in"),
                prompt("up/down/left/right", "help-move"),
            ]
        } else {
            vec![
                prompt("left/right", "help-page"),
                prompt(help.ok(), "help-zoom"),
                prompt(help.back(), "help-close"),
            ]
        }
    }

    /// The renderer has finished a page.
    pub(super) fn page_rendered(&mut self, rendered: Rendered) {
        let page = rendered.0.page;
        if let Some(manual) = self.content.manual.as_mut() {
            manual.store(rendered);
        }
        let current = self
            .manual
            .viewer
            .as_ref()
            .is_some_and(|viewer| viewer.page == page);
        if self.manual_is_open() && current {
            self.show_page(false);
        }
    }

    /// A press or release while the manual is open.
    pub(super) fn manual_input(&mut self, input: Input) {
        let Some(points) = self.page_points() else {
            return;
        };
        let Some(viewer) = self.manual.viewer.as_mut() else {
            return;
        };
        let Input { nav, pressed } = input;
        let holdable = match nav {
            Nav::ZoomIn | Nav::ZoomOut => true,
            Nav::Up | Nav::Down | Nav::Left | Nav::Right => viewer.zoomed(),
            _ => false,
        };
        if !pressed {
            self.manual.held.retain(|held| *held != nav);
            return;
        }
        if holdable {
            if !self.manual.held.contains(&nav) {
                self.manual.held.push(nav);
            }
            self.start_motion();
            return;
        }
        match nav {
            Nav::Left | Nav::Up | Nav::PrevPage => {
                if viewer.turn(-1) {
                    self.show_page(true);
                }
            }
            Nav::Right | Nav::Down | Nav::NextPage => {
                if viewer.turn(1) {
                    self.show_page(true);
                }
            }
            Nav::Confirm => {
                if viewer.zoomed() {
                    viewer.fit();
                } else {
                    viewer.fit_width(points);
                }
                self.show_page(true);
            }
            Nav::Back if viewer.zoomed() => {
                viewer.fit();
                self.show_page(true);
            }
            Nav::Back => self.close_manual(),
            Nav::Close => self.outcome = Some(Outcome::Continue),
            Nav::ZoomIn | Nav::ZoomOut => {}
        }
    }

    /// Move and zoom every frame while a direction or a shoulder is held.
    fn start_motion(&mut self) {
        if self.manual.moving.is_some() {
            return;
        }
        self.manual.moving = Some(Instant::now());
        self.handle
            .insert_source(Timer::from_duration(FRAME), |_, &mut (), state| {
                state.motion_frame()
            })
            .ok();
    }

    fn motion_frame(&mut self) -> TimeoutAction {
        let now = Instant::now();
        let seconds = self
            .manual
            .moving
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32())
            .min(0.1);
        let held = &self.manual.held;
        let axis = |minus: Nav, plus: Nav| {
            f32::from(u8::from(held.contains(&plus))) - f32::from(u8::from(held.contains(&minus)))
        };
        let direction = (axis(Nav::Left, Nav::Right), axis(Nav::Up, Nav::Down));
        let zoom = axis(Nav::ZoomOut, Nav::ZoomIn);
        let (Some(points), true) = (self.page_points(), self.manual_is_open()) else {
            self.manual.moving = None;
            return TimeoutAction::Drop;
        };
        if self.manual.held.is_empty() {
            self.manual.moving = None;
            // the zoom has settled: render the page at its new size
            self.show_page(true);
            return TimeoutAction::Drop;
        }
        let Some(viewer) = self.manual.viewer.as_mut() else {
            return TimeoutAction::Drop;
        };
        let zoomed = viewer.zoomed();
        let mut changed = false;
        if zoom != 0.0 {
            changed |= viewer.zoom_for(zoom, seconds, points);
        }
        if direction != (0.0, 0.0) && viewer.zoomed() {
            changed |= viewer.pan_for(direction, seconds, points);
        }
        if zoomed && !viewer.zoomed() {
            // zoomed out to the whole page: the directions turn pages again
            self.manual
                .held
                .retain(|nav| matches!(nav, Nav::ZoomIn | Nav::ZoomOut));
        }
        self.manual.moving = Some(now);
        if changed {
            self.show_page(false);
        }
        TimeoutAction::ToDuration(FRAME)
    }

    /// Render the page sharp once a wheel zoom has stopped for a moment.
    fn settle_later(&mut self) {
        let pending = self.manual.settle_at.is_some();
        self.manual.settle_at = Some(Instant::now() + SETTLE);
        if pending {
            return;
        }
        self.handle
            .insert_source(
                Timer::from_duration(SETTLE),
                |_, &mut (), state| match state.manual.settle_at {
                    Some(at) if at > Instant::now() => TimeoutAction::ToInstant(at),
                    _ => {
                        state.manual.settle_at = None;
                        state.show_page(true);
                        TimeoutAction::Drop
                    }
                },
            )
            .ok();
    }

    /// The pointer on the manual, as on ES's viewer: the wheel zooms, a
    /// zoomed page is dragged; a click on either half of a whole page turns
    /// it back or on; the right button goes back.
    pub(super) fn manual_pointer(&mut self, event: &PointerInput) {
        let Some(screen) = &self.manual.screen else {
            return;
        };
        // the event's position is on the surface under the pointer
        let offset = [&screen.page, &screen.help]
            .into_iter()
            .flatten()
            .find(|sub| sub.is(&event.surface))
            .map_or((0, 0), |sub| sub.position);
        let position = (
            event.position.0 + f64::from(offset.0),
            event.position.1 + f64::from(offset.1),
        );
        let width = f64::from(screen.backdrop.size.0);
        match event.kind {
            PointerKind::Press(BTN_LEFT) => self.manual.drag = Some((position, false)),
            PointerKind::Motion => self.drag_to(position),
            PointerKind::Release(BTN_LEFT) => {
                let dragged = self.manual.drag.take().is_some_and(|(_, dragged)| dragged);
                let zoomed = self.manual.viewer.as_ref().is_some_and(Viewer::zoomed);
                if !dragged && !zoomed {
                    let nav = if position.0 < width / 2.0 {
                        Nav::PrevPage
                    } else {
                        Nav::NextPage
                    };
                    self.manual_input(Input::press(nav));
                }
            }
            PointerKind::Press(BTN_RIGHT) => self.manual_input(Input::press(Nav::Back)),
            PointerKind::Wheel(steps) if steps != 0 => {
                let Some(points) = self.page_points() else {
                    return;
                };
                let zoomed = self
                    .manual
                    .viewer
                    .as_mut()
                    .is_some_and(|viewer| viewer.zoom_by(WHEEL_ZOOM.powi(-steps), points));
                if zoomed {
                    self.show_page(false);
                    self.settle_later();
                }
            }
            _ => {}
        }
    }

    /// Paint a window of the manual again (X11).
    pub(super) fn manual_expose(&self, window: xproto::Window) {
        if let Some(screen) = &self.manual.screen
            && !screen.backdrop.expose(window)
        {
            for sub in [&screen.page, &screen.help].into_iter().flatten() {
                if sub.expose(window) {
                    break;
                }
            }
        }
    }

    fn drag_to(&mut self, position: (f64, f64)) {
        let Some((from, dragged)) = self.manual.drag else {
            return;
        };
        let (dx, dy) = (position.0 - from.0, position.1 - from.1);
        if !dragged && dx.hypot(dy) < DRAG_THRESHOLD {
            return;
        }
        let Some(points) = self.page_points() else {
            return;
        };
        self.manual.drag = Some((position, true));
        let moved = self
            .manual
            .viewer
            .as_mut()
            .is_some_and(|viewer| viewer.zoomed() && viewer.pan(dx as f32, dy as f32, points));
        if moved {
            self.show_page(false);
        }
    }
}

/// The part of the screen the page is shown in: above the help row when it
/// is at the bottom.
fn manual_view_size(screen: (u32, u32), help: Option<&Help>) -> (u32, u32) {
    let Some(help) = help.filter(|help| help.visible) else {
        return screen;
    };
    let top = help.place(0).1;
    let gap = help.height() / 3;
    if top > 0 && top.unsigned_abs() > screen.1 / 2 {
        (screen.0, top.unsigned_abs().saturating_sub(gap))
    } else {
        screen
    }
}

/// The visible part of a page at its size on screen, nearest pixel (only
/// without `wp_viewporter`).
fn crop_and_scale(image_width: u32, pixels: &[u8], placement: &Placement) -> Vec<u8> {
    let image_width = image_width as usize;
    let image_height = pixels.len() / 4 / image_width.max(1);
    let [left, top, right, bottom] = placement.source;
    let (width, height) = (placement.width as usize, placement.height as usize);
    let mut out = vec![0; width * height * 4];
    let column = |x: usize| {
        let u = left + (right - left) * (x as f32 + 0.5) / width as f32;
        ((u * image_width as f32) as usize).min(image_width.saturating_sub(1))
    };
    let columns: Vec<usize> = (0..width).map(column).collect();
    for (y, row) in out.chunks_exact_mut(width * 4).enumerate() {
        let v = top + (bottom - top) * (y as f32 + 0.5) / height as f32;
        let source_row = ((v * image_height as f32) as usize).min(image_height.saturating_sub(1));
        let source = &pixels[source_row * image_width * 4..];
        for (target, &x) in row.as_chunks_mut::<4>().0.iter_mut().zip(&columns) {
            target.copy_from_slice(&source[x * 4..x * 4 + 4]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_and_scales() {
        // 2×2: red, green / blue, white
        let pixels = [
            0, 0, 255, 255, 0, 255, 0, 255, //
            255, 0, 0, 255, 255, 255, 255, 255,
        ];
        let placement = Placement {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            source: [0.5, 0.5, 1.0, 1.0],
        };
        // the bottom right pixel, twice
        assert_eq!(
            crop_and_scale(2, &pixels, &placement),
            [255, 255, 255, 255, 255, 255, 255, 255]
        );
    }
}
