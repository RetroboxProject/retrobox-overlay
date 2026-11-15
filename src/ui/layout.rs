//! What the on-screen keyboard's keys type: the characters of the active
//! xkb keymap.
//!
//! The virtual keyboard types physical keys, so every character on screen is
//! a sequence of key presses (`Step`s: an evdev key with shift and `AltGr`)
//! that makes the active layout produce it. The layers come from the keymap:
//!
//! * letters and capitals: forty keys of the main block (digits, three rows
//!   of letters), at the first and second level;
//! * symbols: the rest of what the main block types at any of its first four
//!   levels, dead keys included, once each;
//! * accents: letters with accents the layout can type, directly or as a dead
//!   key followed by the letter (the application composes them), in
//!   lower case and capitals.
//!
//! The keymap is the compositor's, on Wayland (its `wl_keyboard` sends it
//! whether or not the overlay has the focus); else the one `overlay.ini`
//! names, the environment's (`XKB_DEFAULT_*`) or `/etc/default/keyboard`'s;
//! else a built-in US one.

use std::collections::HashMap;

use xkbcommon::xkb::{self, Keycode};

use crate::config::OskConfig;

/// How many character keys the grid has.
pub const SLOTS: usize = 40;

/// One key press on the virtual keyboard: an evdev key with its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub code: &'static str,
    pub shift: bool,
    /// Level 3 (`AltGr`: right alt).
    pub altgr: bool,
}

/// A key of the grid: its label and what typing it takes (nothing: it is
/// not on this layout).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slot {
    pub label: String,
    pub steps: Vec<Step>,
}

/// The grid's four pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Letters,
    Capitals,
    Symbols,
    Accents,
    AccentCapitals,
}

/// The characters of a layout, page by page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pages: [Vec<Slot>; 5],
}

impl Layout {
    /// The slots of a page: always [`SLOTS`] of them.
    pub fn page(&self, page: Page) -> &[Slot] {
        &self.pages[page as usize]
    }
}

/// The main block's keys, with their evdev codes: the grid's forty first
/// (digits, three rows), then the rest of the block.
const MAIN_BLOCK: [(&str, u32); 50] = [
    ("KEY_1", 2),
    ("KEY_2", 3),
    ("KEY_3", 4),
    ("KEY_4", 5),
    ("KEY_5", 6),
    ("KEY_6", 7),
    ("KEY_7", 8),
    ("KEY_8", 9),
    ("KEY_9", 10),
    ("KEY_0", 11),
    ("KEY_Q", 16),
    ("KEY_W", 17),
    ("KEY_E", 18),
    ("KEY_R", 19),
    ("KEY_T", 20),
    ("KEY_Y", 21),
    ("KEY_U", 22),
    ("KEY_I", 23),
    ("KEY_O", 24),
    ("KEY_P", 25),
    ("KEY_A", 30),
    ("KEY_S", 31),
    ("KEY_D", 32),
    ("KEY_F", 33),
    ("KEY_G", 34),
    ("KEY_H", 35),
    ("KEY_J", 36),
    ("KEY_K", 37),
    ("KEY_L", 38),
    ("KEY_SEMICOLON", 39),
    ("KEY_Z", 44),
    ("KEY_X", 45),
    ("KEY_C", 46),
    ("KEY_V", 47),
    ("KEY_B", 48),
    ("KEY_N", 49),
    ("KEY_M", 50),
    ("KEY_COMMA", 51),
    ("KEY_DOT", 52),
    ("KEY_SLASH", 53),
    // the rest of the block
    ("KEY_MINUS", 12),
    ("KEY_EQUAL", 13),
    ("KEY_LEFTBRACE", 26),
    ("KEY_RIGHTBRACE", 27),
    ("KEY_APOSTROPHE", 40),
    ("KEY_GRAVE", 41),
    ("KEY_BACKSLASH", 43),
    ("KEY_102ND", 86),
    ("KEY_SPACE", 57),
    ("KEY_TAB", 15),
];

/// How many of the block's keys can hold characters: the table's last two
/// entries are only there to be looked up by name.
const CHARACTER_KEYS: usize = 48;

/// The accented letters (and a few other characters) the accents page
/// offers, in order; the ones the layout cannot type are left out.
const ACCENTS: &str = "áéíóúüñç¿¡àèìòùïëäöâêîôûãõåæøœß€ªº°±";

/// An accented letter, the letter under it and the dead key that adds the
/// accent (a keysym).
const COMPOSED: [(char, char, u32); 31] = [
    ('á', 'a', 0xfe51),
    ('é', 'e', 0xfe51),
    ('í', 'i', 0xfe51),
    ('ó', 'o', 0xfe51),
    ('ú', 'u', 0xfe51),
    ('ý', 'y', 0xfe51),
    ('à', 'a', 0xfe50),
    ('è', 'e', 0xfe50),
    ('ì', 'i', 0xfe50),
    ('ò', 'o', 0xfe50),
    ('ù', 'u', 0xfe50),
    ('â', 'a', 0xfe52),
    ('ê', 'e', 0xfe52),
    ('î', 'i', 0xfe52),
    ('ô', 'o', 0xfe52),
    ('û', 'u', 0xfe52),
    ('ä', 'a', 0xfe57),
    ('ë', 'e', 0xfe57),
    ('ï', 'i', 0xfe57),
    ('ö', 'o', 0xfe57),
    ('ü', 'u', 0xfe57),
    ('ÿ', 'y', 0xfe57),
    ('ã', 'a', 0xfe53),
    ('õ', 'o', 0xfe53),
    ('ñ', 'n', 0xfe53),
    ('ç', 'c', 0xfe5b),
    ('å', 'a', 0xfe58),
    ('ø', 'o', 0xfe5b),
    ('œ', 'o', 0xfe5b),
    ('æ', 'a', 0xfe5b),
    ('ß', 's', 0xfe5b),
];

/// A character a key types at some level, or a dead key.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Glyph {
    Char(char),
    /// A dead keysym (`dead_acute`...).
    Dead(u32),
}

impl Glyph {
    fn label(&self) -> String {
        match self {
            Self::Char(c) => c.to_string(),
            Self::Dead(raw) => dead_label(*raw).to_owned(),
        }
    }
}

/// The accent a dead key adds, drawn on its own.
const fn dead_label(raw: u32) -> &'static str {
    match raw {
        0xfe50 => "`",
        0xfe52 => "^",
        0xfe53 => "~",
        0xfe54 => "¯",
        0xfe55 => "˘",
        0xfe56 => "˙",
        0xfe57 => "¨",
        0xfe58 => "˚",
        0xfe59 => "˝",
        0xfe5a => "ˇ",
        0xfe5b => "¸",
        0xfe5c => "˛",
        _ => "´",
    }
}

fn step(code: &'static str, level: u32) -> Step {
    Step {
        code,
        shift: level & 1 != 0,
        altgr: level & 2 != 0,
    }
}

fn slot(label: String, steps: Vec<Step>) -> Slot {
    Slot { label, steps }
}

/// The glyph a main block key types at a level of the keymap's first layout.
fn glyph(keymap: &xkb::Keymap, evdev: u32, level: u32) -> Option<Glyph> {
    let [sym] = keymap.key_get_syms_by_level(Keycode::new(evdev + 8), 0, level) else {
        return None;
    };
    let raw = sym.raw();
    if (0xfe50..=0xfe6f).contains(&raw) {
        return Some(Glyph::Dead(raw));
    }
    let text = xkb::keysym_to_utf8(*sym);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() && c != ' ' => Some(Glyph::Char(c)),
        _ => None,
    }
}

/// Where each glyph of the block is typed, the cheapest way (fewest
/// modifiers) first.
fn typeable(keymap: &xkb::Keymap) -> HashMap<String, Vec<Step>> {
    let mut found = HashMap::new();
    for level in 0..4 {
        for (name, evdev) in &MAIN_BLOCK[..CHARACTER_KEYS] {
            if let Some(glyph) = glyph(keymap, *evdev, level) {
                found
                    .entry(glyph_key(&glyph))
                    .or_insert_with(|| vec![step(name, level)]);
            }
        }
    }
    found
}

/// What `typeable` calls a glyph: the character, or the dead key's keysym.
fn glyph_key(glyph: &Glyph) -> String {
    match glyph {
        Glyph::Char(c) => c.to_string(),
        Glyph::Dead(raw) => format!("dead:{raw:x}"),
    }
}

/// The steps that type a letter with an accent: the letter itself when the
/// layout has it, else its dead key and the base letter.
fn compose(
    typeable: &HashMap<String, Vec<Step>>,
    letter: char,
    uppercase: bool,
) -> Option<Vec<Step>> {
    let target = if uppercase {
        single(letter.to_uppercase())?
    } else {
        letter
    };
    if let Some(steps) = typeable.get(&target.to_string()) {
        return Some(steps.clone());
    }
    let &(_, base, dead) = COMPOSED
        .iter()
        .find(|(accented, _, _)| *accented == letter)?;
    // ß, æ, œ and ø have no dead key that makes them: the table's dead key is
    // a guess that only the layout's compose table could confirm
    if matches!(letter, 'ß' | 'æ' | 'œ' | 'ø') {
        return None;
    }
    let base = if uppercase {
        single(base.to_uppercase())?
    } else {
        base
    };
    let mut steps = typeable.get(&format!("dead:{dead:x}"))?.clone();
    steps.extend(typeable.get(&base.to_string())?.iter().copied());
    Some(steps)
}

/// The only character of an iterator, if it has just one.
fn single(mut chars: impl Iterator<Item = char>) -> Option<char> {
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

impl Layout {
    /// The layout of a compiled keymap.
    pub fn from_keymap(keymap: &xkb::Keymap) -> Self {
        let grid = &MAIN_BLOCK[..SLOTS];
        let level_page = |level: u32| -> Vec<Slot> {
            grid.iter()
                .map(|(name, evdev)| {
                    glyph(keymap, *evdev, level).map_or_else(Slot::default, |glyph| {
                        slot(glyph.label(), vec![step(name, level)])
                    })
                })
                .collect()
        };
        let letters = level_page(0);
        let capitals = level_page(1);

        // what the grid already types is not repeated on the symbols page
        let shown: Vec<&str> = letters
            .iter()
            .chain(&capitals)
            .map(|slot| slot.label.as_str())
            .collect();
        let mut symbols: Vec<Slot> = Vec::new();
        for level in 0..4 {
            for (name, evdev) in &MAIN_BLOCK[..CHARACTER_KEYS] {
                let Some(glyph) = glyph(keymap, *evdev, level) else {
                    continue;
                };
                let label = glyph.label();
                let letter = matches!(&glyph, Glyph::Char(c) if c.is_alphanumeric());
                if letter
                    || shown.contains(&label.as_str())
                    || symbols.iter().any(|slot| slot.label == label)
                {
                    continue;
                }
                symbols.push(slot(label, vec![step(name, level)]));
            }
        }
        symbols.truncate(SLOTS);

        let found = typeable(keymap);
        let accents = |uppercase: bool| -> Vec<Slot> {
            ACCENTS
                .chars()
                .filter_map(|letter| {
                    let target = if uppercase {
                        single(letter.to_uppercase())?
                    } else {
                        letter
                    };
                    let steps = compose(&found, letter, uppercase)?;
                    Some(slot(target.to_string(), steps))
                })
                .take(SLOTS)
                .collect()
        };

        let mut pages = [letters, capitals, symbols, accents(false), accents(true)];
        for page in &mut pages {
            page.resize(SLOTS, Slot::default());
        }
        Self { pages }
    }

    /// The layout a keymap text describes (the compositor's keymap).
    pub fn from_keymap_text(text: String) -> Option<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_string(
            &context,
            text,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .map(|keymap| Self::from_keymap(&keymap))
    }

    /// The layout `overlay.ini` names, else the environment's, else the
    /// system's keyboard (`/etc/default/keyboard`), else US.
    pub fn load(config: &OskConfig) -> Self {
        let file = std::fs::read_to_string("/etc/default/keyboard").unwrap_or_default();
        let names = Names::resolve(config, &file);
        log::info!(
            "keyboard layout: {:?} variant {:?} (model {:?}, options {:?})",
            names.layout,
            names.variant,
            names.model,
            names.options
        );
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(
            &context,
            "",
            names.model.as_str(),
            names.layout.as_str(),
            names.variant.as_str(),
            Some(names.options.clone()).filter(|options| !options.is_empty()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .map_or_else(
            || {
                log::warn!("cannot compile the keymap: a US layout");
                Self::fallback()
            },
            |keymap| Self::from_keymap(&keymap),
        )
    }

    /// A US layout, for when no keymap can be compiled: no accents page.
    pub fn fallback() -> Self {
        let page = |rows: [&str; 4], shift: bool| -> Vec<Slot> {
            const CODES: [&str; 40] = [
                "KEY_1",
                "KEY_2",
                "KEY_3",
                "KEY_4",
                "KEY_5",
                "KEY_6",
                "KEY_7",
                "KEY_8",
                "KEY_9",
                "KEY_0",
                "KEY_Q",
                "KEY_W",
                "KEY_E",
                "KEY_R",
                "KEY_T",
                "KEY_Y",
                "KEY_U",
                "KEY_I",
                "KEY_O",
                "KEY_P",
                "KEY_A",
                "KEY_S",
                "KEY_D",
                "KEY_F",
                "KEY_G",
                "KEY_H",
                "KEY_J",
                "KEY_K",
                "KEY_L",
                "KEY_SEMICOLON",
                "KEY_Z",
                "KEY_X",
                "KEY_C",
                "KEY_V",
                "KEY_B",
                "KEY_N",
                "KEY_M",
                "KEY_COMMA",
                "KEY_DOT",
                "KEY_SLASH",
            ];
            rows.iter()
                .flat_map(|row| row.chars())
                .zip(CODES)
                .map(|(label, code)| {
                    slot(
                        label.to_string(),
                        vec![Step {
                            code,
                            shift,
                            altgr: false,
                        }],
                    )
                })
                .collect()
        };
        let letters = page(
            ["1234567890", "qwertyuiop", "asdfghjkl;", "zxcvbnm,./"],
            false,
        );
        let capitals = page(
            ["!@#$%^&*()", "QWERTYUIOP", "ASDFGHJKL:", "ZXCVBNM<>?"],
            true,
        );
        let empty = vec![Slot::default(); SLOTS];
        Self {
            pages: [letters, capitals, empty.clone(), empty.clone(), empty],
        }
    }
}

/// The names a keymap is compiled from (empty: xkbcommon's own default,
/// which reads `XKB_DEFAULT_*`).
struct Names {
    model: String,
    layout: String,
    variant: String,
    options: String,
}

impl Names {
    /// `overlay.ini`, else the `XKB_DEFAULT_*` variables, else
    /// `/etc/default/keyboard` (its text, `XKBLAYOUT="es"` lines).
    fn resolve(config: &OskConfig, file: &str) -> Self {
        let from_file = |key: &str| -> Option<String> {
            file.lines().find_map(|line| {
                let (name, value) = line.split_once('=')?;
                (name.trim() == key)
                    .then(|| value.trim().trim_matches('"').trim_matches('\'').to_owned())
                    .filter(|value| !value.is_empty())
            })
        };
        let pick = |ini: &Option<String>, variable: &str, key: &str| -> String {
            ini.clone()
                .or_else(|| std::env::var(variable).ok().filter(|v| !v.is_empty()))
                .or_else(|| from_file(key))
                .unwrap_or_default()
        };
        Self {
            model: pick(&config.model, "XKB_DEFAULT_MODEL", "XKBMODEL"),
            layout: pick(&config.layout, "XKB_DEFAULT_LAYOUT", "XKBLAYOUT"),
            variant: pick(&config.variant, "XKB_DEFAULT_VARIANT", "XKBVARIANT"),
            options: pick(&config.options, "XKB_DEFAULT_OPTIONS", "XKBOPTIONS"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(names: (&str, &str)) -> Option<Layout> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            names.0,
            names.1,
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        Some(Layout::from_keymap(&keymap))
    }

    fn labels(layout: &Layout, page: Page) -> Vec<&str> {
        layout
            .page(page)
            .iter()
            .map(|slot| slot.label.as_str())
            .collect()
    }

    #[test]
    fn every_page_has_all_its_slots() {
        let layout = Layout::fallback();
        for page in [
            Page::Letters,
            Page::Capitals,
            Page::Symbols,
            Page::Accents,
            Page::AccentCapitals,
        ] {
            assert_eq!(layout.page(page).len(), SLOTS);
        }
        assert_eq!(layout.page(Page::Letters)[22].label, "d");
        assert!(layout.page(Page::Capitals)[10].steps[0].shift);
    }

    #[test]
    fn reads_the_layout_name_files() {
        let config = OskConfig::default();
        let names = Names::resolve(
            &config,
            "XKBMODEL=\"pc105\"\nXKBLAYOUT=\"es\"\nXKBVARIANT=\"\"\nXKBOPTIONS='grp:alt_shift_toggle'\n",
        );
        // (the environment may override, in a developer's shell)
        if std::env::var("XKB_DEFAULT_LAYOUT").is_err() {
            assert_eq!(names.layout, "es");
        }
        if std::env::var("XKB_DEFAULT_OPTIONS").is_err() {
            assert_eq!(names.options, "grp:alt_shift_toggle");
        }
        let chosen = OskConfig {
            layout: Some("pt".into()),
            ..OskConfig::default()
        };
        assert_eq!(Names::resolve(&chosen, "XKBLAYOUT=es").layout, "pt");
    }

    #[test]
    fn spanish_layout_types_its_letters_and_accents() {
        let Some(layout) = layout(("es", "")) else {
            eprintln!("no xkb data here: skipped");
            return;
        };
        let letters = labels(&layout, Page::Letters);
        assert_eq!(letters[29], "ñ");
        let accents = layout.page(Page::Accents);
        let acute = accents.iter().find(|slot| slot.label == "á").unwrap();
        // a dead key, then the letter
        assert_eq!(acute.steps.len(), 2);
        assert_eq!(acute.steps[1].code, "KEY_A");
        let enye = accents.iter().find(|slot| slot.label == "ñ").unwrap();
        assert_eq!(enye.steps.len(), 1);
        assert!(
            layout
                .page(Page::AccentCapitals)
                .iter()
                .any(|slot| slot.label == "Á" && slot.steps[1].shift)
        );
        let symbols = labels(&layout, Page::Symbols);
        assert!(symbols.contains(&"@") && symbols.contains(&"€"));
    }

    #[test]
    fn us_layout_has_no_dead_keys_but_still_types() {
        let Some(layout) = layout(("us", "")) else {
            return;
        };
        assert_eq!(labels(&layout, Page::Letters)[39], "/");
        // without dead keys the accents page holds only what is typed directly
        assert!(
            layout
                .page(Page::Accents)
                .iter()
                .all(|slot| slot.label.is_empty() || slot.steps.len() == 1)
        );
    }
}
