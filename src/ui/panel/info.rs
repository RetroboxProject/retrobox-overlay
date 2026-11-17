//! The game's page: its scraped picture, what is known about it (developer,
//! release, play time...) and its description. It is laid out and drawn
//! once, when it opens; Up and Down scroll it.

use std::path::PathBuf;

use cosmic_text::Align;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

use super::Panel;
use crate::ui::picture;
use crate::ui::text::{Fonts, TextBox};

/// What the page shows.
#[derive(Debug, Clone, Default)]
pub struct InfoSheet {
    pub title: String,
    pub image: Option<PathBuf>,
    /// Name and value, in order.
    pub facts: Vec<(String, String)>,
    pub description: String,
}

/// The page, drawn.
pub(super) struct InfoPage {
    pub title: String,
    content: Pixmap,
}

impl InfoPage {
    pub fn height(&self) -> f32 {
        self.content.height() as f32
    }
}

/// The picture's height at most, relative to the screen's.
const IMAGE_HEIGHT: f32 = 0.3;

impl Panel {
    /// Show the game's page instead of the main entries.
    pub fn open_info(&mut self, fonts: &mut Fonts, sheet: &InfoSheet) {
        let Some(content) = self.lay_out_info(fonts, sheet) else {
            return;
        };
        let page = InfoPage {
            title: sheet.title.clone(),
            content,
        };
        self.show_page(super::Page::Info(page), 0);
    }

    /// The page's content, as wide as the panel inside its padding.
    fn lay_out_info(&self, fonts: &mut Fonts, sheet: &InfoSheet) -> Option<Pixmap> {
        let padding = self.padding();
        let width = self.width as f32 - 2.0 * padding;
        let gap = (padding / 2.0).round();
        let font = &self.small.font;
        let row = font.line_height;
        let image = sheet
            .image
            .as_deref()
            .and_then(|path| picture::load(path, width, self.screen_height * IMAGE_HEIGHT));
        let lines: Vec<String> = sheet
            .description
            .lines()
            .filter(|paragraph| !paragraph.trim().is_empty())
            .flat_map(|paragraph| fonts.wrap(font, paragraph, width, usize::MAX))
            .collect();

        let image_height = image
            .as_ref()
            .map_or(0.0, |image| image.height() as f32 + gap);
        let facts_height = sheet.facts.len() as f32 * row;
        let text_height = lines.len() as f32 * row;
        let height = gap + image_height + facts_height + gap + text_height + gap;
        let mut pixmap = Pixmap::new(width.ceil() as u32, height.ceil().max(1.0) as u32)?;

        let mut y = gap;
        if let Some(image) = image {
            let x = ((width - image.width() as f32) / 2.0).round() as i32;
            pixmap.draw_pixmap(
                x,
                y as i32,
                image.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            y += image_height;
        }
        let dim = self.small.style.color;
        let color = self.text.style.color;
        for (name, value) in &sheet.facts {
            let line = |x: f32, width: f32, align: Align| TextBox {
                x,
                y,
                width,
                height: row,
                align,
            };
            fonts.draw(&mut pixmap, font, name, line(0.0, width, Align::Left), dim);
            let name_width = fonts.measure(font, name) + gap;
            let (value_font, value) = fonts.fit(font, value, width - name_width, 0.8);
            fonts.draw(
                &mut pixmap,
                &value_font,
                &value,
                line(name_width, width - name_width, Align::Right),
                color,
            );
            y += row;
        }
        y += gap;
        for text in &lines {
            let line = TextBox {
                x: 0.0,
                y,
                width,
                height: row,
                align: Align::Left,
            };
            fonts.draw(&mut pixmap, font, text, line, color);
            y += row;
        }
        Some(pixmap)
    }

    /// The page, scrolled, from the top of the scrolling area.
    pub(super) fn draw_info(&self, pixmap: &mut Pixmap, page: &InfoPage) {
        let (top, _) = self.list_area();
        pixmap.draw_pixmap(
            self.padding().round() as i32,
            (top - self.scroll).round() as i32,
            page.content.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
}
