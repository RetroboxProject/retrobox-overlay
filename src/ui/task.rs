//! The task screen's drawing: a card in the menu's colours over Retrobox's
//! background, with a message, buttons, the installer's output in a box and
//! the help prompts.
//!
//! What does not change (the background and the card) is drawn once; a frame
//! copies it and draws the rest. The same geometry answers where a click
//! lands, so drawing and hit testing cannot disagree.

use std::path::Path;

use cosmic_text::Align;
use tiny_skia::{FilterQuality, Paint, Pixmap, PixmapPaint, Rect as SkiaRect, Transform};

use super::help::{Help, Prompt};
use super::text::{FontRef, Fonts, TextBox};
use crate::task::terminal::{Position, Row, Terminal};
use crate::theme::{MenuTheme, Rgba};

/// The card, relative to the screen.
const CARD: (f32, f32, f32, f32) = (0.1, 0.07, 0.8, 0.82);
/// The space inside the card, relative to the screen's smaller side.
const PADDING: f32 = 0.03;
/// Font sizes, relative to the screen's smaller side.
const MESSAGE_SIZE: f32 = 0.05;
const BUTTON_SIZE: f32 = 0.042;
const TERMINAL_SIZE: f32 = 0.027;
/// A line's height, relative to its font's size.
const LINE_SPACING: f32 = 1.35;
/// The width of an average letter, relative to the font's size (the
/// terminal wraps by columns of this width, the font being proportional).
const LETTER_WIDTH: f32 = 0.5;
/// What shows of the background behind the card, as an opacity.
const CARD_OPACITY: u8 = 0xF0;
/// When there is no picture.
const PLAIN_BACKGROUND: Rgba = Rgba(0x2020_20FF);
/// The terminal box's tint, and the selection's.
const BOX_OPACITY: u8 = 0x1c;
const SELECTION_OPACITY: u8 = 0x80;

/// A rectangle on the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// What a frame shows.
pub struct Frame<'a> {
    pub message: &'a str,
    pub buttons: &'a [String],
    /// The button chosen with a controller or the keyboard.
    pub focus: Option<usize>,
    /// The button under the pointer.
    pub hover: Option<usize>,
    /// The output box, if it shows: the text and how many rows it is
    /// scrolled up from the end.
    pub terminal: Option<(&'a Terminal, usize)>,
    pub prompts: &'a [Prompt],
}

/// The screen's geometry and what is drawn once.
pub struct TaskUi {
    size: (u32, u32),
    base: Pixmap,
    help: Help,
    message_font: FontRef,
    button_font: FontRef,
    terminal_font: FontRef,
    message: Rect,
    buttons_row: Rect,
    terminal: Rect,
    help_top: f32,
    cell: (f32, f32),
    text: crate::theme::TextStyle,
    title_color: Rgba,
    /// The buttons as last drawn.
    buttons: Vec<Rect>,
}

fn font(path: &Path, size: f32) -> FontRef {
    FontRef {
        path: path.to_owned(),
        size,
        line_height: (size * LINE_SPACING).round(),
        bold: false,
    }
}

fn fill(pixmap: &mut Pixmap, area: Rect, color: Rgba) {
    if let Some(rect) = SkiaRect::from_xywh(area.x, area.y, area.width, area.height) {
        let mut paint = Paint::default();
        paint.set_color(color.to_color());
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// The picture, covering the whole screen (cropped, not stretched).
fn cover(pixmap: &mut Pixmap, picture: &Pixmap) {
    let scale = (pixmap.width() as f32 / picture.width() as f32)
        .max(pixmap.height() as f32 / picture.height() as f32);
    let left = (pixmap.width() as f32 - picture.width() as f32 * scale) / 2.0;
    let top = (pixmap.height() as f32 - picture.height() as f32 * scale) / 2.0;
    pixmap.draw_pixmap(
        0,
        0,
        picture.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        },
        Transform::from_row(scale, 0.0, 0.0, scale, left, top),
        None,
    );
}

impl TaskUi {
    pub fn new(theme: &MenuTheme, size: (u32, u32), background: Option<&Path>) -> Option<Self> {
        let (width, height) = (size.0 as f32, size.1 as f32);
        let smaller = width.min(height);
        let mut base = Pixmap::new(size.0, size.1)?;
        base.fill(PLAIN_BACKGROUND.to_color());
        if let Some(path) = background {
            match Pixmap::load_png(path) {
                Ok(picture) => cover(&mut base, &picture),
                Err(err) => log::warn!("cannot read the background {}: {err}", path.display()),
            }
        }
        let card = Rect {
            x: width * CARD.0,
            y: height * CARD.1,
            width: width * CARD.2,
            height: height * CARD.3,
        };
        fill(
            &mut base,
            card,
            theme.background.center_color.with_alpha(CARD_OPACITY),
        );

        let padding = smaller * PADDING;
        let message_font = font(&theme.title.font.path, smaller * MESSAGE_SIZE).bold();
        let button_font = font(&theme.text.font.path, smaller * BUTTON_SIZE);
        let terminal_font = font(&theme.text_small.font.path, smaller * TERMINAL_SIZE);
        let inner = Rect {
            x: card.x + padding,
            y: card.y + padding,
            width: card.width - padding * 2.0,
            height: card.height - padding * 2.0,
        };
        let message = Rect {
            height: message_font.line_height * 2.0,
            ..inner
        };
        let buttons_row = Rect {
            y: message.y + message.height + padding * 0.5,
            height: (button_font.line_height * 1.5).round(),
            ..inner
        };
        let help = Help::new(theme, size);
        let help_height = help.height() as f32;
        let help_top = card.y + card.height - padding - help_height;
        let box_top = buttons_row.y + buttons_row.height + padding * 0.5;
        let terminal = Rect {
            y: box_top,
            height: (help_top - padding * 0.5 - box_top).max(terminal_font.line_height),
            ..inner
        };
        let cell = (
            (terminal_font.size * LETTER_WIDTH).max(1.0),
            terminal_font.line_height,
        );
        Some(Self {
            size,
            base,
            help,
            message_font,
            button_font,
            terminal_font,
            message,
            buttons_row,
            terminal,
            help_top,
            cell,
            text: theme.text.clone(),
            title_color: theme.title.color,
            buttons: Vec::new(),
        })
    }

    /// The letters per row of the output box (and of the pseudo-terminal).
    pub fn columns(&self) -> usize {
        (((self.terminal.width - self.inset() * 2.0) / self.cell.0).floor() as usize).max(20)
    }

    /// The space between the box's edge and its text.
    fn inset(&self) -> f32 {
        self.cell.0
    }

    /// The rows the output box shows.
    pub fn visible_rows(&self) -> usize {
        ((self.terminal.height / self.cell.1).floor() as usize).max(1)
    }

    /// The help prompts' icon names of confirm and back.
    pub fn help_icons(&self) -> (String, String) {
        (self.help.ok().to_owned(), self.help.back().to_owned())
    }

    /// The rows of the output box for a scroll position (rows above the
    /// end), and how far it can scroll.
    pub fn rows(&self, terminal: &Terminal, scroll: usize) -> (Vec<Row>, usize) {
        let columns = self.columns();
        let visible = self.visible_rows();
        let total = terminal.row_count(columns);
        let furthest = total.saturating_sub(visible);
        let scroll = scroll.min(furthest);
        let first = total.saturating_sub(visible + scroll);
        (terminal.rows(columns, first, visible, total), furthest)
    }

    /// The button under a point.
    pub fn button_at(&self, point: (f32, f32)) -> Option<usize> {
        self.buttons.iter().position(|rect| rect.contains(point))
    }

    /// Whether a point is in the output box.
    pub fn in_terminal(&self, point: (f32, f32)) -> bool {
        self.terminal.contains(point)
    }

    /// The place in the text under a point of the output box (the nearest
    /// one when it is past the ends of its row).
    pub fn position_at(
        &self,
        fonts: &mut Fonts,
        terminal: &Terminal,
        scroll: usize,
        (x, y): (f32, f32),
    ) -> Option<Position> {
        let (rows, _) = self.rows(terminal, scroll);
        let index = ((y - self.terminal.y) / self.cell.1).floor().max(0.0) as usize;
        let row = rows.get(index.min(rows.len().saturating_sub(1)))?;
        let text = &terminal.line(row.line)[row.start..row.end];
        let offset = x - self.terminal.x - self.inset();
        let mut column = 0;
        let mut previous = 0.0;
        for end in 1..=text.len() {
            let prefix: String = text[..end].iter().collect();
            let reach = fonts.measure(&self.terminal_font, &prefix);
            if offset < f32::midpoint(previous, reach) {
                break;
            }
            previous = reach;
            column = end;
        }
        Some(Position {
            line: row.line,
            column: row.start + column,
        })
    }

    /// Draw a frame.
    pub fn render(&mut self, fonts: &mut Fonts, frame: &Frame) -> Pixmap {
        let mut pixmap = self.base.clone();
        let (message_font, message) =
            fonts.fit(&self.message_font, frame.message, self.message.width, 0.6);
        fonts.draw(
            &mut pixmap,
            &message_font,
            &message,
            TextBox {
                x: self.message.x,
                y: self.message.y,
                width: self.message.width,
                height: self.message.height,
                align: Align::Center,
            },
            self.title_color,
        );
        self.draw_buttons(&mut pixmap, fonts, frame);
        if let Some((terminal, scroll)) = frame.terminal {
            self.draw_terminal(&mut pixmap, fonts, terminal, scroll);
        }
        self.draw_help(&mut pixmap, fonts, frame.prompts);
        pixmap
    }

    fn draw_buttons(&mut self, pixmap: &mut Pixmap, fonts: &mut Fonts, frame: &Frame) {
        let row = self.buttons_row;
        let padding = self.button_font.size;
        let widths: Vec<f32> = frame
            .buttons
            .iter()
            .map(|label| fonts.measure(&self.button_font, label) + padding * 2.0)
            .collect();
        let gap = padding;
        let total = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
        let mut x = row.x + (row.width - total) / 2.0;
        self.buttons.clear();
        for (index, (label, width)) in frame.buttons.iter().zip(&widths).enumerate() {
            let rect = Rect {
                x,
                y: row.y,
                width: *width,
                height: row.height,
            };
            let chosen = frame.focus == Some(index) || frame.hover == Some(index);
            if chosen {
                fill(pixmap, rect, self.text.selector_color);
            }
            fonts.draw(
                pixmap,
                &self.button_font,
                label,
                TextBox {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    align: Align::Center,
                },
                if chosen {
                    self.text.selected_color
                } else {
                    self.text.color
                },
            );
            self.buttons.push(rect);
            x += width + gap;
        }
    }

    fn draw_terminal(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        terminal: &Terminal,
        scroll: usize,
    ) {
        let area = self.terminal;
        fill(pixmap, area, self.text.color.with_alpha(BOX_OPACITY));
        let (rows, _) = self.rows(terminal, scroll);
        let selection = terminal.selection();
        for (index, row) in rows.iter().enumerate() {
            let top = area.y + index as f32 * self.cell.1;
            let text: String = terminal.line(row.line)[row.start..row.end].iter().collect();
            if let Some((start, end)) = selection
                && let Some((from, to)) = selected_span(row, start, end)
            {
                let chars = |count: usize| -> String {
                    terminal.line(row.line)[row.start..row.start + count]
                        .iter()
                        .collect()
                };
                let left = fonts.measure(&self.terminal_font, &chars(from));
                let right = fonts.measure(&self.terminal_font, &chars(to));
                fill(
                    pixmap,
                    Rect {
                        x: area.x + self.inset() + left,
                        y: top,
                        width: (right - left).max(self.cell.0 / 2.0),
                        height: self.cell.1,
                    },
                    self.text.selector_color.with_alpha(SELECTION_OPACITY),
                );
            }
            fonts.draw(
                pixmap,
                &self.terminal_font,
                &text,
                TextBox {
                    x: area.x + self.inset(),
                    y: top,
                    width: area.width - self.inset(),
                    height: self.cell.1,
                    align: Align::Left,
                },
                self.text.color,
            );
        }
    }

    fn draw_help(&mut self, pixmap: &mut Pixmap, fonts: &mut Fonts, prompts: &[Prompt]) {
        let max = self.size.0 as f32 * CARD.2;
        if let Some(row) = self.help.render(fonts, prompts, max, None) {
            let x = (self.size.0 as f32 * CARD.0 + (max - row.width() as f32) / 2.0).round() as i32;
            pixmap.draw_pixmap(
                x,
                self.help_top.round() as i32,
                row.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
    }
}

/// The part of a row (as characters from its start) a selection covers.
fn selected_span(row: &Row, start: Position, end: Position) -> Option<(usize, usize)> {
    let first = Position {
        line: row.line,
        column: row.start,
    };
    let last = Position {
        line: row.line,
        column: row.end,
    };
    if end <= first || start >= last {
        return None;
    }
    let from = if start > first {
        start.column - row.start
    } else {
        0
    };
    let to = if end < last {
        end.column - row.start
    } else {
        row.end - row.start
    };
    (to > from).then_some((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: Row = Row {
        line: 3,
        start: 10,
        end: 20,
    };

    fn at(line: usize, column: usize) -> Position {
        Position { line, column }
    }

    #[test]
    fn a_selection_covers_part_of_a_row() {
        assert_eq!(selected_span(&ROW, at(3, 12), at(3, 15)), Some((2, 5)));
        assert_eq!(selected_span(&ROW, at(2, 0), at(4, 0)), Some((0, 10)));
        assert_eq!(selected_span(&ROW, at(3, 5), at(3, 14)), Some((0, 4)));
        assert_eq!(selected_span(&ROW, at(3, 15), at(5, 0)), Some((5, 10)));
    }

    #[test]
    fn a_selection_elsewhere_covers_nothing() {
        assert_eq!(selected_span(&ROW, at(1, 0), at(2, 9)), None);
        assert_eq!(selected_span(&ROW, at(3, 20), at(4, 3)), None);
        assert_eq!(selected_span(&ROW, at(3, 4), at(3, 10)), None);
    }

    #[test]
    fn rects_contain_their_top_left_edge_only() {
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            width: 5.0,
            height: 5.0,
        };
        assert!(rect.contains((10.0, 10.0)));
        assert!(!rect.contains((15.0, 12.0)));
    }
}
