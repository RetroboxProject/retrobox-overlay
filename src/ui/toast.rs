//! A toast's card (see `toast`): drawn once in the menu's colours and fonts
//! (the theme's background, title and small text), with an icon, a title and
//! a line or two; the volume keys' one has the volume's icon and a bar like
//! the menu's volume slider, with the volume in percent after it.

use cosmic_text::Align;
use tiny_skia::Pixmap;

use super::panel::{
    DISABLED_ALPHA, Element, FONT_SIZE_MEDIUM, FONT_SIZE_SMALL, TRACK_ALPHA, draw_icon, fill,
    fill_circle, fill_path, rounded_rect,
};
use super::text::{Fonts, TextBox};
use crate::theme::{MenuTheme, Rgba};
use crate::toast::Toast;
use crate::volume::Level;

/// The card's width, relative to the screen's smaller side, and the most
/// of the screen's width it takes.
const WIDTH: f32 = 0.62;
const MAX_WIDTH: f32 = 0.45;
/// The text's lines at most (the last one takes the rest).
const MAX_LINES: usize = 3;
/// The volume's card is narrower, relative to the screen's smaller side.
const VOLUME_WIDTH: f32 = 0.4;
/// Its bar's thickness and knob, relative to the icon (the menu slider's
/// proportions).
const TRACK_THICKNESS: f32 = 0.14;
const KNOB_RADIUS: f32 = 0.27;

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
        if let Some(level) = toast.level {
            let card_width = (base * VOLUME_WIDTH).min(width * MAX_WIDTH).round();
            let card_height = (pad * 2.0 + icon).round();
            let mut pixmap = card(theme, card_width, card_height, pad)?;
            let color = title.style.color;
            draw_icon(&mut pixmap, toast.icon, icon, (pad, pad), color);
            let bar = Bar {
                left: pad * 2.0 + icon,
                right: card_width - pad,
                top: pad,
                height: icon,
                icon,
            };
            bar.draw(&mut pixmap, fonts, &text, level, color);
            return Some(Self {
                pixmap,
                margin: pad as u32,
            });
        }
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

        let mut pixmap = card(theme, card_width, card_height, pad)?;
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

/// The card's background: the menu's, with rounded corners.
fn card(theme: &MenuTheme, width: f32, height: f32, pad: f32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width as u32, height as u32)?;
    let path = rounded_rect(0.0, 0.0, width, height, pad * 0.6)?;
    fill_path(&mut pixmap, &path, theme.background.center_color);
    Some(pixmap)
}

/// Where the volume's bar goes, the percentage included.
struct Bar {
    left: f32,
    right: f32,
    top: f32,
    height: f32,
    icon: f32,
}

impl Bar {
    /// A line filled up to the volume, with a knob, and the volume in percent
    /// after it; muted, faded (its value kept), as the menu's slider.
    fn draw(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        text: &Element,
        level: Level,
        color: Rgba,
    ) {
        let color = if level.muted {
            color.with_alpha(DISABLED_ALPHA.min((color.0 & 0xff) as u8))
        } else {
            color
        };
        let value_width = fonts.measure(&text.font, "100%").ceil() + self.icon * 0.3;
        let (start, end) = (self.left + self.icon * 0.2, self.right - value_width);
        let middle = self.top + self.height / 2.0;
        let thickness = (self.icon * TRACK_THICKNESS).max(3.0).round();
        let knob = (self.icon * KNOB_RADIUS).max(6.0);
        let filled = start + (end - start) * level.value();
        fill(
            pixmap,
            filled,
            middle - thickness / 2.0,
            end - filled,
            thickness,
            color.with_alpha(TRACK_ALPHA.min((color.0 & 0xff) as u8)),
        );
        fill(
            pixmap,
            start,
            middle - thickness / 2.0,
            filled - start,
            thickness,
            color,
        );
        fill_circle(pixmap, (filled, middle), knob, color);
        fonts.draw(
            pixmap,
            &text.font,
            &format!("{}%", level.percent),
            TextBox {
                x: end,
                y: self.top,
                width: self.right - end,
                height: self.height,
                align: Align::Right,
            },
            color,
        );
    }
}
