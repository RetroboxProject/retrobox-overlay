//! `overlay.ini`: the user's settings, read from the system configuration
//! directory first and the user's one last (the later file wins, key by key).
//!
//! ```ini
//! [overlay]
//! side = right            ; right | left
//! dim = 0.75              ; opacity of the black layer over the game
//! animation_ms = 150
//! take_keyboard = auto    ; auto (not when opened from a controller) | yes | no
//!
//! [clock]
//! enabled = true
//! size = big              ; big (on the free side) | small (status line)
//! time_format = %H:%M:%S  ; strftime; %p etc. follow the language
//! date_format = %A, %-d %B %Y
//! small_format = %H:%M    ; the small clock's
//! position = top          ; top | center | bottom
//! color = FFFFFFFF
//!
//! [status]
//! battery_percent = true  ; the battery's percentage next to its icon
//!
//! [touch]
//! row_scale = 1.0         ; taller rows for fingers: 1.5 to 1.66 on small
//!                         ; touch screens (1 to 2)
//!
//! [system]
//! standby = true          ; offer to sleep (the power button)
//! performance = true      ; offer the Nintendo Switch's performance
//!                         ; profiles there (switchroot's nvpmodel)
//!
//! [sounds]
//! volume = 50             ; the menu's sounds (moving, choosing, going
//!                         ; back), 0 to 100 (0: none); none either with
//!                         ; ES's "Enable navigation sounds" off
//!
//! [volume]                ; the volume keys (retrobox-overlay --volume)
//! toast = false           ; show the volume as they change it, at the top
//!                         ; of the screen on the menu's side (for sessions
//!                         ; without a desktop's own)
//!
//! [break]                 ; read by retrobox-inputd (retrobox_input::breaks)
//! after_min = 120         ; minutes of play before a break is suggested
//!                         ; (0: never)
//! every_min = 30          ; then again every so many minutes (0: once)
//!
//! [manual]
//! scaling = compositor    ; compositor | overlay
//!
//! [osk]                   ; the on-screen keyboard
//! width = 0.75            ; its width, 0.1 to 0.75 of the screen's (the right stick
//!                         ; changes it; saved here when the keyboard closes)
//! position = bottom       ; bottom | top (R3 flips it; saved here too)
//! ```

use std::path::Path;

use anyhow::{Context as _, Result};

use ini::Ini;

use crate::theme::Rgba;

pub const FILE_NAME: &str = "overlay.ini";

/// How long the hotkey is held to open the overlay, by default and at most:
/// shorter would catch the hotkey's own combinations, longer feels broken.
/// The rows' height, in lines of text (more for fingers).
const MIN_ROW_SCALE: f32 = 1.0;
const MAX_ROW_SCALE: f32 = 2.0;
/// The menu's sounds' volume, in percent, by default.
const SOUND_VOLUME: u8 = 50;

/// Which edge of the screen the menu slides from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Where the clock sits, vertically, in the free part of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalPosition {
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone)]
pub struct ClockConfig {
    pub enabled: bool,
    /// The big clock (time and date on the free side of the screen), or the
    /// small one (the time in the menu's status line).
    pub big: bool,
    /// The small clock's format (None: the language's default).
    pub small_format: Option<String>,
    /// None: the language's default (see `i18n`).
    pub time_format: Option<String>,
    pub date_format: Option<String>,
    pub position: VerticalPosition,
    pub color: Rgba,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub side: Side,
    pub dim: f32,
    pub take_keyboard: TakeKeyboard,
    pub icons: bool,
    pub animation_ms: u64,
    pub clock: ClockConfig,
    /// Whether the compositor scales and crops the manual's page
    /// (`wp_viewporter`); off, the overlay does, for compositors that show
    /// scaled buffers wrong.
    pub compositor_scaling: bool,
    pub system: SystemConfig,
    pub osk: OskConfig,
    /// How much taller than a line of text the menu's rows are (fingers).
    pub row_scale: f32,
    /// The volume of the menu's sounds, in percent (0: none).
    pub sound_volume: u8,
    /// Whether the volume keys show a toast of the volume.
    pub volume_toast: bool,
}

/// The on-screen keyboard's width, relative to the screen's: the least, the
/// most and the default.
pub const OSK_WIDTH: (f32, f32, f32) = (0.1, 0.75, 0.75);

/// Where the on-screen keyboard is, and how big.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OskView {
    /// Its width, relative to the screen's (`OSK_WIDTH`'s range).
    pub width: f32,
    /// At the top of the screen instead of the bottom.
    pub top: bool,
}

impl Default for OskView {
    fn default() -> Self {
        Self {
            width: OSK_WIDTH.2,
            top: false,
        }
    }
}

impl OskView {
    /// The same view with its width moved by `delta`, kept in range and in
    /// whole percents (what `overlay.ini` holds).
    #[must_use]
    pub fn resized(self, delta: f32) -> Self {
        let width = ((self.width + delta).clamp(OSK_WIDTH.0, OSK_WIDTH.1) * 100.0).round() / 100.0;
        Self { width, ..self }
    }

    /// The same view at the other edge of the screen.
    #[must_use]
    pub const fn flipped(self) -> Self {
        Self {
            top: !self.top,
            ..self
        }
    }
}

/// The on-screen keyboard's settings: the keyboard layout (xkb names), when
/// `overlay.ini` sets it (else the compositor's, the environment's or the
/// system's), and where it is and how big.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OskConfig {
    pub model: Option<String>,
    pub layout: Option<String>,
    pub variant: Option<String>,
    pub options: Option<String>,
    pub view: OskView,
}

impl OskConfig {
    /// Whether any name is set.
    pub const fn is_set(&self) -> bool {
        self.model.is_some()
            || self.layout.is_some()
            || self.variant.is_some()
            || self.options.is_some()
    }
}

/// Whether the overlay takes the keyboard (and with it the focus) from the
/// game. A game that loses the focus may slow down (many engines lower
/// their frame rate in the background), so opened from a controller it does
/// not by default: the controller is all it needs. Without the keyboard it
/// cannot tell another window taking the focus, so it stays until closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeKeyboard {
    /// Only when opened from the keyboard (Ctrl+Shift+O, Shift+F7, or by hand).
    Auto,
    Always,
    Never,
}

impl TakeKeyboard {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            other if is_off(other) => Self::Never,
            _ => Self::Always,
        }
    }

    /// Whether to take it, opened from a controller or not.
    pub const fn take(self, from_pad: bool) -> bool {
        match self {
            Self::Auto => !from_pad,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

/// The system's part of the menu.
#[derive(Debug, Clone)]
pub struct SystemConfig {
    /// Whether the battery's percentage shows next to its icon.
    pub battery_percent: bool,
    /// Whether the menu offers to put the computer to sleep (on the power
    /// button's page).
    pub standby: bool,
    /// Whether the power button's page offers the performance profiles,
    /// on a Nintendo Switch.
    pub performance: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            side: Side::Right,
            dim: 0.75,
            take_keyboard: TakeKeyboard::Auto,
            icons: true,
            animation_ms: 150,
            clock: ClockConfig {
                enabled: true,
                big: true,
                small_format: None,
                time_format: None,
                date_format: None,
                position: VerticalPosition::Top,
                color: Rgba(0xFFFF_FFFF),
            },
            compositor_scaling: true,
            osk: OskConfig::default(),
            system: SystemConfig {
                battery_percent: true,
                standby: true,
                performance: true,
            },
            row_scale: 1.0,
            sound_volume: SOUND_VOLUME,
            volume_toast: false,
        }
    }
}

/// An ini boolean that says no.
fn is_off(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "false" | "no" | "0" | "off"
    )
}

impl Config {
    /// Read `overlay.ini` from each directory in turn, later ones winning.
    pub fn load<P: AsRef<Path>>(dirs: &[P]) -> Self {
        let mut config = Self::default();
        for dir in dirs {
            let path = dir.as_ref().join(FILE_NAME);
            if !path.exists() {
                continue;
            }
            match Ini::load_from_file(&path) {
                Ok(ini) => config.apply(&ini),
                Err(err) => log::warn!("cannot read {}: {err}", path.display()),
            }
        }
        config
    }

    fn apply(&mut self, ini: &Ini) {
        if let Some(section) = ini.section(Some("overlay")) {
            if let Some(side) = section.get("side") {
                match side.trim().to_ascii_lowercase().as_str() {
                    "left" => self.side = Side::Left,
                    "right" => self.side = Side::Right,
                    other => log::warn!("overlay.ini: unknown side {other:?}"),
                }
            }
            if let Some(dim) = section
                .get("dim")
                .and_then(|value| value.trim().parse::<f32>().ok())
            {
                self.dim = dim.clamp(0.0, 1.0);
            }
            if section.get("pause_game").is_some() {
                log::info!("overlay.ini: pause_game is gone (the game is never paused): ignored");
            }
            if let Some(take) = section.get("take_keyboard") {
                self.take_keyboard = TakeKeyboard::parse(take);
            }
            if let Some(icons) = section.get("icons") {
                self.icons = !is_off(icons);
            }
            if let Some(ms) = section
                .get("animation_ms")
                .and_then(|v| v.trim().parse().ok())
            {
                self.animation_ms = ms;
            }
        }
        if let Some(section) = ini.section(Some("clock")) {
            let clock = &mut self.clock;
            if let Some(enabled) = section.get("enabled") {
                clock.enabled = !is_off(enabled);
            }
            if let Some(size) = section.get("size") {
                match size.trim().to_ascii_lowercase().as_str() {
                    "big" => clock.big = true,
                    "small" => clock.big = false,
                    other => log::warn!("overlay.ini: unknown clock size {other:?}"),
                }
            }
            if let Some(format) = section.get("small_format").filter(|f| !f.trim().is_empty()) {
                clock.small_format = Some(format.trim().to_owned());
            }
            if let Some(format) = section.get("time_format").filter(|f| !f.trim().is_empty()) {
                clock.time_format = Some(format.trim().to_owned());
            }
            if let Some(format) = section.get("date_format").filter(|f| !f.trim().is_empty()) {
                clock.date_format = Some(format.trim().to_owned());
            }
            if let Some(position) = section.get("position") {
                clock.position = match position.trim().to_ascii_lowercase().as_str() {
                    "center" => VerticalPosition::Center,
                    "bottom" => VerticalPosition::Bottom,
                    _ => VerticalPosition::Top,
                };
            }
            if let Some(color) = section.get("color").and_then(Rgba::parse) {
                clock.color = color;
            }
        }
        if let Some(percent) = ini
            .section(Some("status"))
            .and_then(|section| section.get("battery_percent"))
        {
            self.system.battery_percent = !is_off(percent);
        }
        if let Some(scale) = ini
            .section(Some("touch"))
            .and_then(|section| section.get("row_scale"))
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            self.row_scale = scale.clamp(MIN_ROW_SCALE, MAX_ROW_SCALE);
        }
        if let Some(standby) = ini
            .section(Some("system"))
            .and_then(|section| section.get("standby"))
        {
            self.system.standby = !is_off(standby);
        }
        if let Some(performance) = ini
            .section(Some("system"))
            .and_then(|section| section.get("performance"))
        {
            self.system.performance = !is_off(performance);
        }
        if let Some(scaling) = ini
            .section(Some("manual"))
            .and_then(|section| section.get("scaling"))
        {
            match scaling.trim().to_ascii_lowercase().as_str() {
                "compositor" => self.compositor_scaling = true,
                "overlay" => self.compositor_scaling = false,
                other => log::warn!("overlay.ini: unknown manual scaling {other:?}"),
            }
        }
        self.apply_sounds(ini);
        self.apply_osk(ini);
    }

    fn apply_sounds(&mut self, ini: &Ini) {
        if let Some(volume) = ini
            .section(Some("sounds"))
            .and_then(|section| section.get("volume"))
        {
            match volume.trim().parse::<f32>() {
                Ok(volume) => self.sound_volume = volume.clamp(0.0, 100.0).round() as u8,
                Err(_) => log::warn!("overlay.ini: unknown sounds volume {volume:?}"),
            }
        }
        if let Some(toast) = ini
            .section(Some("volume"))
            .and_then(|section| section.get("toast"))
        {
            self.volume_toast = !is_off(toast);
        }
    }

    fn apply_osk(&mut self, ini: &Ini) {
        if let Some(section) = ini.section(Some("osk")) {
            let name = |key| {
                section
                    .get(key)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            };
            let mut view = OskView::default();
            if let Some(width) = name("width").and_then(|value| value.parse::<f32>().ok()) {
                // 0.45 or 45 (percent)
                let fraction = if width > 1.0 { width / 100.0 } else { width };
                view = view.resized(fraction - view.width);
            }
            view.top = name("position").is_some_and(|value| value.eq_ignore_ascii_case("top"));
            self.osk = OskConfig {
                model: name("model"),
                layout: name("layout"),
                variant: name("variant"),
                options: name("options"),
                view,
            };
        }
    }
}

/// Write the on-screen keyboard's view in the `overlay.ini` of `dir` (the
/// user's), when it is not what the file says already: only those two keys
/// are touched, and the rest of the file (comments included) stays as it is.
pub fn save_osk_view(dir: &Path, view: OskView) -> Result<()> {
    let path = dir.join(FILE_NAME);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let width = format!("{:.2}", view.width);
    let position = if view.top { "top" } else { "bottom" };
    let new = set_keys(&text, "osk", &[("width", &width), ("position", position)]);
    if new == text {
        return Ok(());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    // all or nothing: a half-written file would lose the user's settings
    let temporary = dir.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&temporary, new)
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, &path).with_context(|| format!("cannot replace {}", path.display()))
}

/// `text` (an ini file) with the `keys` set in `section`: a key that is
/// there changes its value, the others are added at the end of the section,
/// and the section itself at the end of the file, when it is missing.
fn set_keys(text: &str, section: &str, keys: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = |line: &str| {
        line.trim()
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .map(|name| name.trim().to_ascii_lowercase())
    };
    let Some(start) = lines
        .iter()
        .position(|line| header(line).is_some_and(|name| name == section))
    else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{section}]"));
        lines.extend(keys.iter().map(|(key, value)| format!("{key} = {value}")));
        return lines.join("\n") + "\n";
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| header(line).is_some())
        .map_or(lines.len(), |offset| start + 1 + offset);
    // new keys go after the section's last line that says something
    let mut insert = (start + 1..end)
        .rev()
        .find(|&index| !lines[index].trim().is_empty())
        .map_or(start + 1, |index| index + 1);
    for (key, value) in keys {
        let line = format!("{key} = {value}");
        let found = (start + 1..end).find(|&index| {
            let text = lines[index].trim();
            !text.starts_with([';', '#'])
                && text
                    .split_once('=')
                    .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key))
        });
        if let Some(index) = found {
            lines[index] = line;
        } else {
            lines.insert(insert, line);
            insert += 1;
        }
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: OskView = OskView {
        width: 0.42,
        top: true,
    };

    #[test]
    fn the_keyboard_width_stays_in_range() {
        let view = OskView::default();
        assert!((view.resized(0.5).width - 0.75).abs() < 1e-6);
        assert!((view.resized(-1.0).width - 0.1).abs() < 1e-6);
        assert!((view.resized(-0.02).width - 0.73).abs() < 1e-6);
        assert!(view.flipped().top && !view.flipped().flipped().top);
    }

    #[test]
    fn keys_are_added_or_changed_where_they_belong() {
        let text = "[overlay]\nside = left ; mine\n\n[osk]\nlayout = es\n; a note\n\n[clock]\nenabled = no\n";
        let new = set_keys(text, "osk", &[("width", "0.42"), ("position", "top")]);
        assert_eq!(
            new,
            "[overlay]\nside = left ; mine\n\n[osk]\nlayout = es\n; a note\nwidth = 0.42\nposition = top\n\n[clock]\nenabled = no\n"
        );
        // again: nothing to add, the values change in place
        let again = set_keys(&new, "osk", &[("width", "0.5"), ("position", "top")]);
        assert!(again.contains("width = 0.5\n") && !again.contains("0.42"));
        assert_eq!(set_keys(&again, "osk", &[("width", "0.5")]), again);
    }

    #[test]
    fn a_missing_section_is_added_at_the_end() {
        assert_eq!(
            set_keys("", "osk", &[("width", "0.3")]),
            "[osk]\nwidth = 0.3\n"
        );
        assert_eq!(
            set_keys("[clock]\nenabled = no", "osk", &[("width", "0.3")]),
            "[clock]\nenabled = no\n\n[osk]\nwidth = 0.3\n"
        );
    }

    #[test]
    fn the_view_is_saved_only_when_it_changed() {
        let dir = std::env::temp_dir().join(format!("overlay-osk-{}", std::process::id()));
        let file = dir.join(FILE_NAME);
        std::fs::remove_dir_all(&dir).ok();
        save_osk_view(&dir, VIEW).unwrap();
        let config = Config::load(&[&dir]);
        assert_eq!(config.osk.view, VIEW);
        let written = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        save_osk_view(&dir, VIEW).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().modified().unwrap(),
            written
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn later_files_win() {
        let dir = std::env::temp_dir().join(format!("overlay-config-{}", std::process::id()));
        let (system, user) = (dir.join("etc"), dir.join("user"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(system.join(FILE_NAME), "[overlay]\nside=left\ndim=0.3\n").unwrap();
        std::fs::write(
            user.join(FILE_NAME),
            "[overlay]\ndim=2\n[clock]\nposition=top\nsize=small\n\
             [status]\nbattery_percent=no\n[touch]\nrow_scale=1.66\n\
             [sounds]\nvolume=150\n[system]\nperformance=off\n[volume]\ntoast=yes\n",
        )
        .unwrap();

        let config = Config::load(&[system, user]);
        assert_eq!(config.side, Side::Left);
        assert!((config.dim - 1.0).abs() < f32::EPSILON);
        assert_eq!(config.clock.position, VerticalPosition::Top);
        assert!(!config.clock.big && !config.system.battery_percent && config.system.standby);
        assert!(!config.system.performance);
        assert!((config.row_scale - 1.66).abs() < f32::EPSILON);
        assert_eq!(config.sound_volume, 100);
        assert!(config.volume_toast);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
