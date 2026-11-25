//! The keyboard and mouse of the controls page: EmulationStation's own
//! picture (`kblayout.svg`, in its resources; its keys are labelled with
//! their evdev names), in the menu's colours, the keys the profile presses
//! lit, and those of the selected mapping lit as the selection.
//!
//! It is drawn twice when the page opens, every key the profile presses
//! lit, and every key lit as selected; showing a mapping copies its keys
//! from the second picture onto the first.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

use resvg::usvg;
use tiny_skia::{FilterQuality, Paint, Pattern, Pixmap, Rect, SpreadMode, Transform};

use crate::theme::Rgba;

/// The picture, among EmulationStation's resources.
pub const KEYBOARD_SVG: &str = "/opt/retrobox/frontend/resources/kblayout.svg";

/// The colours the picture is drawn in (its own are greys).
#[derive(Debug, Clone, Copy)]
pub struct KeyColors {
    pub case: Rgba,
    pub key: Rgba,
    pub legend: Rgba,
    /// A key the profile presses.
    pub mapped: Rgba,
    /// A key of the selected mapping, and its legend.
    pub chosen: Rgba,
    pub chosen_legend: Rgba,
}

pub struct Keyboard {
    /// The keys the profile presses lit.
    mapped: Pixmap,
    /// Every key lit as selected.
    chosen: Pixmap,
    /// Where each key is, in pixels.
    keys: HashMap<String, Rect>,
}

/// The picture's colours, as its file has them.
const CASE: [&str; 2] = ["fill:#333333", "fill:#242424"];
const KEY: &str = "fill:#808080";
const LEGEND: &str = "fill:#ffffff";
/// The key that is not one (EmulationStation's editor clears a mapping
/// with it).
const NO_KEY: &str = "KEY_NONE";

/// A key's rectangle in the file: where its text is, its name, its place.
struct KeyRect {
    text: Range<usize>,
    name: String,
    rect: (f32, f32, f32, f32),
}

impl Keyboard {
    /// The picture `width` pixels wide, the `mapped` keys (`KEY_*`, `BTN_*`)
    /// lit; None when EmulationStation's picture is not there.
    pub fn load(
        path: &Path,
        width: u32,
        mapped: &HashSet<&str>,
        colors: &KeyColors,
    ) -> Option<Self> {
        let svg = std::fs::read_to_string(path).ok()?;
        let document = roxmltree::Document::parse(&svg)
            .inspect_err(|err| log::warn!("{}: {err}", path.display()))
            .ok()?;
        let view_width = document
            .root_element()
            .attribute("viewBox")
            .and_then(|view| view.split_whitespace().nth(2))
            .and_then(|width| width.parse::<f32>().ok())?;
        let rects = key_rects(&document);
        drop(document);

        let scale = width as f32 / view_width;
        let mut mapped = render(
            &svg,
            &rects,
            |name| {
                if mapped.contains(name) {
                    colors.mapped
                } else {
                    colors.key
                }
            },
            colors.case,
            colors.legend,
            scale,
        )?;
        let chosen = render(
            &svg,
            &rects,
            |_| colors.chosen,
            colors.case,
            colors.chosen_legend,
            scale,
        )?;
        // the editor's "Select none", which is no key
        if let Some(none) = rects.iter().find(|key| key.name == NO_KEY) {
            let (x, y, width, height) = none.rect;
            // a pixel more around it: no trace of its edges
            let rect = Rect::from_xywh(
                x * scale - 1.0,
                y * scale - 1.0,
                width * scale + 2.0,
                height * scale + 2.0,
            );
            if let Some(rect) = rect {
                let mut paint = Paint::default();
                paint.set_color(colors.case.to_color());
                mapped.fill_rect(rect, &paint, Transform::identity(), None);
            }
        }
        let keys = rects
            .into_iter()
            .filter_map(|key| {
                let (x, y, width, height) = key.rect;
                let rect = Rect::from_xywh(x * scale, y * scale, width * scale, height * scale)?;
                Some((key.name, rect))
            })
            .collect();
        Some(Self {
            mapped,
            chosen,
            keys,
        })
    }

    pub fn height(&self) -> u32 {
        self.mapped.height()
    }

    /// Draw it at `(x, y)`, the `chosen` keys lit as selected.
    pub fn draw(&self, pixmap: &mut Pixmap, x: f32, y: f32, chosen: &[String]) {
        crate::ui::blit(pixmap, &self.mapped, (x.round() as i32, y.round() as i32));
        let at = Transform::from_translate(x.round(), y.round());
        let paint = Paint {
            shader: Pattern::new(
                self.chosen.as_ref(),
                SpreadMode::Pad,
                FilterQuality::Nearest,
                1.0,
                Transform::identity(),
            ),
            ..Paint::default()
        };
        for rect in chosen.iter().filter_map(|key| self.keys.get(key)) {
            pixmap.fill_rect(*rect, &paint, at, None);
        }
    }
}

/// The keys' rectangles (`inkscape:label` holds their names).
fn key_rects(document: &roxmltree::Document) -> Vec<KeyRect> {
    const INKSCAPE: &str = "http://www.inkscape.org/namespaces/inkscape";
    document
        .descendants()
        .filter(|node| node.has_tag_name("rect"))
        .filter_map(|node| {
            let name = node.attribute((INKSCAPE, "label"))?;
            let number = |name: &str| node.attribute(name)?.parse::<f32>().ok();
            Some(KeyRect {
                text: node.range(),
                name: name.to_owned(),
                rect: (
                    number("x")?,
                    number("y")?,
                    number("width")?,
                    number("height")?,
                ),
            })
        })
        .filter(|key| key.name.starts_with("KEY_") || key.name.starts_with("BTN_"))
        .collect()
}

/// The picture in the menu's colours, each key in `key_color`'s.
fn render(
    svg: &str,
    rects: &[KeyRect],
    key_color: impl Fn(&str) -> Rgba,
    case: Rgba,
    legend: Rgba,
    scale: f32,
) -> Option<Pixmap> {
    // the keys first, where they are in the file (in order)
    let mut text = String::with_capacity(svg.len());
    let mut done = 0;
    for key in rects {
        text.push_str(&svg[done..key.text.start]);
        let fill = format!("fill:{}", key_color(&key.name).hex());
        text.push_str(&svg[key.text.clone()].replace(KEY, &fill));
        done = key.text.end;
    }
    text.push_str(&svg[done..]);
    let case = format!("fill:{}", case.hex());
    for grey in CASE {
        text = text.replace(grey, &case);
    }
    let text = text.replace(LEGEND, &format!("fill:{}", legend.hex()));

    let tree = usvg::Tree::from_str(&text, &usvg::Options::default())
        .inspect_err(|err| log::warn!("the keyboard's picture: {err}"))
        .ok()?;
    let size = tree.size();
    let mut pixmap = Pixmap::new(
        (size.width() * scale).ceil().max(1.0) as u32,
        (size.height() * scale).ceil().max(1.0) as u32,
    )?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg"
        xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
        width="40" height="10" viewBox="0 0 40 10">
        <rect style="fill:#333333" x="0" y="0" width="40" height="10"/>
        <rect style="fill:#808080" x="0" y="0" width="10" height="10" inkscape:label="KEY_A"/>
        <rect style="fill:#808080" x="20" y="0" width="10" height="10" inkscape:label="KEY_B"/>
    </svg>"#;

    const COLORS: KeyColors = KeyColors {
        case: Rgba(0x0000_00FF),
        key: Rgba(0x4040_40FF),
        legend: Rgba(0xFFFF_FFFF),
        mapped: Rgba(0x0000_FFFF),
        chosen: Rgba(0xFF00_00FF),
        chosen_legend: Rgba(0xFFFF_FFFF),
    };

    fn red_at(pixmap: &Pixmap, x: u32) -> (u8, u8) {
        let pixel = pixmap.pixel(x, 5).unwrap();
        (pixel.red(), pixel.blue())
    }

    #[test]
    fn lights_mapped_and_chosen_keys() {
        let dir = std::env::temp_dir().join(format!("keyboard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kblayout.svg");
        std::fs::write(&path, SVG).unwrap();
        let keyboard = Keyboard::load(&path, 80, &HashSet::from(["KEY_B"]), &COLORS).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(keyboard.height(), 20);
        assert_eq!(
            keyboard.keys["KEY_B"],
            Rect::from_xywh(40.0, 0.0, 20.0, 20.0).unwrap()
        );

        let mut pixmap = Pixmap::new(80, 20).unwrap();
        keyboard.draw(&mut pixmap, 0.0, 0.0, &[]);
        // A plain, B mapped
        assert_eq!(red_at(&pixmap, 5), (0x40, 0x40));
        assert_eq!(red_at(&pixmap, 45), (0x00, 0xFF));
        keyboard.draw(&mut pixmap, 0.0, 0.0, &["KEY_A".to_owned()]);
        assert_eq!(red_at(&pixmap, 5), (0xFF, 0x00));
    }
}
