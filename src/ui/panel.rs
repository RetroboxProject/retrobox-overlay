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
    Color, FilterQuality, GradientStop, LinearGradient, Paint, Pixmap, PixmapPaint, Point, Rect,
    Shader, SpreadMode, Transform,
};

use super::icons::Icon;
use super::text::{FontRef, Fonts, TextBox};
use crate::emulator::EmuAction;
use crate::savestates::{SLOTS, StateMode};
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
/// Opacity of the mouse hover highlight: the selector colour, faint.
const HOVER_ALPHA: u8 = 0x50;

/// What a menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Continue,
    Emulator(EmuAction),
    /// Open the save or load page.
    States(StateMode),
    /// Save into, or load, a slot.
    Slot(StateMode, u32),
    /// Read the game's manual.
    Manual,
    Quit,
}

/// One save state slot on its page.
pub struct SlotView {
    pub number: u32,
    pub label: String,
    /// When it was saved, or that it is empty.
    pub detail: String,
    pub preview: Option<Pixmap>,
    /// Empty slots cannot be loaded.
    pub enabled: bool,
}

/// The save (or load) page: the slots, shown instead of the main entries.
pub struct SlotPage {
    pub mode: StateMode,
    pub title: String,
    pub slots: Vec<SlotView>,
}

/// The icons' size, relative to the height of the text's capital letters.
const ICON_SCALE: f32 = 1.35;
/// The gap between an icon and its text, relative to the panel's padding.
const ICON_GAP: f32 = 0.7;
/// Opacity of the frame drawn where a slot has no preview.
const EMPTY_PREVIEW_ALPHA: u8 = 0x30;
/// Opacity of the text of a slot that cannot be chosen.
const DISABLED_ALPHA: u8 = 0x80;

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub action: MenuAction,
    /// Entries that can do nothing right now are shown dimmed.
    pub enabled: bool,
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
    /// The entry under the mouse pointer: highlighted, not selected.
    hovered: Option<usize>,
    /// The save state page, when open; `selected`/`hovered` are then its.
    page: Option<SlotPage>,
    /// The main entry selected when the page was opened.
    main_selected: usize,
    /// The first slot shown: the page scrolls when not all of them fit.
    first_slot: usize,
    /// Whether the main entries show their icons.
    icons: bool,
    /// A line about the game's state under the header (the disc in the
    /// drive), on the main page.
    status: Option<String>,
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
            hovered: None,
            page: None,
            main_selected: 0,
            first_slot: 0,
            icons: true,
            status: None,
            background: theme.background.center_color,
            title_element,
            text,
            small,
        }
    }

    /// The line under the header (None: none).
    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    /// Change the label of an entry and whether it can be chosen.
    pub fn set_entry(&mut self, action: MenuAction, label: Option<&str>, enabled: bool) {
        for item in self.items.iter_mut().filter(|item| item.action == action) {
            if let Some(label) = label {
                label.clone_into(&mut item.label);
            }
            item.enabled = enabled;
        }
    }

    /// Show or hide the icons of the main entries.
    pub fn set_icons(&mut self, icons: bool) {
        self.icons = icons;
    }

    /// Show a save state page instead of the main entries.
    pub fn open_page(&mut self, page: SlotPage) {
        self.main_selected = self.selected;
        self.selected = page.slots.iter().position(|slot| slot.enabled).unwrap_or(0);
        self.hovered = None;
        self.page = Some(page);
        self.first_slot = 0;
        self.scroll_to_selection();
    }

    /// Whether a save state page is shown.
    pub const fn page_open(&self) -> bool {
        self.page.is_some()
    }

    /// Back to the main entries; false when no page was open.
    pub fn close_page(&mut self) -> bool {
        if self.page.take().is_none() {
            return false;
        }
        self.selected = self.main_selected;
        self.hovered = None;
        true
    }

    /// How many entries the current page has.
    fn count(&self) -> usize {
        self.page
            .as_ref()
            .map_or(self.items.len(), |page| page.slots.len())
    }

    /// Move the selection; true when it changed.
    pub fn move_selection(&mut self, delta: i32) -> bool {
        if self.count() == 0 {
            return false;
        }
        let count = self.count() as i32;
        let next = (self.selected as i32 + delta).rem_euclid(count) as usize;
        let changed = next != self.selected;
        self.selected = next;
        self.scroll_to_selection();
        changed
    }

    /// Scroll the slots so that the selected one is shown.
    fn scroll_to_selection(&mut self) {
        if self.page.is_none() {
            return;
        }
        let visible = self.visible_slots();
        if self.selected < self.first_slot {
            self.first_slot = self.selected;
        } else if self.selected >= self.first_slot + visible {
            self.first_slot = self.selected + 1 - visible;
        }
    }

    /// Highlight the entry under the pointer (None: none); true when it changed.
    pub fn hover(&mut self, index: Option<usize>) -> bool {
        let index = index.filter(|index| *index < self.count());
        let changed = index != self.hovered;
        self.hovered = index;
        changed
    }

    /// What an entry of the current page does (None: nothing, e.g. an
    /// empty slot to load).
    pub fn action(&self, index: usize) -> Option<MenuAction> {
        match &self.page {
            Some(page) => page
                .slots
                .get(index)
                .filter(|slot| slot.enabled)
                .map(|slot| MenuAction::Slot(page.mode, slot.number)),
            None => self
                .items
                .get(index)
                .filter(|item| item.enabled)
                .map(|item| item.action),
        }
    }

    /// The entry at height `y` of the panel (surface coordinates), if any.
    pub fn item_at(&self, y: f32) -> Option<usize> {
        let top = self.header_height();
        if y < top {
            return None;
        }
        let pitch = if self.page.is_some() {
            self.slot_height()
        } else {
            self.text.font.line_height + self.line()
        };
        let row = ((y - top) / pitch) as usize;
        let index = if self.page.is_some() {
            if row >= self.visible_slots() {
                return None;
            }
            self.first_slot + row
        } else {
            row
        };
        (index < self.count()).then_some(index)
    }

    /// How many slot rows fit, with previews no larger than about a third of
    /// the panel's width.
    fn visible_slots(&self) -> usize {
        let free = self.screen_height - self.header_height();
        ((free / (self.width as f32 * 0.3)).floor() as usize).clamp(1, SLOTS.len())
    }

    /// The height of a slot row: the visible rows fill the panel.
    fn slot_height(&self) -> f32 {
        (self.screen_height - self.header_height()) / self.visible_slots() as f32
    }

    /// The thickness of the separators.
    fn line(&self) -> f32 {
        (self.screen_height / 540.0).max(1.0).round()
    }

    pub fn selected_action(&self) -> Option<MenuAction> {
        self.action(self.selected)
    }

    /// The header's two lines: the game and "developer · year", or on a
    /// save state page, the page and the game.
    fn header_texts(&self) -> (&str, &str) {
        match &self.page {
            Some(page) => (&page.title, &self.title),
            None => (&self.title, &self.subtitle),
        }
    }

    /// The status line, shown on the main page only.
    fn status_line(&self) -> Option<&str> {
        self.page
            .is_none()
            .then_some(self.status.as_deref())
            .flatten()
    }

    /// The small lines under the title: the subtitle and the status line.
    fn header_lines(&self) -> usize {
        usize::from(!self.header_texts().1.is_empty()) + usize::from(self.status_line().is_some())
    }

    fn header_height(&self) -> f32 {
        let h = self.screen_height;
        match self.header_lines() {
            0 => self.title_element.letter_height + h * TITLE_VERT_PADDING,
            lines => {
                self.title_element.letter_height
                    + h * TITLE_WITHSUB_VERT_PADDING
                    + self.small.font.line_height * lines as f32
                    + h * SUBTITLE_VERT_PADDING
            }
        }
    }

    /// Draw the whole panel for a surface of this height.
    pub fn render(&self, fonts: &mut Fonts, height: u32) -> Option<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, height.max(1))?;
        let width = self.width as f32;
        pixmap.fill(self.background.to_color());

        let padding = (width * 0.04).max(20.0);
        let header = self.header_height();
        self.draw_header(&mut pixmap, fonts, padding, header);
        fill(
            &mut pixmap,
            0.0,
            header - self.line(),
            width,
            self.line(),
            self.text.style.separator_color,
        );
        match &self.page {
            Some(page) => self.draw_slots(&mut pixmap, fonts, page, padding, header),
            None => self.draw_items(&mut pixmap, fonts, padding, header),
        }
        Some(pixmap)
    }

    /// The header: its title, fitted to the width, and the smaller lines.
    fn draw_header(&self, pixmap: &mut Pixmap, fonts: &mut Fonts, padding: f32, header: f32) {
        let inner = self.width as f32 - 2.0 * padding;
        let (title, subtitle) = self.header_texts();
        let small_height = self.small.font.line_height;
        let lines = self.header_lines();
        let title_box_height = if lines == 0 {
            header
        } else {
            header - small_height * lines as f32 - self.screen_height * SUBTITLE_VERT_PADDING
        };
        let (title_font, title) = fonts.fit(&self.title_element.font, title, inner, 0.6);
        fonts.draw(
            pixmap,
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
        let mut y = title_box_height;
        if !subtitle.is_empty() {
            let (font, subtitle) = fonts.fit(&self.small.font, subtitle, inner, 0.8);
            fonts.draw(
                pixmap,
                &font,
                &subtitle,
                TextBox {
                    x: padding,
                    y,
                    width: inner,
                    height: small_height,
                    align: Align::Center,
                },
                self.small.style.color,
            );
            y += small_height;
        }
        if let Some(status) = self.status_line() {
            self.draw_status(pixmap, fonts, status, (padding, y, inner));
        }
    }

    /// The status line: the disc icon and the text, centred together.
    fn draw_status(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        status: &str,
        at: (f32, f32, f32),
    ) {
        let (padding, y, inner) = at;
        let height = self.small.font.line_height;
        let color = self.small.style.color;
        let icon_size = (self.small.letter_height * ICON_SCALE).round().max(1.0);
        let gap = icon_size * 0.4;
        let (font, status) = fonts.fit(&self.small.font, status, inner - icon_size - gap, 0.8);
        let text_width = fonts.measure(&font, &status);
        let left = padding + ((inner - icon_size - gap - text_width) / 2.0).max(0.0);
        if let Some(image) = Icon::Disc.render(icon_size as u32, color) {
            pixmap.draw_pixmap(
                left.round() as i32,
                (y + (height - icon_size) / 2.0).round() as i32,
                image.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
        fonts.draw(
            pixmap,
            &font,
            &status,
            TextBox {
                x: left + icon_size + gap,
                y,
                width: text_width + 1.0,
                height,
                align: Align::Left,
            },
            color,
        );
    }

    /// The background of a row: the selector, the hover highlight or nothing.
    fn draw_row_background(&self, pixmap: &mut Pixmap, index: usize, y: f32, height: f32) {
        let width = self.width as f32;
        let style = &self.text.style;
        if index == self.selected {
            fill_selector(pixmap, y, width, height, style);
        } else if self.hovered == Some(index) {
            fill(
                pixmap,
                0.0,
                y,
                width,
                height,
                style.selector_color.with_alpha(HOVER_ALPHA),
            );
        }
    }

    /// The main entries, one line each.
    fn draw_items(&self, pixmap: &mut Pixmap, fonts: &mut Fonts, padding: f32, header: f32) {
        let width = self.width as f32;
        let inner = width - 2.0 * padding;
        let line = self.line();
        let row_height = self.text.font.line_height;
        let style = &self.text.style;
        // icons as tall as capital letters plus a bit, then a gap
        let icon_size = (self.text.letter_height * ICON_SCALE).round().max(1.0);
        let text_x = if self.icons {
            padding + icon_size + padding * ICON_GAP
        } else {
            padding
        };
        let mut y = header;
        for (index, item) in self.items.iter().enumerate() {
            self.draw_row_background(pixmap, index, y, row_height);
            let color = if index == self.selected {
                style.selected_color
            } else {
                style.color
            };
            let color = if item.enabled {
                color
            } else {
                color.with_alpha(DISABLED_ALPHA)
            };
            if self.icons
                && let Some(icon) = Icon::for_action(item.action)
                && let Some(image) = icon.render(icon_size as u32, color)
            {
                let top = y + (row_height - icon_size) / 2.0;
                pixmap.draw_pixmap(
                    padding.round() as i32,
                    top.round() as i32,
                    image.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
            fonts.draw(
                pixmap,
                &self.text.font,
                &item.label.to_uppercase(),
                TextBox {
                    x: text_x,
                    y,
                    width: inner - (text_x - padding),
                    height: row_height,
                    align: Align::Left,
                },
                color,
            );
            y += row_height;
            fill(pixmap, 0.0, y, width, line, style.separator_color);
            y += line;
        }
    }

    /// The save state slots: preview on the left, slot and date on the right.
    fn draw_slots(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        page: &SlotPage,
        padding: f32,
        header: f32,
    ) {
        let width = self.width as f32;
        let line = self.line();
        let row = self.slot_height();
        let style = &self.text.style;
        let margin = row * 0.1;
        let preview_height = row - 2.0 * margin;
        let preview_width = preview_height * 4.0 / 3.0;
        let text_x = padding + preview_width + padding;
        let text_width = (width - text_x - padding).max(1.0);

        let visible = self.visible_slots();
        let shown = page
            .slots
            .iter()
            .enumerate()
            .skip(self.first_slot)
            .take(visible);
        for (index, slot) in shown {
            let y = header + (index - self.first_slot) as f32 * row;
            self.draw_row_background(pixmap, index, y, row - line);
            let color = if index == self.selected {
                style.selected_color
            } else {
                style.color
            };
            let color = if slot.enabled {
                color
            } else {
                color.with_alpha(DISABLED_ALPHA)
            };

            let frame = (padding, y + margin, preview_width, preview_height);
            match &slot.preview {
                Some(preview) => draw_fitted(pixmap, preview, frame),
                None => fill(
                    pixmap,
                    frame.0,
                    frame.1,
                    frame.2,
                    frame.3,
                    style.color.with_alpha(EMPTY_PREVIEW_ALPHA),
                ),
            }

            let middle = y + row / 2.0;
            let (label_font, label) =
                fonts.fit(&self.text.font, &slot.label.to_uppercase(), text_width, 0.7);
            fonts.draw(
                pixmap,
                &label_font,
                &label,
                TextBox {
                    x: text_x,
                    y: middle - self.text.font.line_height,
                    width: text_width,
                    height: self.text.font.line_height,
                    align: Align::Left,
                },
                color,
            );
            let (detail_font, detail) = fonts.fit(&self.small.font, &slot.detail, text_width, 0.7);
            fonts.draw(
                pixmap,
                &detail_font,
                &detail,
                TextBox {
                    x: text_x,
                    y: middle,
                    width: text_width,
                    height: self.small.font.line_height,
                    align: Align::Left,
                },
                color,
            );
            fill(
                pixmap,
                0.0,
                y + row - line,
                width,
                line,
                style.separator_color,
            );
        }
        // where the shown slots are among all of them
        if page.slots.len() > visible {
            let track = self.screen_height - header;
            let bar = (width * 0.006).max(3.0);
            let length = track * visible as f32 / page.slots.len() as f32;
            let top = header + track * self.first_slot as f32 / page.slots.len() as f32;
            fill(pixmap, width - bar, top, bar, length, style.selector_color);
        }
    }
}

/// Draw an image scaled to fit a frame (x, y, width, height), centred.
fn draw_fitted(pixmap: &mut Pixmap, image: &Pixmap, frame: (f32, f32, f32, f32)) {
    let (x, y, width, height) = frame;
    let scale = (width / image.width() as f32).min(height / image.height() as f32);
    let left = x + (width - image.width() as f32 * scale) / 2.0;
    let top = y + (height - image.height() as f32 * scale) / 2.0;
    let paint = PixmapPaint {
        quality: FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    pixmap.draw_pixmap(
        0,
        0,
        image.as_ref(),
        &paint,
        Transform::from_row(scale, 0.0, 0.0, scale, left, top),
        None,
    );
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
