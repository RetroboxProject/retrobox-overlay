//! The clock shown on the free side of the screen: time with seconds, big,
//! and the date under it, in the menu title's font, aligned to the screen's
//! edge on that side.

use chrono::{DateTime, Local};
use cosmic_text::Align;
use tiny_skia::Pixmap;

use super::text::{FontRef, Fonts, TextBox};
use crate::theme::Rgba;

const TIME_SIZE: f32 = 0.095;
const DATE_SIZE: f32 = 0.035;
/// The space between the text and the screen's edge, relative to the screen
/// height (the same as above it in a corner).
pub const MARGIN: f32 = 0.045;
const SHADOW: Rgba = Rgba(0x0000_0099);

pub struct Clock {
    time_format: String,
    date_format: String,
    locale: chrono::Locale,
    color: Rgba,
    time_font: FontRef,
    date_font: FontRef,
    /// Left or right: towards the screen's edge.
    align: Align,
    inset: f32,
}

impl Clock {
    pub fn new(
        font_path: &std::path::Path,
        screen_height: u32,
        formats: (String, String),
        locale: chrono::Locale,
        color: Rgba,
        align: Align,
    ) -> Self {
        let height = screen_height as f32;
        let font = |size: f32| FontRef {
            path: font_path.to_path_buf(),
            size,
            line_height: size * 1.25,
        };
        Self {
            time_format: formats.0,
            date_format: formats.1,
            locale,
            color,
            time_font: font(height * TIME_SIZE),
            date_font: font(height * DATE_SIZE),
            align,
            inset: height * MARGIN,
        }
    }

    /// The height the clock needs.
    pub fn height(&self) -> u32 {
        (self.time_font.line_height + self.date_font.line_height * 1.5).ceil() as u32
    }

    /// The two lines for a moment (strftime, names in the clock's language).
    pub fn lines(&self, now: &DateTime<Local>) -> (String, String) {
        let format = |pattern: &str| now.format_localized(pattern, self.locale).to_string();
        (
            format(&self.time_format),
            capitalize(&format(&self.date_format)),
        )
    }

    pub fn render(&self, fonts: &mut Fonts, width: u32, now: &DateTime<Local>) -> Option<Pixmap> {
        let mut pixmap = Pixmap::new(width.max(1), self.height())?;
        let (time, date) = self.lines(now);
        let width = width as f32;
        let offset = (self.time_font.size / 40.0).max(1.0);
        let time_box = TextBox {
            x: self.inset,
            y: 0.0,
            width: (width - self.inset * 2.0).max(1.0),
            height: self.time_font.line_height,
            align: self.align,
        };
        let date_box = TextBox {
            y: self.time_font.line_height,
            height: self.date_font.line_height * 1.5,
            ..time_box
        };
        for (font, text, area) in [
            (&self.time_font, &time, time_box),
            (&self.date_font, &date, date_box),
        ] {
            let shadow = TextBox {
                x: area.x + offset,
                y: area.y + offset,
                ..area
            };
            fonts.draw(&mut pixmap, font, text, shadow, SHADOW);
            fonts.draw(&mut pixmap, font, text, area, self.color);
        }
        Some(pixmap)
    }
}

/// Day names are lower case in most locales: "miércoles, 4…" -> "Miércoles, 4…".
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn formats_in_the_language() {
        let clock = Clock::new(
            std::path::Path::new(""),
            1080,
            ("%H:%M:%S".into(), "%A, %-d de %B de %Y".into()),
            chrono::Locale::es_ES,
            Rgba(0xFFFF_FFFF),
            Align::Left,
        );
        let moment = Local.with_ymd_and_hms(2026, 11, 4, 9, 5, 7).unwrap();
        assert_eq!(
            clock.lines(&moment),
            (
                "09:05:07".to_owned(),
                "Miércoles, 4 de noviembre de 2026".to_owned()
            )
        );
    }
}
