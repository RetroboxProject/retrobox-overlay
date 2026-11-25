//! Text with the theme's fonts (cosmic-text) drawn into tiny-skia pixmaps.
//!
//! Only the fonts the theme names and a couple of common fallbacks are loaded
//! (scanning every system font would cost a noticeable part of the start-up);
//! the whole system set is used only when none of the fallbacks exists.
//! Each one's bold face is loaded too when it sits next to it (as
//! `Name-Bold.ttf` next to `Name-Regular.ttf`); without one, bold text is
//! drawn twice, a little apart.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use cosmic_text::fontdb::{self, Source};
use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use tiny_skia::Pixmap;

use crate::theme::Rgba;

/// Fonts with the scripts the overlay's languages need (Latin, Cyrillic), the
/// first one that exists being the default: Ubuntu Condensed, as
/// EmulationStation's own (its resource first, else the distribution's).
const FALLBACK_FONTS: &[&str] = &[
    "/opt/retrobox/frontend/resources/ubuntu_condensed.ttf",
    "/usr/share/fonts/truetype/ubuntu/UbuntuCondensed-Regular.ttf",
    "/usr/share/fonts/truetype/ubuntu/Ubuntu-C.ttf",
    "/usr/share/fonts/ubuntu/UbuntuCondensed-Regular.ttf",
    "/usr/share/fonts/ubuntu-fonts/UbuntuCondensed-Regular.ttf",
    "/usr/share/fonts/ubuntu/Ubuntu-C.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
];

/// Fixed-width fonts for terminals, the first one that exists being used.
const MONOSPACE_FONTS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    "/usr/share/fonts/liberation-mono/LiberationMono-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSansMono-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
    "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
    "/usr/share/fonts/ubuntu/UbuntuMono-R.ttf",
];

/// The path a `FontRef::monospace` carries: not a file, the key of the
/// fixed-width font that was found.
const MONOSPACE_KEY: &str = "monospace";

/// A font at a size, in pixels.
#[derive(Debug, Clone)]
pub struct FontRef {
    pub path: PathBuf,
    pub size: f32,
    pub line_height: f32,
    pub bold: bool,
}

impl FontRef {
    /// The fixed-width font (the default one when the system has none).
    pub fn monospace(size: f32, line_height: f32) -> Self {
        Self {
            path: PathBuf::from(MONOSPACE_KEY),
            size,
            line_height,
            bold: false,
        }
    }

    /// The same font, bold.
    pub fn bold(&self) -> Self {
        Self {
            bold: true,
            ..self.clone()
        }
    }
}

/// Where a piece of text goes and how.
#[derive(Debug, Clone, Copy)]
pub struct TextBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub align: Align,
}

/// How many shaped texts are kept (all of them are forgotten past that):
/// more than a menu's worth, with the lines tried while fitting.
const LAYOUTS_KEPT: usize = 2048;

/// What a shaped text depends on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LayoutKey {
    path: PathBuf,
    size: u32,
    line_height: u32,
    bold: bool,
    text: String,
    width: Option<u32>,
    align: u8,
}

impl LayoutKey {
    fn new(font: &FontRef, text: &str, width: Option<f32>, align: Align) -> Self {
        Self {
            path: font.path.clone(),
            size: font.size.to_bits(),
            line_height: font.line_height.to_bits(),
            bold: font.bold,
            text: text.to_owned(),
            width: width.map(f32::to_bits),
            align: match align {
                Align::Left => 0,
                Align::Right => 1,
                Align::Center => 2,
                Align::Justified => 3,
                Align::End => 4,
            },
        }
    }
}

pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    /// Texts already shaped, to be drawn again (a menu redraws the same
    /// ones over and over: shaping is most of what drawing it costs).
    layouts: HashMap<LayoutKey, Buffer>,
    /// The widths already measured.
    widths: HashMap<LayoutKey, f32>,
    families: HashMap<PathBuf, String>,
    default_family: Option<String>,
    /// The families that have a bold face.
    bold_families: HashSet<String>,
}

/// The bold face next to a font file: `Name-Bold.ttf` for
/// `Name-Regular.ttf` (or `Name.ttf`), if there is one.
fn bold_sibling(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let extension = path.extension()?.to_str()?;
    let base = stem
        .strip_suffix("-Regular")
        .or_else(|| stem.strip_suffix("Regular"))
        .unwrap_or(stem);
    ["-Bold", "Bold", "_Bold", "-Bd"]
        .iter()
        .map(|suffix| path.with_file_name(format!("{base}{suffix}.{extension}")))
        .find(|candidate| candidate != path && candidate.exists())
}

fn family_of(db: &fontdb::Database, path: &Path) -> Option<String> {
    db.faces()
        .find(|face| match &face.source {
            Source::File(source) | Source::SharedFile(source, _) => source == path,
            Source::Binary(_) => false,
        })
        .and_then(|face| face.families.first())
        .map(|(name, _)| name.clone())
}

impl Fonts {
    /// Load the theme's fonts plus the fallbacks.
    pub fn new<'a>(theme_fonts: impl IntoIterator<Item = &'a Path>, locale: &str) -> Self {
        let mut db = fontdb::Database::new();
        let mut families = HashMap::new();
        for path in theme_fonts {
            if path.as_os_str().is_empty() || families.contains_key(path) {
                continue;
            }
            match db.load_font_file(path) {
                Ok(()) => {
                    if let Some(family) = family_of(&db, path) {
                        families.insert(path.to_path_buf(), family);
                    }
                    if let Some(bold) = bold_sibling(path) {
                        db.load_font_file(bold).ok();
                    }
                }
                Err(err) => log::warn!("cannot load font {}: {err}", path.display()),
            }
        }

        let mut default_family = None;
        for path in FALLBACK_FONTS
            .iter()
            .map(Path::new)
            .filter(|path| path.exists())
        {
            if db.load_font_file(path).is_ok() && default_family.is_none() {
                default_family = family_of(&db, path);
                if let Some(bold) = bold_sibling(path) {
                    db.load_font_file(bold).ok();
                }
            }
        }
        if default_family.is_none() {
            db.load_system_fonts();
        }
        if let Some(path) = MONOSPACE_FONTS
            .iter()
            .map(Path::new)
            .find(|path| path.exists())
            && db.load_font_file(path).is_ok()
            && let Some(family) = family_of(&db, path)
        {
            families.insert(PathBuf::from(MONOSPACE_KEY), family);
        }
        if let Some(family) = &default_family {
            db.set_sans_serif_family(family.clone());
        }

        let bold_families = db
            .faces()
            .filter(|face| face.weight.0 >= Weight::SEMIBOLD.0)
            .filter_map(|face| face.families.first())
            .map(|(name, _)| name.clone())
            .collect();
        Self {
            system: FontSystem::new_with_locale_and_db(locale.to_owned(), db),
            cache: SwashCache::new(),
            layouts: HashMap::new(),
            widths: HashMap::new(),
            families,
            default_family,
            bold_families,
        }
    }

    fn family(&self, font: &FontRef) -> Option<&String> {
        self.families
            .get(&font.path)
            .or(self.default_family.as_ref())
    }

    fn attrs<'a>(&'a self, font: &FontRef) -> Attrs<'a> {
        let family = self
            .family(font)
            .map_or(Family::SansSerif, |name| Family::Name(name.as_str()));
        let attrs = Attrs::new().family(family);
        if font.bold {
            attrs.weight(Weight::BOLD)
        } else {
            attrs
        }
    }

    /// How far apart bold text without a bold face is drawn twice (0: it
    /// has one).
    fn faux_bold(&self, font: &FontRef) -> f32 {
        let has_bold = self
            .family(font)
            .is_some_and(|family| self.bold_families.contains(family));
        if font.bold && !has_bold {
            (font.size / 24.0).max(1.0).round()
        } else {
            0.0
        }
    }

    fn layout(&mut self, font: &FontRef, text: &str, width: Option<f32>, align: Align) -> Buffer {
        let metrics = Metrics::new(font.size.max(1.0), font.line_height.max(font.size).max(1.0));
        let mut buffer = Buffer::new(&mut self.system, metrics);
        buffer.set_wrap(Wrap::None);
        buffer.set_size(width, None);
        let attrs = self.attrs(font);
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.system, false);
        buffer
    }

    /// The width of a line of text.
    pub fn measure(&mut self, font: &FontRef, text: &str) -> f32 {
        let key = LayoutKey::new(font, text, None, Align::Left);
        if let Some(width) = self.widths.get(&key) {
            return *width;
        }
        let buffer = self.layout(font, text, None, Align::Left);
        let width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max)
            + self.faux_bold(font);
        if self.widths.len() >= LAYOUTS_KEPT {
            self.widths.clear();
        }
        self.widths.insert(key, width);
        width
    }

    /// Break a text into lines no wider than `width`, at its spaces (a word
    /// wider than that stays whole). When it takes more than `max_lines`,
    /// the last line holds all the rest.
    pub fn wrap(
        &mut self,
        font: &FontRef,
        text: &str,
        width: f32,
        max_lines: usize,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        for word in text.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if current.is_empty() || self.measure(font, &candidate) <= width {
                current = candidate;
            } else {
                lines.push(std::mem::replace(&mut current, word.to_owned()));
            }
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(current);
        }
        if lines.len() > max_lines.max(1) {
            let rest = lines.split_off(max_lines.max(1) - 1).join(" ");
            lines.push(rest);
        }
        lines
    }

    /// Make a line fit a width: a smaller font (down to `min_scale` of its
    /// size), then an ellipsis.
    pub fn fit(
        &mut self,
        font: &FontRef,
        text: &str,
        width: f32,
        min_scale: f32,
    ) -> (FontRef, String) {
        let natural = self.measure(font, text);
        if natural <= width || natural <= 0.0 {
            return (font.clone(), text.to_owned());
        }
        let scale = (width / natural).max(min_scale);
        let smaller = FontRef {
            size: font.size * scale,
            line_height: font.line_height * scale,
            ..font.clone()
        };
        if scale > min_scale {
            return (smaller, text.to_owned());
        }
        let mut shortened: Vec<char> = text.chars().collect();
        while !shortened.is_empty() {
            shortened.pop();
            let candidate = format!("{}…", shortened.iter().collect::<String>().trim_end());
            if self.measure(&smaller, &candidate) <= width {
                return (smaller, candidate);
            }
        }
        (smaller, "…".to_owned())
    }

    /// Where the ink of `text` is, vertically, in the middle of a box
    /// `height` high (as `draw` places it): the middle of its glyphs, to
    /// line icons up with them (the line's own middle sits lower or higher,
    /// depending on the font).
    pub fn ink_middle(&mut self, font: &FontRef, text: &str, height: f32) -> Option<f32> {
        let width = (self.measure(font, text).ceil() as u32).max(1) + 2;
        let mut scratch = Pixmap::new(width, height.ceil().max(1.0) as u32)?;
        self.draw(
            &mut scratch,
            font,
            text,
            TextBox {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height,
                align: Align::Left,
            },
            Rgba(0xffff_ffff),
        );
        let inked: Vec<usize> = scratch
            .pixels()
            .chunks_exact(width as usize)
            .enumerate()
            .filter(|(_, row)| row.iter().any(|pixel| pixel.alpha() > 0x40))
            .map(|(y, _)| y)
            .collect();
        let (first, last) = (*inked.first()?, *inked.last()?);
        Some((first + last + 1) as f32 / 2.0)
    }

    /// Draw one line of text, vertically centred in the box and clipped to it.
    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        font: &FontRef,
        text: &str,
        area: TextBox,
        color: Rgba,
    ) {
        let offset = self.faux_bold(font);
        if offset > 0.0 {
            // no bold face: twice, a little apart, within the same box
            let narrower = TextBox {
                width: area.width - offset,
                ..area
            };
            self.draw_once(pixmap, font, text, narrower, color);
            let shifted = TextBox {
                x: area.x + offset,
                ..narrower
            };
            self.draw_once(pixmap, font, text, shifted, color);
        } else {
            self.draw_once(pixmap, font, text, area, color);
        }
    }

    fn draw_once(
        &mut self,
        pixmap: &mut Pixmap,
        font: &FontRef,
        text: &str,
        area: TextBox,
        color: Rgba,
    ) {
        let key = LayoutKey::new(font, text, Some(area.width), area.align);
        if !self.layouts.contains_key(&key) {
            let buffer = self.layout(font, text, Some(area.width), area.align);
            if self.layouts.len() >= LAYOUTS_KEPT {
                self.layouts.clear();
            }
            self.layouts.insert(key.clone(), buffer);
        }
        let Some(buffer) = self.layouts.get_mut(&key) else {
            return;
        };
        let line_height = buffer.metrics().line_height;
        let top = area.y + (area.height - line_height) / 2.0;
        let (left, upper) = (area.x.max(0.0) as i32, area.y.max(0.0) as i32);
        let right = ((area.x + area.width) as i32).min(pixmap.width() as i32);
        let lower = ((area.y + area.height) as i32).min(pixmap.height() as i32);

        let width = pixmap.width() as i32;
        let pixels = pixmap.pixels_mut();
        buffer.draw(
            &mut self.system,
            &mut self.cache,
            color.to_text_color(),
            |x, y, w, h, glyph| {
                let alpha = glyph.a();
                if alpha == 0 {
                    return;
                }
                for py in y..y + h as i32 {
                    let py = py + top as i32;
                    if py < upper || py >= lower {
                        continue;
                    }
                    for px in x..x + w as i32 {
                        let px = px + area.x as i32;
                        if px < left || px >= right {
                            continue;
                        }
                        blend(&mut pixels[(py * width + px) as usize], glyph, alpha);
                    }
                }
            },
        );
    }
}

/// Source-over of a straight-alpha colour onto a premultiplied pixel.
fn blend(pixel: &mut tiny_skia::PremultipliedColorU8, color: cosmic_text::Color, alpha: u8) {
    let a = u32::from(alpha);
    let inverse = 255 - a;
    let mix =
        |src: u8, dst: u8| ((u32::from(src) * a + u32::from(dst) * inverse + 127) / 255) as u8;
    let out_alpha = (a + (u32::from(pixel.alpha()) * inverse + 127) / 255) as u8;
    // the source premultiplied by its alpha is src * a / 255: mix() does it;
    // rounding must not leave a channel above the alpha
    let channel = |src: u8, dst: u8| mix(src, dst).min(out_alpha);
    if let Some(result) = tiny_skia::PremultipliedColorU8::from_rgba(
        channel(color.r(), pixel.red()),
        channel(color.g(), pixel.green()),
        channel(color.b(), pixel.blue()),
        out_alpha,
    ) {
        *pixel = result;
    }
}
