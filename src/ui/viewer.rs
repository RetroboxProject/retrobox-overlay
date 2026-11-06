//! The manual viewer's geometry: which page, how big and which part of it
//! is on screen. Zooming and panning work like EmulationStation's image
//! viewer: the page starts whole, the shoulders zoom while held, the d-pad
//! moves a zoomed page, and Confirm zooms to the screen's width.
//!
//! The page is rendered once per size; the compositor scales and crops it
//! (see app.rs), so a zoom or a move costs no drawing.

/// How far the page can be zoomed, relative to the whole page.
pub const MAX_ZOOM: f32 = 5.0;
/// The share of the view's width a page zoomed to the width takes.
const WIDTH_FIT: f32 = 0.96;
/// The share of the view a whole page may take, around it.
const PAGE_FIT: f32 = 0.98;
/// How fast a held direction moves a zoomed page: view heights per second.
const PAN_SPEED: f32 = 1.2;
/// How fast a held shoulder zooms: the zoom doubles in this many seconds.
const ZOOM_DOUBLING: f32 = 0.6;
/// Rendered pages are at most this big (the compositor scales them past it).
const MAX_SIDE: f32 = 8192.0;
const MAX_PIXELS: f32 = 24_000_000.0;

/// What the screen shows of the page: where, and which part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The visible part on the screen, in pixels.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// The visible part of the page: left, top, right, bottom, from 0 to 1.
    pub source: [f32; 4],
}

/// Which page is shown, and how.
#[derive(Debug, Clone)]
pub struct Viewer {
    pub page: u32,
    pub pages: u32,
    /// 1: the whole page in the view.
    zoom: f32,
    /// The point of the page at the view's centre, from 0 to 1.
    center: (f32, f32),
    /// The part of the screen the page is shown in (above the help row).
    view: (f32, f32),
}

impl Viewer {
    pub fn new(pages: u32, view: (u32, u32)) -> Self {
        Self {
            page: 1,
            pages,
            zoom: 1.0,
            center: (0.5, 0.5),
            view: (view.0 as f32, view.1 as f32),
        }
    }

    pub fn zoomed(&self) -> bool {
        self.zoom > 1.001
    }

    /// Turn `delta` pages; true when the page changed. A zoomed page stays
    /// zoomed, from its top.
    pub fn turn(&mut self, delta: i32) -> bool {
        let page = self
            .page
            .saturating_add_signed(delta)
            .clamp(1, self.pages.max(1));
        let changed = page != self.page;
        self.page = page;
        if changed {
            self.center.1 = 0.0;
        }
        changed
    }

    /// The whole page.
    pub fn fit(&mut self) {
        self.zoom = 1.0;
        self.center = (0.5, 0.5);
    }

    /// The page as wide as the view, from its top (ES's viewer opening).
    pub fn fit_width(&mut self, page: (f32, f32)) {
        let whole = self.whole_scale(page);
        let zoom = self.view.0 * WIDTH_FIT / (page.0 * whole);
        // a page already as wide as the view: zoom in all the same
        self.zoom = if zoom > 1.1 { zoom.min(MAX_ZOOM) } else { 2.0 };
        self.center = (0.5, 0.0);
        self.clamp(page);
    }

    /// Zoom by a factor (around the view's centre); true when it changed.
    pub fn zoom_by(&mut self, factor: f32, page: (f32, f32)) -> bool {
        let zoom = (self.zoom * factor).clamp(1.0, MAX_ZOOM);
        let changed = (zoom - self.zoom).abs() > f32::EPSILON;
        self.zoom = zoom;
        self.clamp(page);
        changed
    }

    /// Zoom for `seconds` of a shoulder held (`direction` 1 in, -1 out).
    pub fn zoom_for(&mut self, direction: f32, seconds: f32, page: (f32, f32)) -> bool {
        self.zoom_by(2f32.powf(direction * seconds / ZOOM_DOUBLING), page)
    }

    /// Move the page by screen pixels (the view goes the other way);
    /// true when it moved.
    pub fn pan(&mut self, dx: f32, dy: f32, page: (f32, f32)) -> bool {
        let (width, height) = self.displayed(page);
        let before = self.center;
        self.center.0 -= dx / width;
        self.center.1 -= dy / height;
        self.clamp(page);
        before != self.center
    }

    /// Move for `seconds` of a direction held (`(dx, dy)` from -1 to 1).
    pub fn pan_for(&mut self, direction: (f32, f32), seconds: f32, page: (f32, f32)) -> bool {
        let step = self.view.1 * PAN_SPEED * seconds;
        // a held direction moves the view: the page goes the other way
        self.pan(-direction.0 * step, -direction.1 * step, page)
    }

    /// The scale at which the whole page fits the view.
    fn whole_scale(&self, page: (f32, f32)) -> f32 {
        (self.view.0 * PAGE_FIT / page.0).min(self.view.1 * PAGE_FIT / page.1)
    }

    /// The page's size on screen, in pixels.
    fn displayed(&self, page: (f32, f32)) -> (f32, f32) {
        let scale = self.whole_scale(page) * self.zoom;
        (page.0 * scale, page.1 * scale)
    }

    /// Keep the view on the page.
    fn clamp(&mut self, page: (f32, f32)) {
        let (width, height) = self.displayed(page);
        let axis = |center: f32, size: f32, view: f32| {
            if size <= view {
                0.5
            } else {
                let half = view / size / 2.0;
                center.clamp(half, 1.0 - half)
            }
        };
        self.center = (
            axis(self.center.0, width, self.view.0),
            axis(self.center.1, height, self.view.1),
        );
    }

    /// What of the page is on screen and where.
    pub fn placement(&self, page: (f32, f32)) -> Placement {
        let (width, height) = self.displayed(page);
        // per axis: (screen offset, visible length, part of the page shown)
        let axis = |center: f32, size: f32, view: f32| {
            if size <= view {
                ((view - size) / 2.0, size, 0.0, 1.0)
            } else {
                let start = (center * size - view / 2.0).clamp(0.0, size - view);
                (0.0, view, start / size, (start + view) / size)
            }
        };
        let (x, visible_width, left, right) = axis(self.center.0, width, self.view.0);
        let (y, visible_height, top, bottom) = axis(self.center.1, height, self.view.1);
        Placement {
            x: x.round() as i32,
            y: y.round() as i32,
            width: (visible_width.round() as u32).max(1),
            height: (visible_height.round() as u32).max(1),
            source: [left, top, right, bottom],
        }
    }

    /// The size to render the page at: as shown, within the limits.
    pub fn render_size(&self, page: (f32, f32)) -> (u32, u32) {
        let (width, height) = self.displayed(page);
        let limit = (MAX_SIDE / width.max(height))
            .min((MAX_PIXELS / (width * height)).sqrt())
            .min(1.0);
        (
            ((width * limit).round() as u32).max(1),
            ((height * limit).round() as u32).max(1),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: (f32, f32) = (612.0, 792.0);

    #[test]
    fn turns_within_the_manual() {
        let mut viewer = Viewer::new(3, (1920, 1000));
        assert!(!viewer.turn(-1));
        assert!(viewer.turn(1));
        assert!(viewer.turn(5));
        assert_eq!(viewer.page, 3);
        assert!(!viewer.turn(1));
    }

    #[test]
    fn shows_the_whole_page_centred() {
        let viewer = Viewer::new(1, (1920, 1000));
        let placement = viewer.placement(LETTER);
        assert_eq!((placement.height, placement.y), (980, 10));
        assert_eq!(placement.width, 757);
        assert_eq!(placement.x, (1920 - 757) / 2);
        let whole = [0.0, 0.0, 1.0, 1.0];
        assert!(
            placement
                .source
                .iter()
                .zip(whole)
                .all(|(a, b)| (a - b).abs() < 1e-6)
        );
        assert_eq!(viewer.render_size(LETTER), (757, 980));
    }

    #[test]
    fn zooms_to_the_width_from_the_top() {
        let mut viewer = Viewer::new(1, (1920, 1000));
        viewer.fit_width(LETTER);
        assert!(viewer.zoomed());
        let placement = viewer.placement(LETTER);
        assert_eq!((placement.y, placement.height), (0, 1000));
        assert!(placement.width.abs_diff(1843) <= 1);
        assert!(placement.source[1].abs() < 1e-6);
        // moving down shows more of the page, never past its end
        assert!(viewer.pan_for((0.0, 1.0), 10.0, LETTER));
        let placement = viewer.placement(LETTER);
        assert!((placement.source[3] - 1.0).abs() < 1e-4);
        assert!(!viewer.pan_for((0.0, 1.0), 1.0, LETTER));
    }

    #[test]
    fn zoom_stays_within_its_limits() {
        let mut viewer = Viewer::new(1, (1920, 1000));
        assert!(!viewer.zoom_for(-1.0, 1.0, LETTER));
        assert!(viewer.zoom_for(1.0, 10.0, LETTER));
        assert!(!viewer.zoom_for(1.0, 1.0, LETTER));
        let (width, height) = viewer.render_size(LETTER);
        assert!(width as f32 * height as f32 <= MAX_PIXELS * 1.01);
        viewer.fit();
        assert!(!viewer.zoomed());
    }
}
