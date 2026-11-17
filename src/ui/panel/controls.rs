//! The controls page: what the game's pad-to-keyboard profile does, one
//! mapping per row (its inputs as EmulationStation's help icons, what it
//! does), under the keyboard and mouse with the keys the profile presses
//! lit; the selected mapping's keys are lit as the selection.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use cosmic_text::Align;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

use super::{Page, Panel, Target, fill};
use crate::controls::Mapping;
use crate::theme::ES_HELP_ICONS;
use crate::ui::help::draw_svg;
use crate::ui::keyboard::{KEYBOARD_SVG, KeyColors, Keyboard};
use crate::ui::text::{Fonts, TextBox};

/// The page, ready to draw.
pub(super) struct ControlsPage {
    pub title: String,
    pub rows: Vec<Mapping>,
    /// Each input's icon, in the rows' colour and the selected row's.
    icons: HashMap<String, [Pixmap; 2]>,
    keyboard: Option<Keyboard>,
}

impl ControlsPage {
    /// The keys of a row, lit on the keyboard.
    fn keys(&self, row: usize) -> &[String] {
        self.rows.get(row).map_or(&[], |control| &control.keys)
    }
}

/// An input's icon among ES's help icons, and a letter telling the left
/// stick from the right one (their icons are the same).
fn input_icon(input: &str) -> Option<(&'static str, Option<&'static str>)> {
    let side = if input.starts_with("joystick2") || input == "r3" {
        Some("R")
    } else {
        Some("L")
    };
    let icon = match input {
        // EmulationStation's names, SNES-like: a is east, b is south
        "a" => "buttons_east.svg",
        "b" => "buttons_south.svg",
        "x" => "buttons_north.svg",
        "y" => "buttons_west.svg",
        "start" => "button_start.svg",
        "select" => "button_select.svg",
        "hotkey" => "button_hotkey.svg",
        "l1" | "pageup" => "button_l.svg",
        "r1" | "pagedown" => "button_r.svg",
        "l2" => "button_lt.svg",
        "r2" => "button_rt.svg",
        "up" => "dpad_up.svg",
        "down" => "dpad_down.svg",
        "left" => "dpad_left.svg",
        "right" => "dpad_right.svg",
        "l3" | "r3" => return Some(("analog_thumb.svg", side)),
        "joystick1" | "joystick2" => return Some(("analog_stick.svg", side)),
        _ => {
            let direction = input
                .strip_prefix("joystick1")
                .or_else(|| input.strip_prefix("joystick2"))?;
            let icon = match direction {
                "up" => "analog_up.svg",
                "down" => "analog_down.svg",
                "left" => "analog_left.svg",
                "right" => "analog_right.svg",
                _ => return None,
            };
            return Some((icon, side));
        }
    };
    Some((icon, None))
}

impl Panel {
    /// Show the controls page instead of the main entries.
    pub fn open_controls(&mut self, title: String, rows: Vec<Mapping>) {
        let size = self.row_icon().round() as u32;
        let style = &self.text.style;
        let mut icons = HashMap::new();
        for input in rows.iter().flat_map(|control| &control.inputs) {
            if icons.contains_key(input) {
                continue;
            }
            let Some((file, _)) = input_icon(input) else {
                continue;
            };
            let path = Path::new(ES_HELP_ICONS).join(file);
            if let (Some(plain), Some(selected)) = (
                draw_svg(&path, size, style.color),
                draw_svg(&path, size, style.selected_color),
            ) {
                icons.insert(input.clone(), [plain, selected]);
            }
        }
        let mapped: HashSet<&str> = rows
            .iter()
            .flat_map(|control| control.keys.iter().map(String::as_str))
            .collect();
        // shades of the text's colour over the background: whatever the
        // theme, the more a key matters, the more it stands out
        let colors = KeyColors {
            case: self.background.mix(style.color, 0.12),
            key: self.background.mix(style.color, 0.3),
            legend: style.color,
            mapped: self.background.mix(style.color, 0.6),
            chosen: style.color,
            chosen_legend: self.background,
        };
        let width = (self.width as f32 - 2.0 * self.padding()).round() as u32;
        let keyboard = Keyboard::load(Path::new(KEYBOARD_SVG), width, &mapped, &colors);
        let page = ControlsPage {
            title,
            rows,
            icons,
            keyboard,
        };
        self.show_page(Page::Controls(page), 0);
    }

    /// The height of the keyboard and its margins above the rows (0: none).
    pub(super) fn keyboard_band(&self) -> f32 {
        match &self.page {
            Some(Page::Controls(ControlsPage {
                keyboard: Some(keyboard),
                ..
            })) => keyboard.height() as f32 + self.padding(),
            _ => 0.0,
        }
    }

    /// The rows, then the keyboard over what scrolled under it.
    pub(super) fn draw_controls(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        page: &ControlsPage,
        padding: f32,
    ) {
        let line = self.line();
        let row_height = self.row_height();
        let (rows, mut y) = self.shown_rows();
        for index in rows {
            let target = Target::Item(index);
            self.draw_row_background(pixmap, target, y, row_height);
            self.draw_control(pixmap, fonts, page, index, (padding, y));
            y += row_height;
            fill(
                pixmap,
                0.0,
                y,
                self.width as f32,
                line,
                self.text.style.separator_color,
            );
            y += line;
        }
        if let Some(keyboard) = &page.keyboard {
            let top = self.header_height();
            fill(
                pixmap,
                0.0,
                top,
                self.width as f32,
                self.keyboard_band(),
                self.background,
            );
            let chosen = match self.selected {
                Target::Item(row) => page.keys(row),
                _ => &[],
            };
            keyboard.draw(pixmap, padding, top + padding / 2.0, chosen);
        }
    }

    /// A row: the inputs' icons ("+" between them, "›" in a sequence),
    /// what it does and, on the right, the keys it presses.
    fn draw_control(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        page: &ControlsPage,
        index: usize,
        at: (f32, f32),
    ) {
        let (padding, y) = at;
        let control = &page.rows[index];
        let target = Target::Item(index);
        let selected = usize::from(target == self.selected);
        let color = self.row_color(target, true);
        let row_height = self.row_height();
        let font = &self.text.font;
        let small = &self.small.font;
        let right = self.width as f32 - padding;
        let gap = (padding * 0.25).round();
        let text = |fonts: &mut Fonts, pixmap: &mut Pixmap, text: &str, x: f32| {
            let width = fonts.measure(small, text) + 1.0;
            let area = TextBox {
                x,
                y,
                width,
                height: row_height,
                align: Align::Left,
            };
            fonts.draw(pixmap, small, text, area, color);
            x + width
        };

        let mut x = padding;
        for (number, input) in control.inputs.iter().enumerate() {
            if number > 0 {
                let joint = if control.sequence { "›" } else { "+" };
                x = text(fonts, pixmap, joint, x + gap) + gap;
            }
            match page.icons.get(input) {
                Some(icons) => {
                    let icon = &icons[selected];
                    let top = y + (row_height - icon.height() as f32) / 2.0;
                    pixmap.draw_pixmap(
                        x.round() as i32,
                        top.round() as i32,
                        icon.as_ref(),
                        &PixmapPaint::default(),
                        Transform::identity(),
                        None,
                    );
                    x += icon.width() as f32;
                    if let Some((_, Some(side))) = input_icon(input) {
                        x = text(fonts, pixmap, side, x);
                    }
                }
                None => x = text(fonts, pixmap, &input.to_uppercase(), x),
            }
        }
        x += padding * 0.5;

        let detail_width = if control.detail.is_empty() {
            0.0
        } else {
            let (detail_font, detail) = fonts.fit(small, &control.detail, (right - x) * 0.45, 0.8);
            let width = fonts.measure(&detail_font, &detail) + 1.0;
            let area = TextBox {
                x: right - width,
                y,
                width,
                height: row_height,
                align: Align::Right,
            };
            fonts.draw(pixmap, &detail_font, &detail, area, color);
            width + gap
        };
        let width = (right - x - detail_width).max(1.0);
        let (label_font, label) = fonts.fit(font, &control.label, width, 0.7);
        let area = TextBox {
            x,
            y,
            width,
            height: row_height,
            align: Align::Left,
        };
        fonts.draw(pixmap, &label_font, &label, area, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_have_es_icons() {
        assert_eq!(input_icon("b"), Some(("buttons_south.svg", None)));
        assert_eq!(input_icon("pageup"), Some(("button_l.svg", None)));
        assert_eq!(input_icon("r3"), Some(("analog_thumb.svg", Some("R"))));
        assert_eq!(
            input_icon("joystick1left"),
            Some(("analog_left.svg", Some("L")))
        );
        assert_eq!(
            input_icon("joystick2"),
            Some(("analog_stick.svg", Some("R")))
        );
        assert_eq!(input_icon("joystick3up"), None);
    }
}
