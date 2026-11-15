//! The on-screen keyboard: a grid of keys drawn with the menu style of the
//! active EmulationStation theme, moved through with a controller (or a
//! pointer) and typed on a virtual keyboard (see `Emulator::tap`).
//!
//! The keys are physical keys (evdev names), not characters: what a press
//! types depends on the layout the game's window has. The labels here are a
//! US layout's; the layout is the part that still has to come from the
//! active xkb keymap.
//!
//! The grid follows the usual controller keyboards: the digits on top, three
//! rows of letters, `LT` (symbols) and `RT` (shift) on the left, `X`
//! (backspace) and the enter key on the right, the cursor keys on the
//! shoulders, and a bottom row with close and the space bar.

use cosmic_text::Align;
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

use super::text::{FontRef, Fonts, TextBox};
use crate::theme::{MenuTheme, Rgba};

/// The grid's width in keys: ten keys between two side columns.
const COLUMNS: f32 = 13.2;
/// The side columns' width, in keys.
const SIDE: f32 = 1.6;
/// The grid's rows.
const ROWS: f32 = 5.0;
/// A row's height, relative to a key's width.
const ROW_PITCH: f32 = 0.74;
/// The space around the grid, and between keys, relative to a key's width.
const PADDING: f32 = 0.15;
const GAP: f32 = 0.06;
/// The keyboard's height, relative to the screen's.
const MAX_HEIGHT: f32 = 0.4;
/// Key caps: the opacity of the text colour over the background, normal,
/// pointed at and active (shift, symbols).
const CAP_ALPHA: u8 = 0x26;
const HOVER_ALPHA: u8 = 0x50;
const ACTIVE_ALPHA: u8 = 0x70;
/// Where the grid starts, in keys: the first letter column.
const LEFT: f32 = SIDE;

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A character key of the layer shown (the key's slot).
    Char(usize),
    Backspace,
    Space,
    Enter,
    CursorLeft,
    CursorRight,
    Shift,
    Symbols,
    Close,
    /// The accents page (not done yet: the key does nothing).
    Accents,
}

/// A key on the virtual keyboard to tap: its evdev name, with shift or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tap {
    pub code: &'static str,
    pub shift: bool,
}

/// What pressing a key came to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activation {
    pub tap: Option<Tap>,
    pub close: bool,
    /// The keyboard looks different now (a layer, shift).
    pub redraw: bool,
}

/// Which characters the keys show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Letters,
    Symbols,
}

/// A character key: its evdev name, its label and whether it is typed with
/// shift, on a layer.
type Char = (&'static str, &'static str, bool);

/// The 40 character keys, row by row, as `(evdev name, label, shift)`.
const LETTERS: [Char; 40] = [
    ("KEY_1", "1", false),
    ("KEY_2", "2", false),
    ("KEY_3", "3", false),
    ("KEY_4", "4", false),
    ("KEY_5", "5", false),
    ("KEY_6", "6", false),
    ("KEY_7", "7", false),
    ("KEY_8", "8", false),
    ("KEY_9", "9", false),
    ("KEY_0", "0", false),
    ("KEY_Q", "q", false),
    ("KEY_W", "w", false),
    ("KEY_E", "e", false),
    ("KEY_R", "r", false),
    ("KEY_T", "t", false),
    ("KEY_Y", "y", false),
    ("KEY_U", "u", false),
    ("KEY_I", "i", false),
    ("KEY_O", "o", false),
    ("KEY_P", "p", false),
    ("KEY_A", "a", false),
    ("KEY_S", "s", false),
    ("KEY_D", "d", false),
    ("KEY_F", "f", false),
    ("KEY_G", "g", false),
    ("KEY_H", "h", false),
    ("KEY_J", "j", false),
    ("KEY_K", "k", false),
    ("KEY_L", "l", false),
    ("KEY_SEMICOLON", ";", false),
    ("KEY_Z", "z", false),
    ("KEY_X", "x", false),
    ("KEY_C", "c", false),
    ("KEY_V", "v", false),
    ("KEY_B", "b", false),
    ("KEY_N", "n", false),
    ("KEY_M", "m", false),
    ("KEY_COMMA", ",", false),
    ("KEY_DOT", ".", false),
    ("KEY_APOSTROPHE", "'", false),
];

/// The same keys with shift: capitals and the digits' symbols.
const CAPITALS: [Char; 40] = [
    ("KEY_1", "!", true),
    ("KEY_2", "@", true),
    ("KEY_3", "#", true),
    ("KEY_4", "$", true),
    ("KEY_5", "%", true),
    ("KEY_6", "^", true),
    ("KEY_7", "&", true),
    ("KEY_8", "*", true),
    ("KEY_9", "(", true),
    ("KEY_0", ")", true),
    ("KEY_Q", "Q", true),
    ("KEY_W", "W", true),
    ("KEY_E", "E", true),
    ("KEY_R", "R", true),
    ("KEY_T", "T", true),
    ("KEY_Y", "Y", true),
    ("KEY_U", "U", true),
    ("KEY_I", "I", true),
    ("KEY_O", "O", true),
    ("KEY_P", "P", true),
    ("KEY_A", "A", true),
    ("KEY_S", "S", true),
    ("KEY_D", "D", true),
    ("KEY_F", "F", true),
    ("KEY_G", "G", true),
    ("KEY_H", "H", true),
    ("KEY_J", "J", true),
    ("KEY_K", "K", true),
    ("KEY_L", "L", true),
    ("KEY_SEMICOLON", ":", true),
    ("KEY_Z", "Z", true),
    ("KEY_X", "X", true),
    ("KEY_C", "C", true),
    ("KEY_V", "V", true),
    ("KEY_B", "B", true),
    ("KEY_N", "N", true),
    ("KEY_M", "M", true),
    ("KEY_COMMA", "<", true),
    ("KEY_DOT", ">", true),
    ("KEY_APOSTROPHE", "\"", true),
];

/// The symbols layer (`LT`).
const SYMBOLS: [Char; 40] = [
    ("KEY_1", "!", true),
    ("KEY_2", "@", true),
    ("KEY_3", "#", true),
    ("KEY_4", "$", true),
    ("KEY_5", "%", true),
    ("KEY_6", "^", true),
    ("KEY_7", "&", true),
    ("KEY_8", "*", true),
    ("KEY_9", "(", true),
    ("KEY_0", ")", true),
    ("KEY_LEFTBRACE", "[", false),
    ("KEY_RIGHTBRACE", "]", false),
    ("KEY_LEFTBRACE", "{", true),
    ("KEY_RIGHTBRACE", "}", true),
    ("KEY_BACKSLASH", "\\", false),
    ("KEY_BACKSLASH", "|", true),
    ("KEY_GRAVE", "`", false),
    ("KEY_GRAVE", "~", true),
    ("KEY_COMMA", "<", true),
    ("KEY_DOT", ">", true),
    ("KEY_MINUS", "-", false),
    ("KEY_MINUS", "_", true),
    ("KEY_EQUAL", "=", false),
    ("KEY_EQUAL", "+", true),
    ("KEY_SEMICOLON", ";", false),
    ("KEY_SEMICOLON", ":", true),
    ("KEY_APOSTROPHE", "'", false),
    ("KEY_APOSTROPHE", "\"", true),
    ("KEY_SLASH", "/", false),
    ("KEY_SLASH", "?", true),
    ("KEY_COMMA", ",", false),
    ("KEY_DOT", ".", false),
    ("KEY_1", "!", true),
    ("KEY_SLASH", "?", true),
    ("KEY_APOSTROPHE", "'", false),
    ("KEY_APOSTROPHE", "\"", true),
    ("KEY_MINUS", "-", false),
    ("KEY_MINUS", "_", true),
    ("KEY_SLASH", "/", false),
    ("KEY_BACKSLASH", "\\", false),
];

/// A key: what it does, where it is (pixels in the keyboard's image) and
/// the controller button that does it, when it has one.
#[derive(Debug, Clone)]
struct Key {
    action: Action,
    rect: (f32, f32, f32, f32),
    badge: Option<&'static str>,
}

impl Key {
    fn centre(&self) -> (f32, f32) {
        (
            self.rect.0 + self.rect.2 / 2.0,
            self.rect.1 + self.rect.3 / 2.0,
        )
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.rect.0
            && x < self.rect.0 + self.rect.2
            && y >= self.rect.1
            && y < self.rect.1 + self.rect.3
    }
}

/// A direction to move the selection in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// The on-screen keyboard.
pub struct Osk {
    /// The keyboard's size in pixels.
    pub width: u32,
    pub height: u32,
    keys: Vec<Key>,
    selected: usize,
    hovered: Option<usize>,
    layer: Layer,
    /// Shift is on for the next character.
    shift: bool,
    background: Rgba,
    separator: Rgba,
    text_color: Rgba,
    selected_text: Rgba,
    selector: Rgba,
    font: FontRef,
    badge_font: FontRef,
    unit: f32,
}

impl Osk {
    /// A keyboard sized for a screen, in the theme's colours and font.
    pub fn new(theme: &MenuTheme, screen: (u32, u32)) -> Self {
        let (screen_width, screen_height) = (screen.0 as f32, screen.1 as f32);
        // the key's width: the height limit, or the screen's width
        let unit = (screen_height * MAX_HEIGHT / (ROWS * ROW_PITCH + 2.0 * PADDING))
            .min(screen_width / (COLUMNS + 2.0 * PADDING))
            .floor()
            .max(8.0);
        let width = ((COLUMNS + 2.0 * PADDING) * unit).round();
        let height = ((ROWS * ROW_PITCH + 2.0 * PADDING) * unit).round();
        let keys = Self::layout(unit);
        let selected = keys
            .iter()
            .position(|key| key.action == Action::Char(22))
            .unwrap_or(0);
        let style = &theme.text;
        let font = |size: f32| FontRef {
            path: style.font.path.clone(),
            size,
            line_height: size * 1.3,
            bold: false,
        };
        Self {
            width: width as u32,
            height: height as u32,
            keys,
            selected,
            hovered: None,
            layer: Layer::Letters,
            shift: false,
            background: theme.background.center_color,
            separator: style.separator_color,
            text_color: style.color,
            selected_text: style.selected_color,
            selector: style.selector_color,
            font: font(unit * 0.4),
            badge_font: font(unit * 0.2),
            unit,
        }
    }

    /// The keys' places, in pixels.
    fn layout(unit: f32) -> Vec<Key> {
        let pitch = unit * ROW_PITCH;
        let (pad, gap) = (unit * PADDING, unit * GAP);
        let mut keys = Vec::new();
        let mut add = |action, x: f32, y: f32, w: f32, h: f32, badge| {
            keys.push(Key {
                action,
                rect: (
                    pad + x * unit + gap / 2.0,
                    pad + y * pitch + gap / 2.0,
                    w * unit - gap,
                    h * pitch - gap,
                ),
                badge,
            });
        };
        add(Action::Symbols, 0.0, 0.0, SIDE, 1.0, Some("L2"));
        add(Action::CursorLeft, 0.0, 1.0, SIDE, 2.0, Some("L"));
        add(Action::Shift, 0.0, 3.0, SIDE, 1.0, Some("R2"));
        add(Action::Backspace, LEFT + 10.0, 0.0, SIDE, 1.0, Some("Y"));
        add(Action::CursorRight, LEFT + 10.0, 1.0, SIDE, 2.0, Some("R"));
        add(Action::Enter, LEFT + 10.0, 3.0, SIDE, 1.0, Some("+"));
        for row in 0..4 {
            for column in 0..10 {
                add(
                    Action::Char(row * 10 + column),
                    LEFT + column as f32,
                    row as f32,
                    1.0,
                    1.0,
                    None,
                );
            }
        }
        add(Action::Close, LEFT, 4.0, 2.0, 1.0, Some("Home"));
        add(Action::Space, LEFT + 2.0, 4.0, 6.0, 1.0, Some("X"));
        add(Action::Accents, LEFT + 8.0, 4.0, 2.0, 1.0, Some("A"));
        keys
    }

    fn chars(&self) -> &'static [Char; 40] {
        match (self.layer, self.shift) {
            (Layer::Symbols, _) => &SYMBOLS,
            (Layer::Letters, true) => &CAPITALS,
            (Layer::Letters, false) => &LETTERS,
        }
    }

    /// The key under a point of the keyboard's image.
    pub fn key_at(&self, x: f32, y: f32) -> Option<usize> {
        self.keys.iter().position(|key| key.contains(x, y))
    }

    /// Highlight the key under the pointer (None: none); whether it changed.
    pub fn hover(&mut self, key: Option<usize>) -> bool {
        let changed = self.hovered != key;
        self.hovered = key;
        changed
    }

    /// Select a key (the pointer pressed it).
    pub fn select(&mut self, key: usize) {
        self.selected = key;
    }

    /// Move the selection to the nearest key in a direction; whether it moved.
    pub fn move_selection(&mut self, direction: Direction) -> bool {
        let (cx, cy) = self.keys[self.selected].centre();
        let best = self
            .keys
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.selected)
            .filter_map(|(index, key)| {
                let (x, y) = key.centre();
                let (along, across) = match direction {
                    Direction::Right => (x - cx, (y - cy).abs()),
                    Direction::Left => (cx - x, (y - cy).abs()),
                    Direction::Down => (y - cy, (x - cx).abs()),
                    Direction::Up => (cy - y, (x - cx).abs()),
                };
                // strictly past the current key (a tall key's neighbours
                // can share its middle), and nearer along than across
                (along > self.unit * 0.2).then_some((index, along + 2.0 * across))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index);
        match best {
            Some(index) => {
                self.selected = index;
                true
            }
            None => false,
        }
    }

    /// The key that does an action.
    pub fn index_of(&self, action: Action) -> Option<usize> {
        self.keys.iter().position(|key| key.action == action)
    }

    /// The selected key.
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Press a key.
    pub fn activate(&mut self, key: usize) -> Activation {
        let tap = |code| Activation {
            tap: Some(Tap { code, shift: false }),
            ..Activation::default()
        };
        match self.keys[key].action {
            Action::Char(slot) => {
                let (code, _, shifted) = self.chars()[slot];
                // shift is for one character
                let redraw = std::mem::take(&mut self.shift);
                Activation {
                    tap: Some(Tap {
                        code,
                        shift: shifted,
                    }),
                    redraw,
                    ..Activation::default()
                }
            }
            Action::Backspace => tap("KEY_BACKSPACE"),
            Action::Space => tap("KEY_SPACE"),
            Action::Enter => tap("KEY_ENTER"),
            Action::CursorLeft => tap("KEY_LEFT"),
            Action::CursorRight => tap("KEY_RIGHT"),
            Action::Shift => {
                self.shift = !self.shift;
                Activation {
                    redraw: true,
                    ..Activation::default()
                }
            }
            Action::Symbols => {
                self.layer = match self.layer {
                    Layer::Letters => Layer::Symbols,
                    Layer::Symbols => Layer::Letters,
                };
                self.shift = false;
                Activation {
                    redraw: true,
                    ..Activation::default()
                }
            }
            Action::Close => Activation {
                close: true,
                ..Activation::default()
            },
            Action::Accents => Activation::default(),
        }
    }

    /// What a key shows.
    fn label(&self, action: Action) -> &'static str {
        match action {
            Action::Char(slot) => self.chars()[slot].1,
            Action::Backspace => "Del",
            Action::Space => "Space",
            Action::Enter => "Enter",
            Action::CursorLeft => "<",
            Action::CursorRight => ">",
            Action::Shift => "Shift",
            Action::Symbols => "#+=",
            Action::Close => "Close",
            Action::Accents => "Accents",
        }
    }

    /// Whether a toggle key is on.
    fn is_on(&self, action: Action) -> bool {
        match action {
            Action::Shift => self.shift,
            Action::Symbols => self.layer == Layer::Symbols,
            _ => false,
        }
    }

    /// Draw the keyboard.
    pub fn render(&self, fonts: &mut Fonts) -> Option<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, self.height)?;
        pixmap.fill(self.background.to_color());
        fill_rect(
            &mut pixmap,
            (0.0, 0.0, self.width as f32, (self.unit * 0.03).max(1.0)),
            self.separator,
        );
        let radius = self.unit * 0.12;
        for (index, key) in self.keys.iter().enumerate() {
            let selected = index == self.selected;
            let alpha = if self.is_on(key.action) {
                ACTIVE_ALPHA
            } else if self.hovered == Some(index) {
                HOVER_ALPHA
            } else {
                CAP_ALPHA
            };
            let (cap, text) = if selected {
                (self.selector, self.selected_text)
            } else {
                (self.text_color.with_alpha(alpha), self.text_color)
            };
            // what does nothing yet is dimmed
            let text = if key.action == Action::Accents {
                text.with_alpha(0x60)
            } else {
                text
            };
            let (x, y, w, h) = key.rect;
            if let Some(path) = rounded_rect(x, y, w, h, radius) {
                fill_path(&mut pixmap, &path, cap);
            }
            let label_height = if key.badge.is_some() { h * 0.62 } else { h };
            fonts.draw(
                &mut pixmap,
                &self.font,
                self.label(key.action),
                TextBox {
                    x,
                    y,
                    width: w,
                    height: label_height,
                    align: Align::Center,
                },
                text,
            );
            if let Some(badge) = key.badge {
                self.draw_badge(&mut pixmap, fonts, badge, key.rect, text);
            }
        }
        Some(pixmap)
    }

    /// The controller button that does a key, in a small outlined box under
    /// its label.
    fn draw_badge(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        badge: &str,
        (x, y, w, h): (f32, f32, f32, f32),
        color: Rgba,
    ) {
        let text_width = fonts.measure(&self.badge_font, badge);
        let line = self.badge_font.line_height;
        let box_width = (text_width + self.unit * 0.2).max(line);
        let left = x + (w - box_width) / 2.0;
        let top = y + h * 0.62;
        if let Some(path) = rounded_rect(left, top, box_width, line, line / 2.0) {
            let mut paint = Paint::default();
            paint.set_color(color.with_alpha(0xB0).to_color());
            paint.anti_alias = true;
            let stroke = Stroke {
                width: (self.unit * 0.02).max(1.0),
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
        fonts.draw(
            pixmap,
            &self.badge_font,
            badge,
            TextBox {
                x: left,
                y: top,
                width: box_width,
                height: line,
                align: Align::Center,
            },
            color.with_alpha(0xB0),
        );
    }
}

fn fill_rect(pixmap: &mut Pixmap, (x, y, width, height): (f32, f32, f32, f32), color: Rgba) {
    if let Some(rect) = tiny_skia::Rect::from_xywh(x, y, width, height) {
        let mut paint = Paint::default();
        paint.set_color(color.to_color());
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

fn fill_path(pixmap: &mut Pixmap, path: &tiny_skia::Path, color: Rgba) {
    let mut paint = Paint::default();
    paint.set_color(color.to_color());
    paint.anti_alias = true;
    pixmap.fill_path(
        path,
        &paint,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
}

/// A rectangle with rounded corners.
fn rounded_rect(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.min(width / 2.0).min(height / 2.0);
    let (right, bottom) = (x + width, y + height);
    let mut builder = PathBuilder::new();
    builder.move_to(x + r, y);
    builder.line_to(right - r, y);
    builder.quad_to(right, y, right, y + r);
    builder.line_to(right, bottom - r);
    builder.quad_to(right, bottom, right - r, bottom);
    builder.line_to(x + r, bottom);
    builder.quad_to(x, bottom, x, bottom - r);
    builder.line_to(x, y + r);
    builder.quad_to(x, y, x + r, y);
    builder.close();
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osk() -> Osk {
        Osk::new(&MenuTheme::default(), (1920, 1080))
    }

    fn selected_action(osk: &Osk) -> Action {
        osk.keys[osk.selected()].action
    }

    #[test]
    fn fits_the_screen_and_starts_on_d() {
        let osk = osk();
        assert!(osk.width <= 1920 && osk.height <= (1080.0 * 0.41) as u32);
        let Action::Char(slot) = selected_action(&osk) else {
            panic!("a character key is selected");
        };
        assert_eq!(LETTERS[slot].1, "d");
    }

    #[test]
    fn moves_across_the_grid() {
        let mut osk = osk();
        assert!(osk.move_selection(Direction::Right));
        assert_eq!(osk.label(selected_action(&osk)), "f");
        assert!(osk.move_selection(Direction::Up));
        assert_eq!(osk.label(selected_action(&osk)), "r");
        assert!(osk.move_selection(Direction::Down));
        assert!(osk.move_selection(Direction::Down));
        assert!(osk.move_selection(Direction::Down));
        // the bottom row: the space bar is under the middle
        assert_eq!(selected_action(&osk), Action::Space);
        assert!(!osk.move_selection(Direction::Down));
    }

    #[test]
    fn side_columns_are_reached_from_the_letters() {
        let mut osk = osk();
        while osk.move_selection(Direction::Left) {}
        assert_eq!(selected_action(&osk), Action::CursorLeft);
        assert!(osk.move_selection(Direction::Up));
        assert_eq!(selected_action(&osk), Action::Symbols);
        assert!(osk.move_selection(Direction::Down));
        assert!(osk.move_selection(Direction::Down));
        assert_eq!(selected_action(&osk), Action::Shift);
    }

    #[test]
    fn shift_is_for_one_character() {
        let mut osk = osk();
        let shift = osk
            .keys
            .iter()
            .position(|key| key.action == Action::Shift)
            .unwrap();
        assert!(osk.activate(shift).redraw);
        let key = osk
            .keys
            .iter()
            .position(|key| key.action == Action::Char(10))
            .unwrap();
        let typed = osk.activate(key);
        assert_eq!(
            typed.tap,
            Some(Tap {
                code: "KEY_Q",
                shift: true
            })
        );
        assert!(typed.redraw);
        let again = osk.activate(key);
        assert_eq!(again.tap.map(|tap| tap.shift), Some(false));
        assert!(!again.redraw);
    }

    #[test]
    fn symbols_layer_and_close() {
        let mut osk = osk();
        let symbols = osk
            .keys
            .iter()
            .position(|key| key.action == Action::Symbols)
            .unwrap();
        osk.activate(symbols);
        let at = osk.key_at(osk.keys[0].centre().0, osk.keys[0].centre().1);
        assert_eq!(at, Some(0));
        let close = osk
            .keys
            .iter()
            .position(|key| key.action == Action::Close)
            .unwrap();
        assert!(osk.activate(close).close);
    }

    #[test]
    fn every_key_is_inside_the_image() {
        let osk = osk();
        for key in &osk.keys {
            let (x, y, w, h) = key.rect;
            assert!(x >= 0.0 && y >= 0.0);
            assert!(x + w <= osk.width as f32 && y + h <= osk.height as f32);
        }
    }
}
