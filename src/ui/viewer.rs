//! The manual viewer: one page over the whole screen, with its number.
//!
//! The page is fitted whole, or zoomed to the screen's width and scrolled.

use cosmic_text::Align;
use tiny_skia::{Color, FilterQuality, Paint, Pixmap, PixmapPaint, Rect, Transform};

use super::text::{FontRef, Fonts, TextBox};
use crate::manual::Fit;
use crate::theme::Rgba;

/// The backdrop behind the page, over the dim.
const BACKDROP: Rgba = Rgba(0x0000_00F2);
const TEXT: Rgba = Rgba(0xFFFF_FFFF);
const SHADOW: Rgba = Rgba(0x0000_00CC);
/// The page number's size, relative to the screen height.
const NUMBER_SIZE: f32 = 0.03;
/// How far Up and Down scroll a zoomed page, relative to the screen height.
const SCROLL_STEP: f32 = 0.15;

/// Which page is shown, and how.
pub struct Viewer {
    pub page: u32,
    pub pages: u32,
    pub fit: Fit,
    /// How far a zoomed page is scrolled down, in pixels.
    scroll: f32,
    screen: (u32, u32),
    font: FontRef,
}

impl Viewer {
    pub fn new(pages: u32, screen: (u32, u32), font_path: &std::path::Path) -> Self {
        let size = screen.1 as f32 * NUMBER_SIZE;
        Self {
            page: 1,
            pages,
            fit: Fit::Page,
            scroll: 0.0,
            screen,
            font: FontRef {
                path: font_path.to_path_buf(),
                size,
                line_height: size * 1.4,
            },
        }
    }

    pub const fn screen(&self) -> (u32, u32) {
        self.screen
    }

    /// Turn `delta` pages; true when the page changed.
    pub fn turn(&mut self, delta: i32) -> bool {
        let page = self
            .page
            .saturating_add_signed(delta)
            .clamp(1, self.pages.max(1));
        let changed = page != self.page;
        self.page = page;
        self.scroll = 0.0;
        changed
    }

    /// Show the whole page or zoom in.
    pub fn toggle_fit(&mut self) {
        self.fit = self.fit.toggled();
        self.scroll = 0.0;
    }

    /// Scroll a zoomed page by `steps` (down when positive) for a page this
    /// tall; false when it did not move (not zoomed, or at an end).
    pub fn scroll(&mut self, steps: f32, page_height: Option<u32>) -> bool {
        if self.fit != Fit::Width {
            return false;
        }
        let limit = page_height.map_or(0.0, |height| self.max_scroll(height));
        let scroll = (self.scroll + steps * self.screen.1 as f32 * SCROLL_STEP).clamp(0.0, limit);
        let moved = (scroll - self.scroll).abs() > 0.5;
        self.scroll = scroll;
        moved
    }

    fn max_scroll(&self, page_height: u32) -> f32 {
        (page_height as f32 + 2.0 * self.margin() - self.screen.1 as f32).max(0.0)
    }

    /// The space above and below the page (the number goes in the lower one).
    fn margin(&self) -> f32 {
        self.font.line_height * 1.2
    }

    /// The whole screen: the page (or nothing while it renders) and its
    /// number.
    pub fn render(&self, fonts: &mut Fonts, page: Option<&Pixmap>) -> Option<Pixmap> {
        let (width, height) = self.screen;
        let mut pixmap = Pixmap::new(width.max(1), height.max(1))?;
        pixmap.fill(BACKDROP.to_color());
        let (width, height) = (width as f32, height as f32);
        let margin = self.margin();

        if let Some(page) = page {
            let x = ((width - page.width() as f32) / 2.0).round();
            let y = match self.fit {
                Fit::Page => ((height - page.height() as f32) / 2.0).max(0.0),
                Fit::Width => {
                    let limit = self.max_scroll(page.height());
                    margin - self.scroll.min(limit)
                }
            }
            .round();
            pixmap.draw_pixmap(
                x as i32,
                y as i32,
                page.as_ref(),
                &PixmapPaint {
                    quality: FilterQuality::Nearest,
                    ..PixmapPaint::default()
                },
                Transform::identity(),
                None,
            );
        }

        let number = format!("{} / {}", self.page, self.pages);
        let label_width = fonts.measure(&self.font, &number) + self.font.size * 1.6;
        let area = TextBox {
            x: (width - label_width) / 2.0,
            y: height - margin + (margin - self.font.line_height) / 2.0,
            width: label_width,
            height: self.font.line_height,
            align: Align::Center,
        };
        if let Some(pill) = Rect::from_xywh(area.x, area.y, area.width, area.height) {
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(0, 0, 0, 0xB0));
            pixmap.fill_rect(pill, &paint, Transform::identity(), None);
        }
        let offset = (self.font.size / 20.0).max(1.0);
        let shadow = TextBox {
            x: area.x + offset,
            y: area.y + offset,
            ..area
        };
        fonts.draw(&mut pixmap, &self.font, &number, shadow, SHADOW);
        fonts.draw(&mut pixmap, &self.font, &number, area, TEXT);
        Some(pixmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_within_the_manual() {
        let mut viewer = Viewer::new(3, (1920, 1080), std::path::Path::new(""));
        assert!(!viewer.turn(-1));
        assert!(viewer.turn(1));
        assert!(viewer.turn(5));
        assert_eq!(viewer.page, 3);
        assert!(!viewer.turn(1));
    }

    #[test]
    fn scrolls_only_when_zoomed() {
        let mut viewer = Viewer::new(1, (1920, 1080), std::path::Path::new(""));
        assert!(!viewer.scroll(1.0, Some(3000)));
        viewer.toggle_fit();
        assert!(viewer.scroll(1.0, Some(3000)));
        // back up, and no further than the top
        assert!(viewer.scroll(-5.0, Some(3000)));
        assert!(!viewer.scroll(-1.0, Some(3000)));
        // a page shorter than the screen does not scroll
        assert!(!viewer.scroll(1.0, Some(800)));
    }
}
