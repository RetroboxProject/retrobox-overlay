//! The side menu, laid out and coloured like EmulationStation's game options
//! menu (`GuiGameOptions` with side menus: full height, slid in from an edge).
//!
//! Sizes follow ES: the width is half the screen or 33 times the width of an
//! "S" in the menu font, whichever is smaller, but at least a third of the
//! screen; the header is the title font's letter height plus ES's title
//! padding (`TITLE_VERT_PADDING` and friends in `MenuComponent.h`); a row is
//! one line of the menu font. Font sizes are the theme's, scaled from ES's
//! screen to this one.
//!
//! Above the header, a status line like the Switch's: the time (when the
//! clock is the small one), the network and the battery. At the bottom,
//! pinned (they do not scroll, and hide while a save state page is open):
//! round buttons (continue, screenshot, quit), each telling what it does
//! when highlighted for a while, and the sliders (the brightness, the
//! volume): their icon, the slider and its value. The entries between the
//! header and those scroll when they do not all fit (smoothly when dragged).
//! For touch screens, the rows can be taller than ES's (`row_scale`), the
//! text staying as it is.

use cosmic_text::Align;
use tiny_skia::{
    Color, FilterQuality, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, PixmapPaint,
    Point, Rect, Shader, SpreadMode, Transform,
};

use super::icons::Icon;
use super::text::{FontRef, Fonts, TextBox};
use crate::emulator::EmuAction;
use crate::savestates::{SLOTS, StateMode};
use crate::system::SliderKind;
use crate::system::status::Status;
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
    /// Put the computer to sleep.
    Standby,
    /// A slider: Left/Right (or the mouse) change it.
    Slider(SliderKind),
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
/// The entries' and sliders' icons are `ICON_SCALE` at this `row_scale`,
/// and grow or shrink with it.
const ICON_ROW_SCALE: f32 = 1.25;
/// The status line (time, network, battery) next to the small text: larger.
const STATUS_SCALE: f32 = 1.5;
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
    /// A slider's value, 0 to 1.
    pub value: Option<f32>,
    /// The volume's slider: muted (its value kept).
    pub muted: bool,
}

impl MenuItem {
    pub const fn new(label: String, action: MenuAction) -> Self {
        Self {
            label,
            action,
            enabled: true,
            value: None,
            muted: false,
        }
    }
}

/// The status line above the header.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusBar {
    /// The time, when the clock is the small one.
    pub time: Option<String>,
    pub status: Status,
    /// Whether the battery's percentage shows next to its icon.
    pub battery_percent: bool,
}

impl StatusBar {
    fn is_empty(&self) -> bool {
        self.time.is_none() && self.status.network.is_none() && self.status.battery.is_none()
    }
}

/// Where a slider row's parts go, as fractions of the panel's inner width:
/// the icon first, the track up to `TRACK_END`, the value after
/// `VALUE_START`. Sliders have no label: their icon says what they are.
const TRACK_END: f32 = 0.84;
const VALUE_START: f32 = 0.87;
/// Opacity of the part of a slider's track past its value.
const TRACK_ALPHA: u8 = 0x50;
/// A slider's row is this much taller than the other entries, whatever
/// their size (`row_scale` included), for a finger.
const SLIDER_SCALE: f32 = 1.2;
/// A round button's diameter, relative to the entries' height.
const BUTTON_SCALE: f32 = 1.1;
/// The space between two round buttons, relative to their diameter.
const BUTTON_GAP: f32 = 0.6;
/// The space above and below the round buttons, relative to their diameter.
const BUTTON_MARGIN: f32 = 0.16;
/// A round button's icon, relative to its diameter.
const BUTTON_ICON: f32 = 0.5;
/// Opacity of a round button's disc when not highlighted.
const BUTTON_ALPHA: u8 = 0x22;
/// The ring around the selected round button: its thickness and its gap
/// from the disc, relative to the diameter.
const RING_WIDTH: f32 = 0.06;
const RING_GAP: f32 = 0.05;

/// What can be highlighted: an entry of the scrolling list (a slot on a
/// save state page), a round button or a slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Item(usize),
    Button(usize),
    Slider(usize),
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
                bold: false,
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
    /// The scrolling entries.
    items: Vec<MenuItem>,
    /// The round buttons, pinned at the bottom.
    buttons: Vec<MenuItem>,
    /// The sliders, under the buttons (brightness first).
    sliders: Vec<MenuItem>,
    selected: Target,
    /// What is under the mouse pointer: highlighted, not selected.
    hovered: Option<Target>,
    /// The save state page, when open; its slots are `Target::Item`s.
    page: Option<SlotPage>,
    /// What was selected, and the scroll, when the page was opened.
    main_selected: Target,
    main_scroll: f32,
    /// How far the entries (or the slots) are scrolled, in pixels.
    scroll: f32,
    /// The entry and the button last selected, to go back to them.
    item_memory: usize,
    button_memory: usize,
    /// The button whose function shows next to it.
    tooltip: Option<usize>,
    /// The status line above the header.
    status_bar: StatusBar,
    /// How much taller than a line of text the main entries are.
    row_scale: f32,
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
        let mut title_element = Element::new(&theme.title, scale, base * FONT_SIZE_LARGE);
        title_element.font = title_element.font.bold();
        let text = Element::new(&theme.text, scale, base * FONT_SIZE_MEDIUM);
        let small = Element::new(&theme.text_small, scale, base * FONT_SIZE_SMALL);

        let letters = fonts.measure(&text.font, "S") * WIDTH_IN_LETTERS;
        let panel_width = (width * 0.5).min(letters).max(width / 3.0);

        let mut panel = Self {
            width: panel_width.round() as u32,
            screen_height: height,
            title: title.to_uppercase(),
            subtitle: subtitle.to_owned(),
            items,
            buttons: Vec::new(),
            sliders: Vec::new(),
            selected: Target::Item(0),
            hovered: None,
            page: None,
            main_selected: Target::Item(0),
            main_scroll: 0.0,
            scroll: 0.0,
            item_memory: 0,
            button_memory: 0,
            tooltip: None,
            status_bar: StatusBar::default(),
            row_scale: 1.0,
            icons: true,
            status: None,
            background: theme.background.center_color,
            title_element,
            text,
            small,
        };
        panel.fix_selection();
        panel
    }

    /// The round buttons at the bottom.
    pub fn set_buttons(&mut self, buttons: Vec<MenuItem>) {
        self.buttons = buttons;
        self.fix_selection();
    }

    /// The line under the header (None: none).
    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
        self.scroll_to_selection();
    }

    /// The status line above the header; true when it changed.
    pub fn set_status_bar(&mut self, bar: StatusBar) -> bool {
        let changed = bar != self.status_bar;
        self.status_bar = bar;
        changed
    }

    /// Set a slider, adding it (brightness first) when the menu does not
    /// have it yet.
    pub fn set_slider(&mut self, kind: SliderKind, label: &str, value: f32, muted: bool) {
        let action = MenuAction::Slider(kind);
        if let Some(item) = self.sliders.iter_mut().find(|item| item.action == action) {
            item.value = Some(value.clamp(0.0, 1.0));
            item.muted = muted;
            return;
        }
        let position = self
            .sliders
            .iter()
            .position(|item| {
                matches!(item.action, MenuAction::Slider(other) if slider_order(other) > slider_order(kind))
            })
            .unwrap_or(self.sliders.len());
        let mut item = MenuItem::new(label.to_owned(), action);
        item.value = Some(value.clamp(0.0, 1.0));
        item.muted = muted;
        self.sliders.insert(position, item);
        // the same slider stays selected
        for selected in [&mut self.selected, &mut self.main_selected] {
            if let Target::Slider(index) = selected
                && position <= *index
            {
                *index += 1;
            }
        }
        self.fix_selection();
        self.scroll_to_selection();
    }

    /// Move the selected slider by `delta`: what changed, if it is a slider.
    pub fn adjust(&mut self, delta: f32) -> Option<(SliderKind, f32)> {
        self.set_selected_value(|value| value + delta)
    }

    /// Set a slider from the pointer at `x` (surface coordinates): what
    /// changed, if `x` is on its track (or `force`, while dragging).
    pub fn slide_to(&mut self, index: usize, x: f32, force: bool) -> Option<(SliderKind, f32)> {
        let (start, end) = self.track();
        // a fingertip's width around the track
        let slack = self.slider_height() * 0.3;
        if !force && (x < start - slack || x > end + slack) {
            return None;
        }
        if self.page.is_some() || index >= self.sliders.len() {
            return None;
        }
        self.selected = Target::Slider(index);
        let value = (x - start) / (end - start);
        self.set_selected_value(|_| value)
    }

    fn set_selected_value(&mut self, change: impl Fn(f32) -> f32) -> Option<(SliderKind, f32)> {
        let (None, Target::Slider(index)) = (&self.page, self.selected) else {
            return None;
        };
        let item = self.sliders.get_mut(index)?;
        let (MenuAction::Slider(kind), Some(value), true) = (item.action, item.value, item.enabled)
        else {
            return None;
        };
        // whole percents, as shown
        let new = (change(value).clamp(0.0, 1.0) * 100.0).round() / 100.0;
        if (new - value).abs() < f32::EPSILON {
            return None;
        }
        item.value = Some(new);
        // moving the volume unmutes it (the mixer does too)
        if new > 0.0 {
            item.muted = false;
        }
        Some((kind, new))
    }

    /// Mute or unmute the volume, when its slider is selected: whether it
    /// is muted now.
    pub fn toggle_mute(&mut self) -> Option<bool> {
        let (None, Target::Slider(index)) = (&self.page, self.selected) else {
            return None;
        };
        let item = self
            .sliders
            .get_mut(index)
            .filter(|item| item.action == MenuAction::Slider(SliderKind::Volume) && item.enabled)?;
        item.muted = !item.muted;
        Some(item.muted)
    }

    /// What is selected.
    pub const fn selected(&self) -> Target {
        self.selected
    }

    /// Select something of the current page (from the pointer).
    pub fn select(&mut self, target: Target) {
        if self.exists(target) {
            self.set_selected(target);
        }
    }

    fn set_selected(&mut self, target: Target) {
        self.selected = target;
        match target {
            Target::Item(index) if self.page.is_none() => self.item_memory = index,
            Target::Button(index) => self.button_memory = index,
            _ => {}
        }
        self.scroll_to_selection();
    }

    /// Whether a target is there on the current page.
    fn exists(&self, target: Target) -> bool {
        match (target, &self.page) {
            (Target::Item(index), _) => index < self.count(),
            (_, Some(_)) => false,
            (Target::Button(index), None) => index < self.buttons.len(),
            (Target::Slider(index), None) => index < self.sliders.len(),
        }
    }

    /// Something that exists selected (the first stop when what was
    /// selected is gone, or was never there).
    fn fix_selection(&mut self) {
        if !self.exists(self.selected)
            && let Some(first) = self.stops().first().copied()
        {
            self.set_selected(first);
        }
    }

    /// Where Up and Down stop, in order: the entries, the buttons' row (at
    /// the button last selected) and the sliders.
    fn stops(&self) -> Vec<Target> {
        let mut stops: Vec<Target> = (0..self.count()).map(Target::Item).collect();
        if self.page.is_none() {
            if !self.buttons.is_empty() {
                stops.push(Target::Button(
                    self.button_memory.min(self.buttons.len() - 1),
                ));
            }
            stops.extend((0..self.sliders.len()).map(Target::Slider));
        }
        stops
    }

    /// Up or Down: the previous or next stop, round; true when it changed.
    pub fn move_selection(&mut self, delta: i32) -> bool {
        let stops = self.stops();
        if stops.is_empty() {
            return false;
        }
        let current = stops
            .iter()
            .position(|stop| match (stop, self.selected) {
                (Target::Button(_), Target::Button(_)) => true,
                (stop, selected) => *stop == selected,
            })
            .unwrap_or(0);
        let next = stops[(current as i32 + delta).rem_euclid(stops.len() as i32) as usize];
        let changed = next != self.selected;
        self.set_selected(next);
        changed
    }

    /// Left or Right among the buttons; false when not on them (or at the
    /// end).
    pub fn move_button(&mut self, delta: i32) -> bool {
        let Target::Button(index) = self.selected else {
            return false;
        };
        let next = index
            .saturating_add_signed(delta as isize)
            .min(self.buttons.len().saturating_sub(1));
        if next == index {
            return false;
        }
        self.set_selected(Target::Button(next));
        true
    }

    /// To the buttons (or the sliders) from the entries, and back; true
    /// when it changed.
    pub fn toggle_footer(&mut self) -> bool {
        if matches!(self.selected, Target::Item(_)) {
            self.go_to_footer()
        } else {
            self.go_to_list()
        }
    }

    /// To the buttons (or the first slider); true when it changed.
    pub fn go_to_footer(&mut self) -> bool {
        let target = if self.buttons.is_empty() {
            Target::Slider(0)
        } else {
            Target::Button(self.button_memory.min(self.buttons.len() - 1))
        };
        self.go(target)
    }

    /// Back to the entry last selected; true when it changed.
    pub fn go_to_list(&mut self) -> bool {
        self.go(Target::Item(
            self.item_memory.min(self.count().saturating_sub(1)),
        ))
    }

    fn go(&mut self, target: Target) -> bool {
        if self.page.is_some() || target == self.selected || !self.exists(target) {
            return false;
        }
        self.set_selected(target);
        true
    }

    /// The entries' and the sliders' icon size: larger with taller rows.
    fn row_icon(&self) -> f32 {
        (self.text.letter_height * ICON_SCALE * self.row_scale / ICON_ROW_SCALE)
            .round()
            .max(1.0)
    }

    /// The horizontal span of the sliders' tracks: from after the icon to
    /// before the value.
    fn track(&self) -> (f32, f32) {
        let padding = self.padding();
        let inner = self.width as f32 - 2.0 * padding;
        let start = padding + self.row_icon() + padding * ICON_GAP;
        (start, padding + inner * TRACK_END)
    }

    fn padding(&self) -> f32 {
        (self.width as f32 * 0.04).max(20.0)
    }

    /// Change the label of an entry (or a button) and whether it can be
    /// chosen.
    pub fn set_entry(&mut self, action: MenuAction, label: Option<&str>, enabled: bool) {
        let all = self.items.iter_mut().chain(self.buttons.iter_mut());
        for item in all.filter(|item| item.action == action) {
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
        self.main_scroll = self.scroll;
        let first = page.slots.iter().position(|slot| slot.enabled).unwrap_or(0);
        self.hovered = None;
        self.page = Some(page);
        self.scroll = 0.0;
        self.set_selected(Target::Item(first));
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
        self.scroll = self.main_scroll;
        self.hovered = None;
        self.fix_selection();
        self.scroll_to_selection();
        true
    }

    /// How many entries (or slots) the current page has.
    fn count(&self) -> usize {
        self.page
            .as_ref()
            .map_or(self.items.len(), |page| page.slots.len())
    }

    /// Taller rows, for fingers: 1 is ES's.
    pub fn set_row_scale(&mut self, scale: f32) {
        self.row_scale = scale.max(1.0);
        self.scroll_to_selection();
    }

    /// The height of a main entry, without its separator.
    pub fn row_height(&self) -> f32 {
        (self.text.font.line_height * self.row_scale).round()
    }

    /// The height of a slider's row, without its separator.
    fn slider_height(&self) -> f32 {
        (self.row_height() * SLIDER_SCALE).round()
    }

    /// A round button's diameter.
    fn button_size(&self) -> f32 {
        (self.row_height() * BUTTON_SCALE).round()
    }

    /// The height of the buttons' row (0: no buttons).
    fn buttons_height(&self) -> f32 {
        if self.buttons.is_empty() {
            0.0
        } else {
            (self.button_size() * (1.0 + 2.0 * BUTTON_MARGIN)).round()
        }
    }

    /// Whether the buttons and sliders show: not on a save state page.
    fn footer_shown(&self) -> bool {
        self.page.is_none() && !(self.buttons.is_empty() && self.sliders.is_empty())
    }

    /// The height of what is pinned at the bottom: the buttons' row and
    /// the sliders, each with a separator above.
    fn footer_height(&self) -> f32 {
        if !self.footer_shown() {
            return 0.0;
        }
        let line = self.line();
        let buttons = if self.buttons.is_empty() {
            0.0
        } else {
            line + self.buttons_height()
        };
        buttons + self.sliders.len() as f32 * (line + self.slider_height())
    }

    /// Where the scrolling entries are: from under the header to the
    /// footer.
    fn list_area(&self) -> (f32, f32) {
        let top = self.header_height();
        let bottom = (self.screen_height - self.footer_height()).max(top + 1.0);
        (top, bottom)
    }

    /// The height of a row of the current page, with its separator.
    fn pitch(&self) -> f32 {
        if self.page.is_some() {
            self.slot_height()
        } else {
            self.row_height() + self.line()
        }
    }

    /// How far the current page can scroll.
    fn max_scroll(&self) -> f32 {
        let (top, bottom) = self.list_area();
        (self.count() as f32 * self.pitch() - (bottom - top)).max(0.0)
    }

    /// Scroll so that the selected entry shows whole.
    fn scroll_to_selection(&mut self) {
        if let Target::Item(index) = self.selected {
            let (top, bottom) = self.list_area();
            let pitch = self.pitch();
            let row_top = index as f32 * pitch;
            if row_top < self.scroll {
                self.scroll = row_top;
            } else if row_top + pitch > self.scroll + (bottom - top) {
                self.scroll = row_top + pitch - (bottom - top);
            }
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// Whether `y` (surface coordinates) is on the scrolling entries.
    pub fn in_list(&self, y: f32) -> bool {
        let (top, bottom) = self.list_area();
        (top..bottom).contains(&y)
    }

    /// Scroll the current page by `pixels` (down when positive), keeping a
    /// selected entry among the ones shown whole; true when it moved.
    pub fn scroll_by(&mut self, pixels: f32) -> bool {
        let scroll = (self.scroll + pixels).clamp(0.0, self.max_scroll());
        if (scroll - self.scroll).abs() < 0.5 {
            return false;
        }
        self.scroll = scroll;
        if let Target::Item(index) = self.selected {
            let (top, bottom) = self.list_area();
            let pitch = self.pitch();
            // the rows shown whole (a hair of slack for rounding)
            let first = ((scroll - 0.5) / pitch).ceil().max(0.0) as usize;
            let last = (((scroll + bottom - top + 0.5) / pitch).floor() as usize).saturating_sub(1);
            let last = last.max(first).min(self.count().saturating_sub(1));
            let clamped = index.clamp(first.min(last), last);
            if clamped != index {
                self.selected = Target::Item(clamped);
                if self.page.is_none() {
                    self.item_memory = clamped;
                }
            }
        }
        true
    }

    /// Highlight what is under the pointer (None: nothing); true when it
    /// changed.
    pub fn hover(&mut self, target: Option<Target>) -> bool {
        let target = target.filter(|target| self.exists(*target));
        let changed = target != self.hovered;
        self.hovered = target;
        changed
    }

    /// The button highlighted (selected, or under the pointer), whose
    /// function shows after a while.
    pub fn highlighted_button(&self) -> Option<usize> {
        match (&self.page, self.hovered.unwrap_or(self.selected)) {
            (None, Target::Button(index)) => Some(index),
            _ => None,
        }
    }

    /// Show what a button does next to it (None: hide it); true when it
    /// changed.
    pub fn set_tooltip(&mut self, button: Option<usize>) -> bool {
        let changed = button != self.tooltip;
        self.tooltip = button;
        changed
    }

    /// What a target does (None: nothing, e.g. an empty slot to load).
    pub fn action(&self, target: Target) -> Option<MenuAction> {
        let entry = |items: &[MenuItem], index: usize| {
            items
                .get(index)
                .filter(|item| item.enabled)
                .map(|item| item.action)
        };
        match (&self.page, target) {
            (Some(page), Target::Item(index)) => page
                .slots
                .get(index)
                .filter(|slot| slot.enabled)
                .map(|slot| MenuAction::Slot(page.mode, slot.number)),
            (Some(_), _) => None,
            (None, Target::Item(index)) => entry(&self.items, index),
            (None, Target::Button(index)) => entry(&self.buttons, index),
            (None, Target::Slider(index)) => entry(&self.sliders, index),
        }
    }

    pub fn selected_action(&self) -> Option<MenuAction> {
        self.action(self.selected)
    }

    /// The round buttons' centres, and their row's top.
    fn button_centres(&self) -> (Vec<(f32, f32)>, f32) {
        let size = self.button_size();
        let gap = (size * BUTTON_GAP).round();
        let count = self.buttons.len() as f32;
        let total = count * size + (count - 1.0).max(0.0) * gap;
        let left = ((self.width as f32 - total) / 2.0).round();
        let row_top = self.list_area().1 + self.line();
        let middle = row_top + self.buttons_height() / 2.0;
        let centres = (0..self.buttons.len())
            .map(|index| (left + size / 2.0 + index as f32 * (size + gap), middle))
            .collect();
        (centres, row_top)
    }

    /// What is at a point of the panel (surface coordinates), if anything.
    pub fn target_at(&self, x: f32, y: f32) -> Option<Target> {
        let (top, bottom) = self.list_area();
        if y < top {
            return None;
        }
        if y < bottom {
            let index = ((y - top + self.scroll) / self.pitch()) as usize;
            return (index < self.count()).then_some(Target::Item(index));
        }
        if !self.footer_shown() {
            return None;
        }
        let line = self.line();
        let mut row_top = bottom;
        if !self.buttons.is_empty() {
            row_top += line + self.buttons_height();
            if y < row_top {
                // a finger's slack around each button
                let reach = self.button_size() / 2.0 * 1.25;
                let (centres, _) = self.button_centres();
                return centres
                    .iter()
                    .position(|&(cx, cy)| (x - cx).hypot(y - cy) <= reach)
                    .map(Target::Button);
            }
        }
        let index = ((y - row_top) / (line + self.slider_height())) as usize;
        (index < self.sliders.len()).then_some(Target::Slider(index))
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

    /// The status line's height (0: nothing to show).
    fn status_bar_height(&self) -> f32 {
        if self.status_bar.is_empty() {
            0.0
        } else {
            self.status_font().line_height + self.screen_height * SUBTITLE_VERT_PADDING
        }
    }

    /// The status line's font: the small text's, larger.
    fn status_font(&self) -> FontRef {
        let font = &self.small.font;
        FontRef {
            size: font.size * STATUS_SCALE,
            line_height: font.line_height * STATUS_SCALE,
            ..font.clone()
        }
    }

    fn header_height(&self) -> f32 {
        self.status_bar_height() + self.title_height()
    }

    /// The header without the status line.
    fn title_height(&self) -> f32 {
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

        let padding = self.padding();
        // the entries first: the header and the footer cover what scrolled
        // past them
        match &self.page {
            Some(page) => self.draw_slots(&mut pixmap, fonts, page, padding),
            None => self.draw_items(&mut pixmap, fonts, padding),
        }
        self.draw_scrollbar(&mut pixmap);

        let header = self.header_height();
        fill(&mut pixmap, 0.0, 0.0, width, header, self.background);
        let bar = self.status_bar_height();
        if bar > 0.0 {
            self.draw_status_bar(&mut pixmap, fonts, padding, bar);
        }
        self.draw_header(&mut pixmap, fonts, padding, (bar, header - bar));
        fill(
            &mut pixmap,
            0.0,
            header - self.line(),
            width,
            self.line(),
            self.text.style.separator_color,
        );
        if self.footer_shown() {
            self.draw_footer(&mut pixmap, fonts, padding);
            self.draw_tooltip(&mut pixmap, fonts);
        }
        Some(pixmap)
    }

    /// Where the shown entries are among all of them, when they scroll.
    fn draw_scrollbar(&self, pixmap: &mut Pixmap) {
        let (top, bottom) = self.list_area();
        let track = bottom - top;
        let content = self.count() as f32 * self.pitch();
        if content <= track + 0.5 {
            return;
        }
        let width = self.width as f32;
        let bar = (width * 0.006).max(3.0);
        let length = track * track / content;
        let at = top + track * self.scroll / content;
        fill(
            pixmap,
            width - bar,
            at,
            bar,
            length,
            self.text.style.selector_color,
        );
    }

    /// The status line: the time, then the network's and the battery's
    /// icons (and the battery's percentage), aligned to the right.
    fn draw_status_bar(&self, pixmap: &mut Pixmap, fonts: &mut Fonts, padding: f32, height: f32) {
        let color = self.small.style.color;
        let font = &self.status_font();
        let row = font.line_height;
        let top = height - row;
        let icon_size = (self.small.letter_height * STATUS_SCALE * ICON_SCALE)
            .round()
            .max(1.0);
        let gap = icon_size * 0.6;
        // the icons centred on the digits' ink, not on the line
        let middle = fonts
            .ink_middle(font, "0123456789", row)
            .unwrap_or(row / 2.0);
        let icon_top = (top + middle - icon_size / 2.0).round();
        let bar = &self.status_bar;
        // right to left
        let mut right = self.width as f32 - padding;
        let text = |pixmap: &mut Pixmap, fonts: &mut Fonts, right: &mut f32, text: &str| {
            let width = fonts.measure(font, text) + 1.0;
            *right -= width;
            fonts.draw(
                pixmap,
                font,
                text,
                TextBox {
                    x: *right,
                    y: top,
                    width,
                    height: row,
                    align: Align::Right,
                },
                color,
            );
        };
        if let Some(battery) = bar.status.battery {
            if bar.battery_percent {
                text(pixmap, fonts, &mut right, &format!("{}%", battery.percent));
                right -= icon_size * 0.25;
            }
            right -= icon_size;
            draw_icon(
                pixmap,
                Icon::battery(battery),
                icon_size,
                (right, icon_top),
                color,
            );
            right -= gap;
        }
        if let Some(network) = bar.status.network {
            right -= icon_size;
            draw_icon(
                pixmap,
                Icon::network(network),
                icon_size,
                (right, icon_top),
                color,
            );
            right -= gap;
        }
        if let Some(time) = &bar.time {
            text(pixmap, fonts, &mut right, time);
        }
    }

    /// The header: its title, fitted to the width, and the smaller lines,
    /// from `top`, `header` high.
    fn draw_header(&self, pixmap: &mut Pixmap, fonts: &mut Fonts, padding: f32, at: (f32, f32)) {
        let (top, header) = at;
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
                y: top,
                width: inner,
                height: title_box_height,
                align: Align::Center,
            },
            self.title_element.style.color,
        );
        let mut y = top + title_box_height;
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
        draw_icon(
            pixmap,
            Icon::Disc,
            icon_size,
            (left, y + (height - icon_size) / 2.0),
            color,
        );
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
    fn draw_row_background(&self, pixmap: &mut Pixmap, target: Target, y: f32, height: f32) {
        let width = self.width as f32;
        let style = &self.text.style;
        if target == self.selected {
            fill_selector(pixmap, y, width, height, style);
        } else if self.hovered == Some(target) {
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

    /// The colour of a row's text and icons.
    fn row_color(&self, target: Target, enabled: bool) -> Rgba {
        let style = &self.text.style;
        let color = if target == self.selected {
            style.selected_color
        } else {
            style.color
        };
        if enabled {
            color
        } else {
            color.with_alpha(DISABLED_ALPHA)
        }
    }

    /// The rows of the current page shown, at least in part: their indices
    /// and where the first one starts.
    fn shown_rows(&self) -> (std::ops::Range<usize>, f32) {
        let (top, bottom) = self.list_area();
        let pitch = self.pitch();
        let first = (self.scroll / pitch).floor() as usize;
        let last = (((self.scroll + bottom - top) / pitch).ceil() as usize).min(self.count());
        (
            first..last.max(first),
            top + first as f32 * pitch - self.scroll,
        )
    }

    /// The main entries, one line each.
    fn draw_items(&self, pixmap: &mut Pixmap, fonts: &mut Fonts, padding: f32) {
        let width = self.width as f32;
        let inner = width - 2.0 * padding;
        let line = self.line();
        let row_height = self.row_height();
        let style = &self.text.style;
        // icons as tall as capital letters plus a bit (with the rows), then a gap
        let icon_size = self.row_icon();
        let text_x = if self.icons {
            padding + icon_size + padding * ICON_GAP
        } else {
            padding
        };
        let (rows, mut y) = self.shown_rows();
        for index in rows {
            let item = &self.items[index];
            let target = Target::Item(index);
            self.draw_row_background(pixmap, target, y, row_height);
            let color = self.row_color(target, item.enabled);
            if self.icons
                && let Some(icon) = Icon::for_action(item.action)
            {
                let top = y + (row_height - icon_size) / 2.0;
                draw_icon(pixmap, icon, icon_size, (padding, top), color);
            }
            let label_width = inner - (text_x - padding);
            let (font, label) = fonts.fit(
                &self.text.font,
                &item.label.to_uppercase(),
                label_width,
                0.7,
            );
            fonts.draw(
                pixmap,
                &font,
                &label,
                TextBox {
                    x: text_x,
                    y,
                    width: label_width.max(1.0),
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

    /// What is pinned at the bottom: the round buttons, then the sliders,
    /// each under a separator.
    fn draw_footer(&self, pixmap: &mut Pixmap, fonts: &mut Fonts, padding: f32) {
        let width = self.width as f32;
        let line = self.line();
        let style = &self.text.style;
        let mut y = self.list_area().1;
        fill(
            pixmap,
            0.0,
            y,
            width,
            self.screen_height - y,
            self.background,
        );
        if !self.buttons.is_empty() {
            fill(pixmap, 0.0, y, width, line, style.separator_color);
            y += line + self.buttons_height();
            self.draw_buttons(pixmap);
        }
        let height = self.slider_height();
        for (index, slider) in self.sliders.iter().enumerate() {
            fill(pixmap, 0.0, y, width, line, style.separator_color);
            y += line;
            let target = Target::Slider(index);
            self.draw_row_background(pixmap, target, y, height);
            let color = self.row_color(target, slider.enabled);
            let value = slider.value.unwrap_or_default();
            let icon = match slider.action {
                MenuAction::Slider(SliderKind::Volume) if slider.muted => Icon::VolumeX,
                MenuAction::Slider(SliderKind::Volume) => Icon::volume(value),
                _ => Icon::Sun,
            };
            let size = self.row_icon();
            draw_icon(
                pixmap,
                icon,
                size,
                (padding, y + (height - size) / 2.0),
                color,
            );
            // muted: the slider faded, its value kept
            let slider_color = if slider.muted {
                color.with_alpha(DISABLED_ALPHA.min((color.0 & 0xff) as u8))
            } else {
                color
            };
            self.draw_slider(pixmap, fonts, value, (y, height), slider_color);
            y += height;
        }
    }

    /// The round buttons: a faint disc with the icon, the selector's
    /// colours when selected.
    fn draw_buttons(&self, pixmap: &mut Pixmap) {
        let style = &self.text.style;
        let size = self.button_size();
        let icon_size = (size * BUTTON_ICON).round().max(1.0);
        let (centres, _) = self.button_centres();
        for (index, (button, &(cx, cy))) in self.buttons.iter().zip(&centres).enumerate() {
            let target = Target::Button(index);
            let selected = target == self.selected;
            let disc = if selected {
                style.selector_color
            } else {
                style.color.with_alpha(BUTTON_ALPHA)
            };
            fill_circle(pixmap, (cx, cy), size / 2.0, disc);
            if selected {
                // a ring in the text's colour around it: it stands out whatever
                // the theme's selector
                let width = (size * RING_WIDTH).max(2.0).round();
                let radius = size / 2.0 + (size * RING_GAP).max(2.0).round() + width / 2.0;
                stroke_circle(pixmap, (cx, cy), radius, width, style.color);
            }
            if !selected && self.hovered == Some(target) {
                fill_circle(
                    pixmap,
                    (cx, cy),
                    size / 2.0,
                    style.selector_color.with_alpha(HOVER_ALPHA),
                );
            }
            if let Some(icon) = Icon::for_action(button.action) {
                // the play triangle looks centred a bit to the right
                let nudge = if button.action == MenuAction::Continue {
                    icon_size * 0.06
                } else {
                    0.0
                };
                draw_icon(
                    pixmap,
                    icon,
                    icon_size,
                    (cx - icon_size / 2.0 + nudge, cy - icon_size / 2.0),
                    self.row_color(target, button.enabled),
                );
            }
        }
    }

    /// What the highlighted button does, in a bubble above it.
    fn draw_tooltip(&self, pixmap: &mut Pixmap, fonts: &mut Fonts) {
        let Some(button) = self
            .tooltip
            .and_then(|index| self.buttons.get(index).map(|b| (index, b)))
        else {
            return;
        };
        let (index, button) = button;
        let (centres, row_top) = self.button_centres();
        let Some(&(cx, _)) = centres.get(index) else {
            return;
        };
        let style = &self.text.style;
        let font = &self.small.font;
        let margin = self.padding() / 2.0;
        let pad = (self.small.letter_height * 0.8).round();
        let (font, label) = fonts.fit(
            font,
            &button.label,
            self.width as f32 - 2.0 * (margin + pad),
            0.8,
        );
        let text_width = fonts.measure(&font, &label).ceil();
        let width = text_width + 2.0 * pad;
        let height = (font.line_height + pad * 0.5).round();
        let arrow = (height * 0.3).round();
        // above the button, inside the panel
        let left = (cx - width / 2.0)
            .clamp(margin, (self.width as f32 - margin - width).max(margin))
            .round();
        // over the selected button's ring
        let size = self.button_size();
        let ring = (size * RING_GAP).max(2.0).round() + (size * RING_WIDTH).max(2.0).round();
        let circle_top = row_top + (self.buttons_height() - size) / 2.0 - ring;
        let bottom = (circle_top - arrow * 0.6).round();
        let top = bottom - height;
        let color = style.selector_color;
        if let Some(path) = rounded_rect(left, top, width, height, height * 0.3) {
            fill_path(pixmap, &path, color);
        }
        let mut builder = PathBuilder::new();
        builder.move_to(cx - arrow, bottom - 1.0);
        builder.line_to(cx + arrow, bottom - 1.0);
        builder.line_to(cx, bottom + arrow * 0.8);
        builder.close();
        if let Some(path) = builder.finish() {
            fill_path(pixmap, &path, color);
        }
        fonts.draw(
            pixmap,
            &font,
            &label,
            TextBox {
                x: left + pad,
                y: top,
                width: text_width + 1.0,
                height,
                align: Align::Left,
            },
            style.selected_color,
        );
    }

    /// A slider like ES's: a line, filled up to the value, with a knob, and
    /// the value in percent after it.
    fn draw_slider(
        &self,
        pixmap: &mut Pixmap,
        fonts: &mut Fonts,
        value: f32,
        row: (f32, f32),
        color: Rgba,
    ) {
        let (y, height) = row;
        let (start, end) = self.track();
        let middle = y + height / 2.0;
        let thickness = (height * 0.1).max(3.0).round();
        let knob = (height * 0.2).max(6.0);
        let filled = start + (end - start) * value.clamp(0.0, 1.0);
        fill(
            pixmap,
            filled,
            middle - thickness / 2.0,
            end - filled,
            thickness,
            color.with_alpha(TRACK_ALPHA.min((color.0 & 0xff) as u8)),
        );
        fill(
            pixmap,
            start,
            middle - thickness / 2.0,
            filled - start,
            thickness,
            color,
        );
        fill_circle(pixmap, (filled, middle), knob, color);
        let padding = self.padding();
        let inner = self.width as f32 - 2.0 * padding;
        let value_x = padding + inner * VALUE_START;
        fonts.draw(
            pixmap,
            &self.small.font,
            &format!("{}%", (value * 100.0).round() as u32),
            TextBox {
                x: value_x,
                y,
                width: padding + inner - value_x,
                height,
                align: Align::Right,
            },
            color,
        );
    }

    /// The save state slots: preview on the left, slot and date on the right.
    fn draw_slots(&self, pixmap: &mut Pixmap, fonts: &mut Fonts, page: &SlotPage, padding: f32) {
        let width = self.width as f32;
        let line = self.line();
        let row = self.slot_height();
        let style = &self.text.style;
        let margin = row * 0.1;
        let preview_height = row - 2.0 * margin;
        let preview_width = preview_height * 4.0 / 3.0;
        let text_x = padding + preview_width + padding;
        let text_width = (width - text_x - padding).max(1.0);

        let (rows, first_y) = self.shown_rows();
        let first = rows.start;
        for index in rows {
            let slot = &page.slots[index];
            let y = first_y + (index - first) as f32 * row;
            let target = Target::Item(index);
            self.draw_row_background(pixmap, target, y, row - line);
            let color = self.row_color(target, slot.enabled);

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
    }
}

/// A filled circle.
fn fill_circle(pixmap: &mut Pixmap, centre: (f32, f32), radius: f32, color: Rgba) {
    if let Some(path) = PathBuilder::from_circle(centre.0, centre.1, radius) {
        fill_path(pixmap, &path, color);
    }
}

/// A circle's outline, `width` thick.
fn stroke_circle(pixmap: &mut Pixmap, centre: (f32, f32), radius: f32, width: f32, color: Rgba) {
    if let Some(path) = PathBuilder::from_circle(centre.0, centre.1, radius) {
        let mut paint = Paint::default();
        paint.set_color(color.to_color());
        paint.anti_alias = true;
        let stroke = tiny_skia::Stroke {
            width,
            ..tiny_skia::Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
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

/// An icon, `size` pixels square, at `at` (its top left corner).
fn draw_icon(pixmap: &mut Pixmap, icon: Icon, size: f32, at: (f32, f32), color: Rgba) {
    if let Some(image) = icon.render(size as u32, color) {
        pixmap.draw_pixmap(
            at.0.round() as i32,
            at.1.round() as i32,
            image.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
}

/// The order of the sliders, when there are both.
const fn slider_order(kind: SliderKind) -> u8 {
    match kind {
        SliderKind::Brightness => 0,
        SliderKind::Volume => 1,
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
