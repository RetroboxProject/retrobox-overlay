//! The help prompts at the bottom of the screen, drawn as EmulationStation's
//! `HelpComponent` draws them: an icon and an upper-case label per prompt,
//! in the theme's help style (`helpsystem` of its system view), with ES's
//! icons or the theme's own.

use std::collections::HashMap;
use std::path::PathBuf;

use cosmic_text::Align;
use resvg::usvg;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

use super::text::{FontRef, Fonts, TextBox};
use crate::theme::{HelpTheme, MenuTheme, Rgba};

/// ES's `FONT_SIZE_SMALL`, relative to the screen's smaller side.
const FONT_SIZE_SMALL: f32 = 0.035;
/// ES's default position, relative to the screen.
const POSITION: (f32, f32) = (0.012, 0.9515);
/// The space between an icon and its label, relative to the screen width
/// (ES's `ICON_TEXT_SPACING`); twice that between prompts.
const ICON_TEXT_SPACING: f32 = 0.0042;

/// A prompt: the icon's name (`up/down`, `a`, `l`...) and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub icon: String,
    pub label: String,
}

/// The help style at this screen's size, with its icons drawn once.
pub struct Help {
    pub visible: bool,
    font: FontRef,
    /// ES's row height: the letter height and a quarter.
    height: f32,
    /// Where the row goes (its top left corner, after the origin).
    position: (f32, f32),
    origin: (f32, f32),
    spacing: f32,
    icon_color: Rgba,
    text_color: Rgba,
    paths: HashMap<String, PathBuf>,
    icons: HashMap<String, Option<Pixmap>>,
    ok: String,
    back: String,
}

impl Help {
    pub fn new(theme: &MenuTheme, screen: (u32, u32)) -> Self {
        let HelpTheme {
            visible,
            position,
            origin,
            icon_color,
            text_color,
            font,
            icons,
            ok,
            back,
        } = theme.help.clone();
        let (width, height) = (screen.0 as f32, screen.1 as f32);
        let (scale_x, scale_y) = if theme.screen.width > 0 && theme.screen.height > 0 {
            (
                width / theme.screen.width as f32,
                height / theme.screen.height as f32,
            )
        } else {
            (1.0, 1.0)
        };
        let size = if font.size > 0.0 {
            font.size * scale_y
        } else {
            width.min(height) * FONT_SIZE_SMALL
        };
        let letter_height = if font.letter_height > 0.0 {
            font.letter_height * scale_y
        } else {
            size * 0.7
        };
        let position = position.map_or((width * POSITION.0, height * POSITION.1), |(x, y)| {
            (x * scale_x, y * scale_y)
        });
        Self {
            visible,
            font: FontRef {
                path: font.path,
                size,
                line_height: if font.height > 0.0 {
                    font.height * scale_y
                } else {
                    size * 1.25
                },
                bold: false,
            },
            height: (letter_height * 1.25).round().max(1.0),
            position,
            origin,
            spacing: width * ICON_TEXT_SPACING,
            icon_color,
            text_color,
            paths: icons,
            icons: HashMap::new(),
            ok,
            back,
        }
    }

    /// The icon names of confirm and back.
    pub fn ok(&self) -> &str {
        &self.ok
    }

    pub fn back(&self) -> &str {
        &self.back
    }

    pub fn height(&self) -> u32 {
        self.height as u32
    }

    /// The height of a drawn row: the icons' height, or the font's line if
    /// taller (accents above capitals must not be cut).
    fn row_height(&self) -> f32 {
        self.height.max(self.font.line_height.ceil())
    }

    /// The left edge of the row, on the screen.
    pub const fn left(&self) -> f32 {
        self.position.0
    }

    /// Where a row this wide goes on the screen (its top left corner): as
    /// ES places its icons, with any extra height of the text shared above
    /// and below.
    pub fn place(&self, width: u32) -> (i32, i32) {
        let extra = (self.row_height() - self.height) / 2.0;
        (
            (self.position.0 - self.origin.0 * width as f32).round() as i32,
            (self.position.1 - self.origin.1 * self.height - extra).round() as i32,
        )
    }

    /// The prompts in a row at most `max_width` wide (ES leaves out those
    /// that do not fit), with `trailing` (the page number) at its right end
    /// when given; None when there is nothing to show.
    pub fn render(
        &mut self,
        fonts: &mut Fonts,
        prompts: &[Prompt],
        max_width: f32,
        trailing: Option<&str>,
    ) -> Option<Pixmap> {
        if !self.visible || (prompts.is_empty() && trailing.is_none()) {
            return None;
        }
        let height = self.row_height();
        let icon_top = ((height - self.height) / 2.0).round() as i32;
        let mut items = Vec::new();
        let mut used: f32 = 0.0;
        let reserved = trailing.map_or(0.0, |text| {
            fonts.measure(&self.font, text) + self.spacing * 2.0
        });
        for prompt in prompts {
            let icon = self.icon(&prompt.icon, self.height as u32);
            let icon_width = icon.as_ref().map_or(0.0, |icon| icon.width() as f32);
            let label = prompt.label.to_uppercase();
            let label_width = fonts.measure(&self.font, &label);
            let width = icon_width + self.spacing + label_width + self.spacing * 2.0;
            if used + width + reserved >= max_width {
                break;
            }
            items.push((icon, label, label_width));
            used += width;
        }
        let total = if trailing.is_some() { max_width } else { used };
        let mut pixmap = Pixmap::new(total.ceil().max(1.0) as u32, height as u32)?;
        let mut x: f32 = 0.0;
        for (icon, label, label_width) in items {
            if let Some(icon) = icon {
                pixmap.draw_pixmap(
                    x.round() as i32,
                    icon_top,
                    icon.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
                x += icon.width() as f32;
            }
            x += self.spacing;
            let area = TextBox {
                x,
                y: 0.0,
                width: label_width + 1.0,
                height,
                align: Align::Left,
            };
            fonts.draw(&mut pixmap, &self.font, &label, area, self.text_color);
            x += label_width + self.spacing * 2.0;
        }
        if let Some(text) = trailing {
            let area = TextBox {
                x: 0.0,
                y: 0.0,
                width: total,
                height,
                align: Align::Right,
            };
            fonts.draw(&mut pixmap, &self.font, text, area, self.text_color);
        }
        Some(pixmap)
    }

    /// An icon at the row's height, tinted like ES's colour shift (the
    /// icon's colours multiplied by the icon colour).
    fn icon(&mut self, name: &str, height: u32) -> Option<Pixmap> {
        if let Some(icon) = self.icons.get(name) {
            return icon.clone();
        }
        let icon = self
            .paths
            .get(name)
            .and_then(|path| draw_svg(path, height, self.icon_color));
        if icon.is_none() {
            log::debug!("no help icon {name:?}");
        }
        self.icons.insert(name.to_owned(), icon.clone());
        icon
    }
}

fn draw_svg(path: &std::path::Path, height: u32, tint: Rgba) -> Option<Pixmap> {
    let data = std::fs::read(path).ok()?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .inspect_err(|err| log::warn!("help icon {}: {err}", path.display()))
        .ok()?;
    let size = tree.size();
    let scale = height as f32 / size.height();
    let width = (size.width() * scale).round().max(1.0) as u32;
    let mut pixmap = Pixmap::new(width, height.max(1))?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let [red, green, blue, alpha] = tint.0.to_be_bytes();
    let multiply = |channel: u8, by: u8| ((u16::from(channel) * u16::from(by) + 127) / 255) as u8;
    for pixel in pixmap.pixels_mut() {
        // premultiplied: every channel is scaled, the colours stay under
        // the alpha
        if let Some(tinted) = tiny_skia::PremultipliedColorU8::from_rgba(
            multiply(multiply(pixel.red(), red), alpha),
            multiply(multiply(pixel.green(), green), alpha),
            multiply(multiply(pixel.blue(), blue), alpha),
            multiply(pixel.alpha(), alpha),
        ) {
            *pixel = tinted;
        }
    }
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help() -> Help {
        Help::new(&MenuTheme::default(), (1920, 1080))
    }

    #[test]
    fn follows_es_defaults() {
        let help = help();
        // FONT_SIZE_SMALL of 1080 is 37.8, its letter height 26.5
        assert_eq!(help.height(), 33);
        // the row is as tall as the font's line, centred on the icons' row
        assert_eq!(help.place(400), (23, 1020));
        assert_eq!((help.ok(), help.back()), ("b", "a"));
    }

    #[test]
    fn tints_icons() {
        let dir = std::env::temp_dir().join("retrobox-overlay-help-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("square.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" fill="#fff"/></svg>"##,
        )
        .unwrap();
        let icon = draw_svg(&path, 20, Rgba(0x8000_00FF)).unwrap();
        assert_eq!((icon.width(), icon.height()), (40, 20));
        let pixel = icon.pixel(10, 10).unwrap();
        assert_eq!((pixel.red(), pixel.green(), pixel.alpha()), (0x80, 0, 0xFF));
    }
}
