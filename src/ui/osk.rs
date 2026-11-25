//! The on-screen keyboard: a grid of keys drawn with the menu style of the
//! active EmulationStation theme, moved through with a controller (or a
//! pointer) and typed on a virtual keyboard (see `Emulator::tap`).
//!
//! The keys are physical keys (evdev names), not characters: what a press
//! types depends on the layout the game's window has. The labels and what
//! each key types come from the active xkb keymap (see `layout`).
//!
//! The grid follows the usual controller keyboards: the digits on top, three
//! rows of letters, `LT` (symbols) and `RT` (shift) on the left, `X`
//! (backspace) and the enter key on the right, the cursor keys on the
//! shoulders, and a bottom row with close and the space bar.

use cosmic_text::Align;
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

use std::collections::HashMap;
use std::path::Path;

use super::icons::Icon;
use super::layout::{Layout, Page, Slot, Step};
use super::text::{FontRef, Fonts, TextBox};
use crate::config::OskView;
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
const GAP: f32 = 0.1;
/// The radius of the keyboard's own corners, in key units.
const WINDOW_RADIUS: f32 = 0.2;
/// The keyboard's height, relative to the screen's.
const MAX_HEIGHT: f32 = 0.4;
/// Key caps: the opacity of the text colour over the background, normal,
/// pointed at and active (shift, symbols).
const CAP_ALPHA: u8 = 0x26;
const HOVER_ALPHA: u8 = 0x60;
/// The selected key's ring: its thickness and its gap from the key,
/// relative to a key's width.
const RING_WIDTH: f32 = 0.035;
const RING_GAP: f32 = 0.02;
const ACTIVE_ALPHA: u8 = 0x70;
/// An icon's size on a key, relative to a key's width.
const ICON_SIZE: f32 = 0.42;
/// Where the grid starts, in keys: the first letter column.
const LEFT: f32 = SIDE;

/// The words on the keys that have one (the others have icons).
#[derive(Debug, Clone)]
pub struct Labels {
    pub accents: String,
}

/// A badge's icon, plain and on a selected key.
type BadgeIcon = (Pixmap, Pixmap);

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A character key of the layer shown (the key's slot).
    Char(usize),
    Backspace,
    Space,
    Enter,
    Tab,
    /// Leave for the overlay.
    Overlay,
    CursorLeft,
    CursorRight,
    Shift,
    Symbols,
    Close,
    /// The accents page.
    Accents,
}

/// What pressing a key came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Activation {
    /// The keys to tap, in order (an accent is a dead key and a letter).
    pub taps: Vec<Step>,
    pub close: bool,
    /// Close it and open the overlay.
    pub overlay: bool,
    /// The keyboard looks different now (a layer, shift).
    pub redraw: bool,
}

/// Which characters the keys show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Letters,
    Symbols,
    Accents,
}

/// A key: what it does, where it is (pixels in the keyboard's image) and
/// the controller button that does it, when it has one.
#[derive(Debug, Clone)]
struct Key {
    action: Action,
    rect: (f32, f32, f32, f32),
    /// The help icon's name (`y`, `start`...), written when the theme has
    /// no such icon.
    badge: Option<String>,
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
    /// How big it is and which edge it is at.
    pub view: OskView,
    keys: Vec<Key>,
    selected: usize,
    hovered: Option<usize>,
    layout: Layout,
    labels: Labels,
    icons: HashMap<String, BadgeIcon>,
    layer: Layer,
    /// Shift is locked on (capitals) until pressed again; it starts on.
    shift: bool,
    background: Rgba,
    separator: Rgba,
    text_color: Rgba,
    selected_text: Rgba,
    selector: Rgba,
    font: FontRef,
    /// For the keys with a word on them.
    word_font: FontRef,
    badge_font: FontRef,
    unit: f32,
}

impl Osk {
    /// A keyboard sized for a screen, in the theme's colours and font.
    pub fn new(
        theme: &MenuTheme,
        screen: (u32, u32),
        view: OskView,
        layout: Layout,
        labels: Labels,
    ) -> Self {
        let (screen_width, screen_height) = (screen.0 as f32, screen.1 as f32);
        // the key's width: the height limit, or the width the view asks for
        let unit = (screen_height * MAX_HEIGHT / (ROWS * ROW_PITCH + 2.0 * PADDING))
            .min(screen_width * view.width / (COLUMNS + 2.0 * PADDING))
            .floor()
            .max(8.0);
        let width = ((COLUMNS + 2.0 * PADDING) * unit).round();
        let height = ((ROWS * ROW_PITCH + 2.0 * PADDING) * unit).round();
        let keys = Self::layout(unit, &theme.help.back);
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
        let icons = Self::badge_icons(theme, &keys, (unit * 0.26).round() as u32);
        Self {
            width: width as u32,
            height: height as u32,
            view,
            keys,
            selected,
            hovered: None,
            layout,
            labels,
            icons,
            layer: Layer::Letters,
            shift: true,
            background: theme.background.center_color,
            separator: style.separator_color,
            text_color: style.color,
            selected_text: style.selected_color,
            selector: style.selector_color,
            font: font(unit * 0.4),
            word_font: font(unit * 0.28),
            badge_font: font(unit * 0.2),
            unit,
        }
    }

    /// This keyboard in another view, with what the user has done to it (the
    /// key selected, the layer, shift) kept.
    #[must_use]
    pub fn reshaped(&self, theme: &MenuTheme, screen: (u32, u32), view: OskView) -> Self {
        let mut osk = Self::new(
            theme,
            screen,
            view,
            self.layout.clone(),
            self.labels.clone(),
        );
        osk.selected = self.selected;
        osk.layer = self.layer;
        osk.shift = self.shift;
        osk
    }

    /// The theme's icons for the controller buttons the keys name, drawn in
    /// the text colour and in the selected one.
    fn badge_icons(theme: &MenuTheme, keys: &[Key], height: u32) -> HashMap<String, BadgeIcon> {
        let fallback = Path::new(crate::theme::ES_HELP_ICONS);
        keys.iter()
            .filter_map(|key| key.badge.as_ref())
            .filter_map(|name| {
                let path = theme
                    .help
                    .icons
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| fallback.join(format!("button_{name}.svg")));
                let draw = |tint| super::help::draw_svg(&path, height, tint);
                let normal = draw(theme.text.color.with_alpha(0xB0))?;
                let selected = draw(theme.text.selected_color.with_alpha(0xB0))?;
                Some((name.clone(), (normal, selected)))
            })
            .collect()
    }

    /// The keys' places, in pixels.
    fn layout(unit: f32, accents_icon: &str) -> Vec<Key> {
        let pitch = unit * ROW_PITCH;
        let (pad, gap) = (unit * PADDING, unit * GAP);
        let mut keys = Vec::new();
        let mut add = |action, x: f32, y: f32, w: f32, h: f32, badge: Option<&str>| {
            keys.push(Key {
                action,
                rect: (
                    pad + x * unit + gap / 2.0,
                    pad + y * pitch + gap / 2.0,
                    w * unit - gap,
                    h * pitch - gap,
                ),
                badge: badge.map(str::to_owned),
            });
        };
        let right = LEFT + 10.0;
        // the side columns: layers and tab on the left, the overlay and
        // delete on the right, with shift and enter twice as tall
        add(Action::Symbols, 0.0, 0.0, SIDE, 1.0, Some("l2"));
        add(Action::Tab, 0.0, 1.0, SIDE, 1.0, None);
        add(Action::Shift, 0.0, 2.0, SIDE, 2.0, Some("r2"));
        add(Action::Overlay, right, 0.0, SIDE, 1.0, None);
        add(Action::Backspace, right, 1.0, SIDE, 1.0, Some("y"));
        add(Action::Enter, right, 2.0, SIDE, 2.0, Some("start"));
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
        // the bottom row: the cursors on both sides of the space bar, under
        // the first and last two letters, and close and accents under the
        // side columns
        add(Action::Close, 0.0, 4.0, SIDE, 1.0, Some("hotkey"));
        add(Action::CursorLeft, LEFT, 4.0, 2.0, 1.0, Some("l"));
        add(Action::Space, LEFT + 2.0, 4.0, 6.0, 1.0, Some("x"));
        add(Action::CursorRight, LEFT + 8.0, 4.0, 2.0, 1.0, Some("r"));
        // the cancel button, wherever the theme has put it
        add(Action::Accents, right, 4.0, SIDE, 1.0, Some(accents_icon));
        keys
    }

    /// The page of characters on show.
    fn page(&self) -> Page {
        match (self.layer, self.shift) {
            (Layer::Symbols, _) => Page::Symbols,
            (Layer::Accents, false) => Page::Accents,
            (Layer::Accents, true) => Page::AccentCapitals,
            (Layer::Letters, false) => Page::Letters,
            (Layer::Letters, true) => Page::Capitals,
        }
    }

    fn slot(&self, slot: usize) -> &Slot {
        &self.layout.page(self.page())[slot]
    }

    /// Change what the keys type (the compositor's keymap changed).
    pub fn set_layout(&mut self, layout: Layout) -> bool {
        let changed = self.layout != layout;
        self.layout = layout;
        changed
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

    /// Whether a key types (and so repeats when held).
    pub fn types(&self, key: usize) -> bool {
        matches!(
            self.keys[key].action,
            Action::Char(_)
                | Action::Backspace
                | Action::Space
                | Action::Enter
                | Action::Tab
                | Action::CursorLeft
                | Action::CursorRight
        )
    }

    /// Press a key.
    pub fn activate(&mut self, key: usize) -> Activation {
        let tap = |code| Activation {
            taps: vec![Step {
                code,
                shift: false,
                altgr: false,
            }],
            ..Activation::default()
        };
        // a layer key goes back to the letters when pressed again
        let toggle = |osk: &mut Self, layer| {
            osk.layer = if osk.layer == layer {
                Layer::Letters
            } else {
                layer
            };
            Activation {
                redraw: true,
                ..Activation::default()
            }
        };
        match self.keys[key].action {
            Action::Char(slot) => Activation {
                taps: self.slot(slot).steps.clone(),
                ..Activation::default()
            },
            Action::Backspace => tap("KEY_BACKSPACE"),
            Action::Space => tap("KEY_SPACE"),
            Action::Enter => tap("KEY_ENTER"),
            Action::Tab => tap("KEY_TAB"),
            Action::Overlay => Activation {
                close: true,
                overlay: true,
                ..Activation::default()
            },
            Action::CursorLeft => tap("KEY_LEFT"),
            Action::CursorRight => tap("KEY_RIGHT"),
            Action::Shift => {
                self.shift = !self.shift;
                Activation {
                    redraw: true,
                    ..Activation::default()
                }
            }
            Action::Symbols => toggle(self, Layer::Symbols),
            Action::Accents => toggle(self, Layer::Accents),
            Action::Close => Activation {
                close: true,
                ..Activation::default()
            },
        }
    }

    /// The word on a key: a character, or the symbols and accents keys'.
    fn label(&self, action: Action) -> &str {
        match action {
            Action::Char(slot) => &self.slot(slot).label,
            Action::Symbols => "#+=",
            Action::Accents => &self.labels.accents,
            _ => "",
        }
    }

    /// The icon on a key, for the keys that have one.
    fn icon(&self, action: Action) -> Option<Icon> {
        Some(match action {
            Action::Backspace => Icon::Delete,
            Action::Space => Icon::Space,
            Action::Enter => Icon::CornerDownLeft,
            Action::Tab => Icon::ArrowRightToLine,
            Action::Overlay => Icon::Gamepad,
            Action::CursorLeft => Icon::ArrowLeft,
            Action::CursorRight => Icon::ArrowRight,
            // locked on, shift is caps lock
            Action::Shift if self.shift => Icon::ArrowBigUpDash,
            Action::Shift => Icon::ArrowBigUp,
            Action::Close => Icon::X,
            Action::Char(_) | Action::Symbols | Action::Accents => return None,
        })
    }

    /// Whether a toggle key is on.
    fn is_on(&self, action: Action) -> bool {
        match action {
            Action::Shift => self.shift,
            Action::Symbols => self.layer == Layer::Symbols,
            Action::Accents => self.layer == Layer::Accents,
            _ => false,
        }
    }

    /// Draw the keyboard.
    pub fn render(&self, fonts: &mut Fonts) -> Option<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, self.height)?;
        pixmap.fill(self.background.to_color());
        // the line on the edge that faces the screen
        let line = (self.unit * 0.03).max(1.0);
        let line_y = if self.view.top {
            self.height as f32 - line
        } else {
            0.0
        };
        fill_rect(
            &mut pixmap,
            (0.0, line_y, self.width as f32, line),
            self.separator,
        );
        let radius = self.unit * 0.12;
        for (index, key) in self.keys.iter().enumerate() {
            let selected = index == self.selected;
            // the selected key is the one a button press types: the selector's
            // colour and a ring; the one under the pointer is only tinted
            let (cap, text) = if selected {
                (self.selector, self.selected_text)
            } else if self.hovered == Some(index) {
                (self.selector.with_alpha(HOVER_ALPHA), self.text_color)
            } else if self.is_on(key.action) {
                (self.text_color.with_alpha(ACTIVE_ALPHA), self.text_color)
            } else {
                (self.text_color.with_alpha(CAP_ALPHA), self.text_color)
            };
            // a character this layout cannot type is left blank
            let text = if matches!(key.action, Action::Char(slot) if self.slot(slot).steps.is_empty())
            {
                text.with_alpha(0x60)
            } else {
                text
            };
            let (x, y, w, h) = key.rect;
            if let Some(path) = rounded_rect(x, y, w, h, radius) {
                fill_path(&mut pixmap, &path, cap);
            }
            if selected {
                // a ring in the text's colour, as the round buttons of the
                // menu have: it stands out whatever the theme's selector
                let (grow, width) = (self.unit * RING_GAP, (self.unit * RING_WIDTH).max(2.0));
                let ring = rounded_rect(
                    x - grow,
                    y - grow,
                    w + 2.0 * grow,
                    h + 2.0 * grow,
                    radius + grow,
                );
                if let Some(path) = ring {
                    stroke_path(&mut pixmap, &path, width, self.text_color);
                }
            }
            let label_height = if key.badge.is_some() { h * 0.62 } else { h };
            let area = TextBox {
                x,
                y,
                width: w,
                height: label_height,
                align: Align::Center,
            };
            if let Some(icon) = self.icon(key.action) {
                let size = (self.unit * ICON_SIZE).round().max(1.0) as u32;
                if let Some(image) = icon.render(size, text) {
                    let left = x + (w - size as f32) / 2.0;
                    let top = y + (label_height - size as f32) / 2.0;
                    crate::ui::blit(
                        &mut pixmap,
                        &image,
                        (left.round() as i32, top.round() as i32),
                    );
                }
            } else {
                let font = if matches!(key.action, Action::Char(_)) {
                    &self.font
                } else {
                    &self.word_font
                };
                fonts.draw(&mut pixmap, font, self.label(key.action), area, text);
            }
            if let Some(badge) = &key.badge {
                self.draw_badge(&mut pixmap, fonts, badge, key.rect, (text, selected));
            }
        }
        self.round_outer_corners(&mut pixmap);
        Some(pixmap)
    }

    /// Round the keyboard's corners that face the screen (the others are past
    /// its edge), a little more than the keys': what is outside is
    /// transparent.
    fn round_outer_corners(&self, pixmap: &mut Pixmap) {
        let radius = self.unit * WINDOW_RADIUS;
        let (width, height) = (self.width as f32, self.height as f32);
        // the path goes past the canvas: only the corners facing the screen show
        let overshoot = if self.view.top { -radius } else { 0.0 };
        let Some(path) = rounded_rect(0.0, overshoot, width, height + radius, radius) else {
            return;
        };
        if let Some(mut mask) = tiny_skia::Mask::new(self.width, self.height) {
            mask.fill_path(
                &path,
                tiny_skia::FillRule::Winding,
                true,
                Transform::identity(),
            );
            pixmap.apply_mask(&mask);
        }
    }

    /// The controller button that does a key, in a small outlined box under
    /// its label.
    fn draw_badge(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        name: &str,
        (x, y, w, h): (f32, f32, f32, f32),
        (color, selected): (Rgba, bool),
    ) {
        if let Some((normal, highlighted)) = self.icons.get(name) {
            let icon = if selected { highlighted } else { normal };
            let left = x + (w - icon.width() as f32) / 2.0;
            let top = y + h * 0.62 + (self.badge_font.line_height - icon.height() as f32) / 2.0;
            crate::ui::blit(pixmap, icon, (left.round() as i32, top.round() as i32));
            return;
        }
        let written = match name {
            "start" => "+".to_owned(),
            "hotkey" => "Hotkey".to_owned(),
            other => other.to_uppercase(),
        };
        let badge = written.as_str();
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

fn stroke_path(pixmap: &mut Pixmap, path: &tiny_skia::Path, width: f32, color: Rgba) {
    let mut paint = Paint::default();
    paint.set_color(color.to_color());
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
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

    fn labels() -> Labels {
        Labels {
            accents: "Accents".to_owned(),
        }
    }

    /// A keyboard with shift off (it starts locked on).
    fn osk() -> Osk {
        let mut osk = capitals_osk();
        osk.shift = false;
        osk
    }

    fn capitals_osk() -> Osk {
        Osk::new(
            &MenuTheme::default(),
            (1920, 1080),
            OskView::default(),
            Layout::fallback(),
            labels(),
        )
    }

    fn selected_action(osk: &Osk) -> Action {
        osk.keys[osk.selected()].action
    }

    #[test]
    fn the_view_sizes_and_places_the_keyboard_and_keeps_its_state() {
        let mut osk = osk();
        osk.move_selection(Direction::Right);
        let screen = (1920, 1080);
        let narrow = osk.reshaped(
            &MenuTheme::default(),
            screen,
            osk.view.resized(-0.3).flipped(),
        );
        assert!(narrow.width < osk.width && narrow.view.top);
        assert!(f32::from(u16::try_from(narrow.width).unwrap()) <= 1920.0 * 0.46);
        assert_eq!(narrow.selected(), osk.selected());
        assert!(!narrow.shift);
        let small = osk.reshaped(
            &MenuTheme::default(),
            screen,
            OskView::default().resized(-1.0),
        );
        assert!(small.width >= 100 && small.width <= 1920 / 10 + 14);
    }

    #[test]
    fn fits_the_screen_and_starts_on_d() {
        let osk = osk();
        assert!(osk.width <= 1920 && osk.height <= (1080.0 * 0.41) as u32);
        let Action::Char(slot) = selected_action(&osk) else {
            panic!("a character key is selected");
        };
        assert_eq!(osk.slot(slot).label, "d");
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
        // shift is two rows tall: the row of d and the one under it
        assert_eq!(selected_action(&osk), Action::Shift);
        assert!(osk.move_selection(Direction::Up));
        assert_eq!(selected_action(&osk), Action::Tab);
        assert!(osk.move_selection(Direction::Up));
        assert_eq!(selected_action(&osk), Action::Symbols);
        assert!(osk.move_selection(Direction::Down));
        assert!(osk.move_selection(Direction::Down));
        assert!(osk.move_selection(Direction::Down));
        assert_eq!(selected_action(&osk), Action::Close);
    }

    #[test]
    fn the_bottom_row_has_the_cursors_around_the_space_bar() {
        let osk = osk();
        let bottom: Vec<Action> = {
            let mut keys: Vec<_> = osk
                .keys
                .iter()
                .filter(|key| key.rect.1 > osk.height as f32 * 0.7)
                .collect();
            keys.sort_by(|a, b| a.rect.0.total_cmp(&b.rect.0));
            keys.iter().map(|key| key.action).collect()
        };
        assert_eq!(
            bottom,
            [
                Action::Close,
                Action::CursorLeft,
                Action::Space,
                Action::CursorRight,
                Action::Accents
            ]
        );
    }

    #[test]
    fn the_overlay_key_closes_for_the_overlay() {
        let mut osk = osk();
        let key = osk.index_of(Action::Overlay).unwrap();
        let pressed = osk.activate(key);
        assert!(pressed.close && pressed.overlay);
        let tab = osk.index_of(Action::Tab).unwrap();
        assert_eq!(osk.activate(tab).taps[0].code, "KEY_TAB");
    }

    #[test]
    fn shift_starts_locked_and_stays() {
        let mut osk = capitals_osk();
        let q = osk
            .keys
            .iter()
            .position(|key| key.action == Action::Char(10))
            .unwrap();
        let typed = osk.activate(q);
        assert_eq!(
            typed.taps,
            vec![Step {
                code: "KEY_Q",
                shift: true,
                altgr: false
            }]
        );
        // still capitals for the next one, until shift is pressed
        assert!(!typed.redraw);
        assert!(osk.activate(q).taps[0].shift);
        let shift = osk
            .keys
            .iter()
            .position(|key| key.action == Action::Shift)
            .unwrap();
        assert!(osk.activate(shift).redraw);
        assert!(!osk.activate(q).taps[0].shift);
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
