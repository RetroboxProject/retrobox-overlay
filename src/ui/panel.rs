//! The side menu, laid out and coloured like EmulationStation's game options
//! menu (`GuiGameOptions` with side menus: full height, slid in from an edge).
//!
//! Sizes follow ES: the width is half the screen or 33 times the width of an
//! "S" in the menu font, whichever is smaller, but at least a third of the
//! screen; the header is the title font's letter height plus ES's title
//! padding (`TITLE_VERT_PADDING` and friends in `MenuComponent.h`); a row is
//! one line of the menu font. Font sizes are the theme's, scaled from ES's
//! screen to this one.

use cosmic_text::Align;
use tiny_skia::{
    Color, GradientStop, LinearGradient, Paint, Pixmap, Point, Rect, Shader, SpreadMode, Transform,
};

use super::text::{FontRef, Fonts, TextBox};
use crate::emulator::EmuAction;
use crate::theme::{FontSpec, MenuTheme, Rgba, TextStyle};

// MenuComponent.h / Font.h, as fractions of the screen height
const TITLE_VERT_PADDING: f32 = 0.0637;
const TITLE_WITHSUB_VERT_PADDING: f32 = 0.05;
const SUBTITLE_VERT_PADDING: f32 = 0.019;
const FONT_SIZE_SMALL: f32 = 0.035;
const FONT_SIZE_MEDIUM: f32 = 0.045;
const FONT_SIZE_LARGE: f32 = 0.085;
const LINE_SPACING: f32 = 1.5;
const WIDTH_IN_LETTERS: f32 = 33.0;

/// What a menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Continue,
    Emulator(EmuAction),
    Quit,
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub action: MenuAction,
}

/// A text element of the theme at this screen's size.
#[derive(Debug, Clone)]
struct Element {
    font: FontRef,
    letter_height: f32,
    style: TextStyle,
}

impl Element {
    fn new(style: &TextStyle, scale: f32, default_size: f32) -> Self {
        let FontSpec {
            path,
            size,
            height,
            letter_height,
        } = style.font.clone();
        let size = if size > 0.0 {
            size * scale
        } else {
            default_size
        };
        let line_height = if height > 0.0 {
            height * scale
        } else {
            size * LINE_SPACING
        };
        let letter_height = if letter_height > 0.0 {
            letter_height * scale
        } else {
            size * 0.7
        };
        Self {
            font: FontRef {
                path,
                size,
                line_height,
            },
            letter_height,
            style: style.clone(),
        }
    }
}

pub struct Panel {
    pub width: u32,
    screen_height: f32,
    title: String,
    subtitle: String,
    items: Vec<MenuItem>,
    selected: usize,
    background: Rgba,
    title_element: Element,
    text: Element,
    small: Element,
}

impl Panel {
    pub fn new(
        theme: &MenuTheme,
        screen: (u32, u32),
        fonts: &mut Fonts,
        title: &str,
        subtitle: &str,
        items: Vec<MenuItem>,
    ) -> Self {
        let (width, height) = (screen.0 as f32, screen.1 as f32);
        let scale = if theme.screen.height > 0 {
            height / theme.screen.height as f32
        } else {
            1.0
        };
        let base = width.min(height);
        let title_element = Element::new(&theme.title, scale, base * FONT_SIZE_LARGE);
        let text = Element::new(&theme.text, scale, base * FONT_SIZE_MEDIUM);
        let small = Element::new(&theme.text_small, scale, base * FONT_SIZE_SMALL);

        let letters = fonts.measure(&text.font, "S") * WIDTH_IN_LETTERS;
        let panel_width = (width * 0.5).min(letters).max(width / 3.0);

        Self {
            width: panel_width.round() as u32,
            screen_height: height,
            title: title.to_uppercase(),
            subtitle: subtitle.to_owned(),
            items,
            selected: 0,
            background: theme.background.center_color,
            title_element,
            text,
            small,
        }
    }

    /// Move the selection; true when it changed.
    pub fn move_selection(&mut self, delta: i32) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let count = self.items.len() as i32;
        let next = (self.selected as i32 + delta).rem_euclid(count) as usize;
        let changed = next != self.selected;
        self.selected = next;
        changed
    }

    pub fn selected_action(&self) -> Option<MenuAction> {
        self.items.get(self.selected).map(|item| item.action)
    }

    fn header_height(&self) -> f32 {
        let h = self.screen_height;
        if self.subtitle.is_empty() {
            self.title_element.letter_height + h * TITLE_VERT_PADDING
        } else {
            self.title_element.letter_height
                + h * TITLE_WITHSUB_VERT_PADDING
                + self.small.font.line_height
                + h * SUBTITLE_VERT_PADDING
        }
    }

    /// Draw the whole panel for a surface of this height.
    pub fn render(&self, fonts: &mut Fonts, height: u32) -> Option<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, height.max(1))?;
        let width = self.width as f32;
        pixmap.fill(self.background.to_color());

        let padding = (width * 0.04).max(20.0);
        let inner = width - 2.0 * padding;
        let header = self.header_height();
        let line = (self.screen_height / 540.0).max(1.0).round();

        // header: game name, then "developer · year"
        let title_box_height = if self.subtitle.is_empty() {
            header
        } else {
            header - self.small.font.line_height - self.screen_height * SUBTITLE_VERT_PADDING
        };
        let (title_font, title) = fonts.fit(&self.title_element.font, &self.title, inner, 0.6);
        fonts.draw(
            &mut pixmap,
            &title_font,
            &title,
            TextBox {
                x: padding,
                y: 0.0,
                width: inner,
                height: title_box_height,
                align: Align::Center,
            },
            self.title_element.style.color,
        );
        if !self.subtitle.is_empty() {
            fonts.draw(
                &mut pixmap,
                &self.small.font,
                &self.subtitle,
                TextBox {
                    x: padding,
                    y: title_box_height,
                    width: inner,
                    height: self.small.font.line_height,
                    align: Align::Center,
                },
                self.small.style.color,
            );
        }

        // rows
        let row_height = self.text.font.line_height;
        let style = &self.text.style;
        let mut y = header;
        fill(
            &mut pixmap,
            0.0,
            y - line,
            width,
            line,
            style.separator_color,
        );
        for (index, item) in self.items.iter().enumerate() {
            let selected = index == self.selected;
            if selected {
                fill_selector(&mut pixmap, y, width, row_height, style);
            }
            fonts.draw(
                &mut pixmap,
                &self.text.font,
                &item.label.to_uppercase(),
                TextBox {
                    x: padding,
                    y,
                    width: inner,
                    height: row_height,
                    align: Align::Left,
                },
                if selected {
                    style.selected_color
                } else {
                    style.color
                },
            );
            y += row_height;
            fill(&mut pixmap, 0.0, y, width, line, style.separator_color);
            y += line;
        }
        Some(pixmap)
    }
}

fn fill(pixmap: &mut Pixmap, x: f32, y: f32, width: f32, height: f32, color: Rgba) {
    if let Some(rect) = Rect::from_xywh(x, y, width, height) {
        let mut paint = Paint::default();
        paint.set_color(color.to_color());
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// The selector bar: the theme's selector colour, or its gradient.
fn fill_selector(pixmap: &mut Pixmap, y: f32, width: f32, height: f32, style: &TextStyle) {
    let Some(rect) = Rect::from_xywh(0.0, y, width, height) else {
        return;
    };
    let start: Color = style.selector_color.to_color();
    let end: Color = style.selector_color_end.to_color();
    let mut paint = Paint::default();
    if start == end {
        paint.set_color(start);
    } else {
        let (from, to) = if style.selector_gradient_type == "horizontal" {
            (Point::from_xy(0.0, y), Point::from_xy(width, y))
        } else {
            (Point::from_xy(0.0, y), Point::from_xy(0.0, y + height))
        };
        match LinearGradient::new(
            from,
            to,
            vec![GradientStop::new(0.0, start), GradientStop::new(1.0, end)],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            Some(shader) => paint.shader = shader,
            None => paint.shader = Shader::SolidColor(start),
        }
    }
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}
