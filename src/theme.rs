//! The menu style EmulationStation draws with, as it exports it on every game
//! launch (`es-menu-theme.json`, retrobox-emulationstation patch 0006).
//!
//! Every value has EmulationStation's own default (`ThemeData.h`), so the
//! overlay still looks like ES's default menus when the file is missing.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

/// A colour as ES writes it: `RRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub u32);

impl Rgba {
    /// The tiny-skia colour.
    pub fn to_color(self) -> tiny_skia::Color {
        let [r, g, b, a] = self.0.to_be_bytes();
        tiny_skia::Color::from_rgba8(r, g, b, a)
    }

    /// The cosmic-text colour.
    pub fn to_text_color(self) -> cosmic_text::Color {
        let [r, g, b, a] = self.0.to_be_bytes();
        cosmic_text::Color::rgba(r, g, b, a)
    }

    /// Parse `RRGGBB` or `RRGGBBAA` (with or without `#`).
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().trim_start_matches('#');
        let value = u32::from_str_radix(hex, 16).ok()?;
        match hex.len() {
            6 => Some(Self((value << 8) | 0xFF)),
            8 => Some(Self(value)),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("bad colour {text:?}")))
    }
}

/// A font as ES uses it: file and sizes in pixels on ES's screen.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FontSpec {
    pub path: PathBuf,
    pub size: f32,
    pub height: f32,
    pub letter_height: f32,
}

/// A text element of the menu (title, rows, small text, footer).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextStyle {
    pub font: FontSpec,
    pub color: Rgba,
    pub selected_color: Rgba,
    pub selector_color: Rgba,
    pub selector_color_end: Rgba,
    pub selector_gradient_type: String,
    pub separator_color: Rgba,
}

impl TextStyle {
    const fn with_colors(color: u32, selected: u32, selector: u32, separator: u32) -> Self {
        Self {
            font: FontSpec {
                path: PathBuf::new(),
                size: 0.0,
                height: 0.0,
                letter_height: 0.0,
            },
            color: Rgba(color),
            selected_color: Rgba(selected),
            selector_color: Rgba(selector),
            selector_color_end: Rgba(selector),
            selector_gradient_type: String::new(),
            separator_color: Rgba(separator),
        }
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        // ES's "Text" defaults
        Self::with_colors(0x7777_77FF, 0xFFFF_FFFF, 0x8787_87FF, 0xC6C7_C6FF)
    }
}

/// The menu background (a tinted 9-patch in ES).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Background {
    pub color: Rgba,
    pub center_color: Rgba,
    pub path: PathBuf,
    pub corner_size: (f32, f32),
}

impl Default for Background {
    fn default() -> Self {
        Self {
            color: Rgba(0xFFFF_FFFF),
            center_color: Rgba(0xFFFF_FFFF),
            path: PathBuf::new(),
            corner_size: (16.0, 16.0),
        }
    }
}

/// ES's screen when it exported the theme.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
        }
    }
}

/// The whole exported menu theme.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuTheme {
    pub screen: Screen,
    pub language: String,
    pub invert_buttons: bool,
    pub clock12h: bool,
    pub background: Background,
    pub title: TextStyle,
    pub text: TextStyle,
    pub text_small: TextStyle,
    pub footer: TextStyle,
}

impl Default for MenuTheme {
    fn default() -> Self {
        Self {
            screen: Screen::default(),
            language: String::new(),
            invert_buttons: false,
            clock12h: false,
            background: Background::default(),
            title: TextStyle::with_colors(0x5555_55FF, 0x5555_55FF, 0x5555_55FF, 0xFFFF_FFFF),
            text: TextStyle::default(),
            text_small: TextStyle::default(),
            footer: TextStyle::with_colors(0xC6C6_C6FF, 0xC6C6_C6FF, 0xC6C6_C6FF, 0xFFFF_FFFF),
        }
    }
}

impl MenuTheme {
    /// The exported theme, or ES's defaults when it is missing or unreadable.
    pub fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                log::warn!("cannot read the menu theme {}: {err}", path.display());
                Self::default()
            }),
            Err(err) => {
                log::info!("no menu theme at {} ({err}): ES defaults", path.display());
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colours() {
        assert_eq!(Rgba::parse("112233"), Some(Rgba(0x1122_33FF)));
        assert_eq!(Rgba::parse("#11223344"), Some(Rgba(0x1122_3344)));
        assert_eq!(Rgba::parse("xyz"), None);
    }

    #[test]
    fn partial_export_keeps_defaults() {
        let theme: MenuTheme = serde_json::from_str(
            r#"{"version": 1, "text": {"color": "FF0000FF", "font": {"size": 30, "height": 45}},
                "screen": {"width": 1280, "height": 720}}"#,
        )
        .unwrap();
        assert_eq!(theme.text.color, Rgba(0xFF00_00FF));
        assert_eq!(theme.text.selected_color, Rgba(0xFFFF_FFFF));
        assert_eq!(theme.title.color, Rgba(0x5555_55FF));
        assert_eq!(theme.screen.height, 720);
    }
}
