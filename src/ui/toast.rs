//! A toast's card (see `toast`): drawn once in the menu's colours and fonts
//! (the theme's background, title and small text), with an icon, a title and
//! a line or two.

use cosmic_text::Align;
use tiny_skia::Pixmap;

use super::panel::{
    Element, FONT_SIZE_MEDIUM, FONT_SIZE_SMALL, draw_icon, fill_path, rounded_rect,
};
use super::text::{Fonts, TextBox};
use crate::theme::MenuTheme;
use crate::toast::Toast;

/// The card's width, relative to the screen's smaller side, and the most
/// of the screen's width it takes.
const WIDTH: f32 = 0.62;
const MAX_WIDTH: f32 = 0.45;
/// The text's lines at most (the last one takes the rest).
const MAX_LINES: usize = 3;

/// A toast drawn for a screen: its picture, and where it goes.
pub struct ToastCard {
    pub pixmap: Pixmap,
    /// Its distance to the screen's top and side edges.
    pub margin: u32,
}

impl ToastCard {
    /// The card for `screen` (width, height), None when it cannot be drawn.
    pub fn new(
        theme: &MenuTheme,
        screen: (u32, u32),
        fonts: &mut Fonts,
        toast: &Toast,
    ) -> Option<Self> {
        let (width, height) = (screen.0 as f32, screen.1 as f32);
        let scale = if theme.screen.height > 0 {
            height / theme.screen.height as f32
        } else {
            1.0
        };
        let base = width.min(height);
        let mut title = Element::new(&theme.title, scale, base * FONT_SIZE_MEDIUM);
        title.font = title.font.bold();
        let text = Element::new(&theme.text_small, scale, base * FONT_SIZE_SMALL);

        let pad = (text.letter_height * 1.1).round();
        let icon = (title.font.line_height * 1.4).round();
        let card_width = (base * WIDTH).min(width * MAX_WIDTH).round();
        let text_left = pad * 2.0 + icon;
        let text_width = (card_width - text_left - pad).max(1.0);
        let lines = if toast.text.trim().is_empty() {
            Vec::new()
        } else {
            fonts.wrap(&text.font, &toast.text, text_width, MAX_LINES)
        };
        let text_height = title.font.line_height + text.font.line_height * lines.len() as f32;
        let card_height = (pad * 2.0 + text_height.max(icon)).round();

        let mut pixmap = Pixmap::new(card_width as u32, card_height as u32)?;
        let path = rounded_rect(0.0, 0.0, card_width, card_height, pad * 0.6)?;
        fill_path(&mut pixmap, &path, theme.background.center_color);
        let icon_color = title.style.color;
        draw_icon(
            &mut pixmap,
            toast.icon,
            icon,
            (pad, (card_height - icon) / 2.0),
            icon_color,
        );

        // the text block, centred against the icon
        let mut y = ((card_height - text_height) / 2.0).round();
        let row = |y: f32, height: f32| TextBox {
            x: text_left,
            y,
            width: text_width,
            height,
            align: Align::Left,
        };
        let (title_font, title_text) = fonts.fit(&title.font, &toast.title, text_width, 0.8);
        fonts.draw(
            &mut pixmap,
            &title_font,
            &title_text,
            row(y, title.font.line_height),
            title.style.color,
        );
        y += title.font.line_height;
        for line in &lines {
            fonts.draw(
                &mut pixmap,
                &text.font,
                line,
                row(y, text.font.line_height),
                text.style.color,
            );
            y += text.font.line_height;
        }
        Some(Self {
            pixmap,
            margin: pad as u32,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.pixmap.width(), self.pixmap.height())
    }
}
