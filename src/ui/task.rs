//! The task screen's drawing: a small rounded window in the menu's colours
//! over Retrobox's background, like EmulationStation's dialogs: the question
//! (in capitals), the installer's output in a box when there is any, the
//! buttons at the bottom, and the help prompts below the window.
//!
//! The question is written as the menu's entries are (the theme's text, at
//! their size), and the rest of the window is sized from it, so a dialog
//! looks like the menu it comes from. The question alone makes a compact
//! window; once the installer runs, the window grows to hold its output. What does not change (the background) is
//! drawn once; a frame copies it and draws the rest. The same geometry
//! answers where a click lands, so drawing and hit testing cannot disagree.

use std::path::Path;

use cosmic_text::Align;
use tiny_skia::{FilterQuality, Paint, Pixmap, PixmapPaint, Rect as SkiaRect, Stroke, Transform};

use super::help::{Help, Prompt};
use super::panel::{Element, FONT_SIZE_MEDIUM, HOVER_ALPHA, rounded_rect, selector_paint};
use super::text::{FontRef, Fonts, TextBox};
use crate::task::terminal::{Position, Row, Terminal};
use crate::theme::{MenuTheme, Rgba};

/// The window with the installer's output: its left edge, top and width,
/// relative to the screen.
const FULL_WINDOW: (f32, f32, f32) = (0.08, 0.06, 0.84);
/// The window of the question alone is as wide as the question needs: at
/// least the first of these (relative to the question's letters), at most
/// the second (relative to the screen's width); a longer question goes on
/// more lines.
const COMPACT_WIDTH: (f32, f32) = (10.5, 0.5);
/// The most lines the question takes.
const MAX_MESSAGE_LINES: usize = 4;
/// Relative to the question's letters (the menu's entries' size): the
/// windows' corners, a button's least width and the gap between buttons,
/// the line's width (separators, the pointed button's outline), the space
/// inside the card and the buttons' letters.
const RADIUS: f32 = 0.24;
const BUTTON_MIN_WIDTH: f32 = 3.0;
const BUTTON_GAP: f32 = 0.28;
const LINE_WIDTH: f32 = 0.032;
const PADDING: f32 = 0.6;
const BUTTON_SIZE: f32 = 0.84;
/// The output's letters, relative to the screen's smaller side.
const TERMINAL_SIZE: f32 = 0.02;
/// A line's height, relative to its font's size.
const LINE_SPACING: f32 = 1.35;
/// The least letters per row of the output box: the font shrinks to fit
/// them, as a terminal's own default.
const MIN_COLUMNS: usize = 100;
/// What the output box shows: white on black, whatever the theme.
const TERMINAL_BACKGROUND: Rgba = Rgba(0x0000_00FF);
const TERMINAL_TEXT: Rgba = Rgba(0xFFFF_FFFF);
const TERMINAL_SELECTION: Rgba = Rgba(0x4D78_CCFF);
/// The space over the first row of the output box, relative to a row.
const TERMINAL_TOP: f32 = 0.25;
/// How opaque the window is.
const WINDOW_OPACITY: u8 = 0xF4;
/// When there is no picture.
const PLAIN_BACKGROUND: Rgba = Rgba(0x2020_20FF);
/// The selection's opacity.
const SELECTION_OPACITY: u8 = 0x90;

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
    pub buttons: &'a [String],
    /// The button chosen with a controller or the keyboard.
    pub focus: Option<usize>,
    /// The button under the pointer.
    pub hover: Option<usize>,
    /// The output box, if it shows: the text and how many rows it is
    /// scrolled up from the end.
    pub terminal: Option<(&'a Terminal, usize)>,
    /// Whether the cursor shows (while the command runs, when it blinks on).
    pub cursor: bool,
    pub prompts: &'a [Prompt],
}

/// Where the parts of a window go.
#[derive(Debug, Clone, Copy)]
struct Layout {
    window: Rect,
    message: Rect,
    /// The height of the line under the message.
    line: f32,
    buttons_row: Rect,
    terminal: Rect,
}

impl Layout {
    /// The parts of a `window`, the question at the top, the buttons at the
    /// bottom and, between the two, the room for the output box.
    fn new(window: Rect, padding: f32, message_height: f32, button_height: f32) -> Self {
        let inner_x = window.x + padding;
        let inner_width = window.width - padding * 2.0;
        let message = Rect {
            x: inner_x,
            y: window.y + padding,
            width: inner_width,
            height: message_height,
        };
        let line = message.y + message.height + padding * 0.5;
        let buttons_row = Rect {
            x: inner_x,
            y: window.y + window.height - padding - button_height,
            width: inner_width,
            height: button_height,
        };
        let top = line + padding * 0.5;
        let terminal = Rect {
            x: inner_x,
            y: top,
            width: inner_width,
            height: (buttons_row.y - padding * 0.5 - top).max(0.0),
        };
        Self {
            window,
            message,
            line,
            buttons_row,
            terminal,
        }
    }
}

/// The screen's geometry and what is drawn once.
pub struct TaskUi {
    size: (u32, u32),
    base: Pixmap,
    help: Help,
    message_font: FontRef,
    button_font: FontRef,
    terminal_font: FontRef,
    /// The question's letters: what the rest is sized from.
    unit: f32,
    padding: f32,
    button_height: f32,
    /// How far down the windows may reach (the help is below).
    room: f32,
    /// The window with the installer's output.
    full_window: Rect,
    /// Where the parts of the window being shown go, and the question's lines.
    layout: Layout,
    message_lines: Vec<String>,
    /// The output box: taller while there are no buttons under it.
    terminal: Rect,
    /// The room under the output box that buttons use.
    buttons_band: f32,
    cell: (f32, f32),
    line_width: f32,
    radius: f32,
    window_color: Rgba,
    text: crate::theme::TextStyle,
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

/// A rectangle with rounded corners.
fn fill_rounded(pixmap: &mut Pixmap, area: Rect, radius: f32, color: Rgba) {
    if let Some(path) = rounded_rect(area.x, area.y, area.width, area.height, radius) {
        let mut paint = Paint::default();
        paint.set_color(color.to_color());
        paint.anti_alias = true;
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// The outline of a rectangle with rounded corners.
fn outline(pixmap: &mut Pixmap, area: Rect, radius: f32, width: f32, color: Rgba) {
    if let Some(path) = rounded_rect(area.x, area.y, area.width, area.height, radius) {
        let mut paint = Paint::default();
        paint.set_color(color.to_color());
        paint.anti_alias = true;
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
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

/// The labels in capitals, as everything in EmulationStation's menus.
fn capitals(labels: &[String]) -> Vec<String> {
    labels.iter().map(|label| label.to_uppercase()).collect()
}

/// The selector (the theme's colour or its gradient) over a rounded rectangle.
fn fill_selector(pixmap: &mut Pixmap, area: Rect, radius: f32, style: &crate::theme::TextStyle) {
    if let Some(path) = rounded_rect(area.x, area.y, area.width, area.height, radius) {
        let mut paint = selector_paint(style, (area.x, area.y, area.width, area.height));
        paint.anti_alias = true;
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

impl TaskUi {
    pub fn new(
        theme: &MenuTheme,
        fonts: &mut Fonts,
        size: (u32, u32),
        background: Option<&Path>,
    ) -> Option<Self> {
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
        // the menu's entries
        let scale = if theme.screen.height > 0 {
            height / theme.screen.height as f32
        } else {
            1.0
        };
        let message_font = Element::new(&theme.text, scale, smaller * FONT_SIZE_MEDIUM).font;
        let unit = message_font.size;
        let padding = (unit * PADDING).round();
        let button_font = font(&message_font.path, unit * BUTTON_SIZE);
        let help = Help::new(theme, size);
        // the help goes where the theme puts it: the windows stay above it
        let room = (help.place(1).1 as f32 - padding * 0.6).min(height * 0.96);
        let message_height = message_font.line_height;
        let button_height = (button_font.line_height * 1.3).round();
        let top = height * FULL_WINDOW.1;
        let full = Layout::new(
            Rect {
                x: width * FULL_WINDOW.0,
                y: top,
                width: width * FULL_WINDOW.2,
                height: (room - top).max(padding * 8.0),
            },
            padding,
            message_height,
            button_height,
        );
        // a fixed-width font small enough for the least columns: they are
        // then exact, and so is where a click lands
        let advance = fonts.measure(&FontRef::monospace(100.0, 100.0), "M") / 100.0;
        let advance = if advance > 0.0 { advance } else { 0.6 };
        let fits = full.terminal.width / ((MIN_COLUMNS + 2) as f32 * advance);
        let terminal_size = (smaller * TERMINAL_SIZE).min(fits).floor().max(1.0);
        let terminal_font =
            FontRef::monospace(terminal_size, (terminal_size * LINE_SPACING).round());
        let cell = (
            (fonts.measure(&terminal_font, "MMMMMMMMMM") / 10.0).max(1.0),
            terminal_font.line_height,
        );
        Some(Self {
            size,
            base,
            help,
            unit,
            message_font,
            button_font,
            terminal_font,
            padding,
            button_height,
            room,
            full_window: full.window,
            layout: full,
            message_lines: Vec::new(),
            terminal: full.terminal,
            buttons_band: padding * 0.5 + button_height,
            cell,
            line_width: (unit * LINE_WIDTH).max(1.0),
            radius: unit * RADIUS,
            window_color: theme.background.center_color.with_alpha(WINDOW_OPACITY),
            text: theme.text.clone(),
            buttons: Vec::new(),
        })
    }

    /// The letters per row of the output box (and of the pseudo-terminal).
    pub fn columns(&self) -> usize {
        (((self.terminal.width - self.inset() * 2.0) / self.cell.0).floor() as usize)
            .max(MIN_COLUMNS)
    }

    /// The space over the first row and under the last one.
    fn top_gap(&self) -> f32 {
        self.cell.1 * TERMINAL_TOP
    }

    /// The space between the box's edge and its text.
    fn inset(&self) -> f32 {
        self.cell.0
    }

    /// Place what a frame shows: the question's lines (the window of the
    /// question alone is as wide as it needs, up to half the screen, and
    /// more lines when that is not enough) and the output box (it takes the
    /// buttons' room when there are none). Call it before anything asks for
    /// rows, and before `render`.
    pub fn arrange(
        &mut self,
        fonts: &mut Fonts,
        message: &str,
        buttons: &[String],
        with_terminal: bool,
    ) {
        let message = message.to_uppercase();
        let screen = (self.size.0 as f32, self.size.1 as f32);
        let window_width = if with_terminal {
            self.full_window.width
        } else {
            let labels = capitals(buttons);
            let least = (self.buttons_width(fonts, &labels) + self.padding * 2.0)
                .max(self.unit * COMPACT_WIDTH.0);
            let wanted = fonts.measure(&self.message_font, &message) + self.padding * 2.0;
            wanted.max(least).min(screen.0 * COMPACT_WIDTH.1)
        };
        self.message_lines = fonts.wrap(
            &self.message_font,
            &message,
            window_width - self.padding * 2.0,
            MAX_MESSAGE_LINES,
        );
        let message_height = self.message_font.line_height * self.message_lines.len() as f32;
        let window = if with_terminal {
            self.full_window
        } else {
            let height = self.padding * 3.4 + message_height + self.button_height;
            Rect {
                x: (screen.0 - window_width) / 2.0,
                y: ((self.room - height) / 2.0).max(0.0),
                width: window_width,
                height,
            }
        };
        self.layout = Layout::new(window, self.padding, message_height, self.button_height);
        self.terminal = self.layout.terminal;
        if buttons.is_empty() {
            self.terminal.height += self.buttons_band;
        }
    }

    /// The width of every button: as wide as the widest label, and not
    /// narrower than a least; and how wide they all are together.
    fn button_width(&self, fonts: &mut Fonts, labels: &[String]) -> f32 {
        let padding = self.button_font.size;
        labels
            .iter()
            .map(|label| fonts.measure(&self.button_font, label) + padding * 2.0)
            .fold(self.unit * BUTTON_MIN_WIDTH, f32::max)
    }

    fn button_gap(&self) -> f32 {
        self.unit * BUTTON_GAP
    }

    /// How wide the row of buttons is.
    fn buttons_width(&self, fonts: &mut Fonts, labels: &[String]) -> f32 {
        let count = labels.len() as f32;
        self.button_width(fonts, labels) * count + self.button_gap() * (count - 1.0).max(0.0)
    }

    /// The rows the output box shows.
    pub fn visible_rows(&self) -> usize {
        (((self.terminal.height - self.top_gap() * 2.0) / self.cell.1).floor() as usize).max(1)
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
        let index = ((y - self.terminal.y - self.top_gap()) / self.cell.1)
            .floor()
            .max(0.0) as usize;
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

    /// Draw a frame (after `arrange`).
    pub fn render(&mut self, fonts: &mut Fonts, frame: &Frame) -> Pixmap {
        let mut pixmap = self.base.clone();
        let layout = self.layout;
        fill_rounded(&mut pixmap, layout.window, self.radius, self.window_color);
        for (index, line) in self.message_lines.iter().enumerate() {
            let (font, line) = fonts.fit(&self.message_font, line, layout.message.width, 0.6);
            fonts.draw(
                &mut pixmap,
                &font,
                &line,
                TextBox {
                    x: layout.message.x,
                    y: layout.message.y + index as f32 * self.message_font.line_height,
                    width: layout.message.width,
                    height: self.message_font.line_height,
                    align: Align::Center,
                },
                self.text.color,
            );
        }
        fill(
            &mut pixmap,
            Rect {
                x: layout.window.x,
                y: layout.line,
                width: layout.window.width,
                height: self.line_width,
            },
            self.text.separator_color,
        );
        self.draw_buttons(&mut pixmap, fonts, frame, layout.buttons_row);
        if let Some((terminal, scroll)) = frame.terminal {
            self.draw_terminal(&mut pixmap, fonts, terminal, scroll, frame.cursor);
        }
        self.draw_help(&mut pixmap, fonts, frame.prompts);
        pixmap
    }

    fn draw_buttons(&mut self, pixmap: &mut Pixmap, fonts: &mut Fonts, frame: &Frame, row: Rect) {
        let labels = capitals(frame.buttons);
        let width = self.button_width(fonts, &labels);
        let gap = self.button_gap();
        let total = self.buttons_width(fonts, &labels);
        let mut x = row.x + (row.width - total) / 2.0;
        self.buttons.clear();
        for (index, label) in labels.iter().enumerate() {
            let rect = Rect {
                x,
                y: row.y,
                width,
                height: row.height,
            };
            // the colours of the rest of the overlay: the chosen one (controller
            // or keyboard) has the selector and the selected text colour; the
            // one under the pointer, the faint tint; the others a border, as ES
            let focused = frame.focus == Some(index);
            let corner = self.radius * 0.6;
            if focused {
                fill_selector(pixmap, rect, corner, &self.text);
            } else {
                if frame.hover == Some(index) {
                    fill_rounded(
                        pixmap,
                        rect,
                        corner,
                        self.text.selector_color.with_alpha(HOVER_ALPHA),
                    );
                }
                let border = self.line_width * 1.5;
                outline(
                    pixmap,
                    Rect {
                        x: rect.x + border / 2.0,
                        y: rect.y + border / 2.0,
                        width: rect.width - border,
                        height: rect.height - border,
                    },
                    corner,
                    border,
                    self.text.color,
                );
            }
            fonts.draw(
                pixmap,
                &self.button_font,
                label,
                TextBox {
                    x: rect.x,
                    y: rect.y + (rect.height - self.button_font.line_height) / 2.0,
                    width: rect.width,
                    height: self.button_font.line_height,
                    align: Align::Center,
                },
                if focused {
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
        cursor: bool,
    ) {
        let area = self.terminal;
        fill_rounded(pixmap, area, self.radius * 0.6, TERMINAL_BACKGROUND);
        let (rows, _) = self.rows(terminal, scroll);
        let selection = terminal.selection();
        for (index, row) in rows.iter().enumerate() {
            let top = area.y + self.top_gap() + index as f32 * self.cell.1;
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
                    TERMINAL_SELECTION.with_alpha(SELECTION_OPACITY),
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
                TERMINAL_TEXT,
            );
            if cursor
                && let Some(column) = cursor_in(row, terminal, rows.get(index + 1), self.columns())
            {
                self.draw_cursor(pixmap, fonts, terminal, row, column, top);
            }
        }
    }

    /// A block over the cursor's cell, with the letter under it (if any) in
    /// the box's colour, as terminals draw it.
    fn draw_cursor(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        terminal: &Terminal,
        row: &Row,
        column: usize,
        top: f32,
    ) {
        let x = self.terminal.x + self.inset() + (column - row.start) as f32 * self.cell.0;
        let cell = Rect {
            x,
            y: top,
            width: self.cell.0,
            height: self.cell.1,
        };
        fill(pixmap, cell, TERMINAL_TEXT);
        if let Some(letter) = terminal.line(row.line).get(column) {
            fonts.draw(
                pixmap,
                &self.terminal_font,
                &letter.to_string(),
                TextBox {
                    x,
                    y: top,
                    width: self.cell.0 * 2.0,
                    height: self.cell.1,
                    align: Align::Left,
                },
                TERMINAL_BACKGROUND,
            );
        }
    }

    fn draw_help(&mut self, pixmap: &mut Pixmap, fonts: &mut Fonts, prompts: &[Prompt]) {
        let max = self.size.0 as f32 * FULL_WINDOW.2;
        if let Some(row) = self.help.render(fonts, prompts, max, None) {
            let (x, y) = self.help.place(row.width());
            pixmap.draw_pixmap(
                x,
                y,
                row.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
    }
}

/// The cursor's column, when it is in `row`: the row of the line being
/// written that holds its column, or that line's last row shown when the
/// cursor is past its end (at most on the row's last cell, `columns`).
fn cursor_in(row: &Row, terminal: &Terminal, next: Option<&Row>, columns: usize) -> Option<usize> {
    if row.line != terminal.cursor_line() {
        return None;
    }
    let column = terminal.cursor_column();
    let last_of_line = next.is_none_or(|next| next.line != row.line);
    if (row.start..row.end).contains(&column) {
        Some(column)
    } else if last_of_line && column >= row.start {
        Some(column.min(row.start + columns.max(1) - 1))
    } else {
        None
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

    #[test]
    fn the_cursor_is_on_the_row_of_its_column() {
        let mut terminal = Terminal::new();
        terminal.feed("done\r\nabcdefgh");
        let line = terminal.cursor_line();
        let rows = terminal.rows(4, 0, 3, terminal.row_count(4));
        // "abcd" then "efgh": the cursor, past the end of a full row, on its last cell
        assert_eq!(cursor_in(&rows[1], &terminal, rows.get(2), 4), None);
        assert_eq!(cursor_in(&rows[2], &terminal, None, 4), Some(7));
        assert_eq!(cursor_in(&rows[0], &terminal, rows.get(1), 4), None);
        terminal.feed("\r");
        assert_eq!(cursor_in(&rows[1], &terminal, rows.get(2), 4), Some(0));
        assert_eq!(rows[1].line, line);
    }

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
    fn the_parts_of_a_window_stay_inside_it() {
        let window = Rect {
            x: 100.0,
            y: 50.0,
            width: 800.0,
            height: 500.0,
        };
        let layout = Layout::new(window, 20.0, 40.0, 50.0);
        let inside = |part: Rect| {
            part.x >= window.x
                && part.y >= window.y
                && part.x + part.width <= window.x + window.width
                && part.y + part.height <= window.y + window.height
        };
        assert!(inside(layout.message));
        assert!(inside(layout.buttons_row));
        assert!(inside(layout.terminal));
        // the output box lies between the line under the message and the buttons
        assert!(layout.terminal.y > layout.line);
        assert!(layout.terminal.y + layout.terminal.height < layout.buttons_row.y);
    }

    #[test]
    fn a_window_too_small_for_its_parts_has_an_empty_output_box() {
        let window = Rect {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 100.0,
        };
        let layout = Layout::new(window, 20.0, 40.0, 50.0);
        assert!(layout.terminal.height.abs() < f32::EPSILON);
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
