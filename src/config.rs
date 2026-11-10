//! `overlay.ini`: the user's settings, read from the system configuration
//! directory first and the user's one last (the later file wins, key by key).
//!
//! ```ini
//! [overlay]
//! side = right            ; right | left
//! dim = 0.6               ; opacity of the black layer over the game
//! animation_ms = 150
//! hold_ms = 750           ; how long the hotkey is held to open it
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
//! [system]
//! standby = true          ; offer to sleep, outside games
//!
//! [manual]
//! scaling = compositor    ; compositor | overlay
//! ```

use std::path::Path;
use std::time::Duration;

use ini::Ini;

use crate::theme::Rgba;

pub const FILE_NAME: &str = "overlay.ini";

/// How long the hotkey is held to open the overlay, by default and at most:
/// shorter would catch the hotkey's own combinations, longer feels broken.
const DEFAULT_HOLD_MS: u64 = 750;
const MIN_HOLD_MS: u64 = 300;
const MAX_HOLD_MS: u64 = 3000;

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
    pub icons: bool,
    pub animation_ms: u64,
    /// How long the hotkey (or Select+A) is held to open the overlay.
    pub hold: Duration,
    pub clock: ClockConfig,
    /// Whether the compositor scales and crops the manual's page
    /// (`wp_viewporter`); off, the overlay does, for compositors that show
    /// scaled buffers wrong.
    pub compositor_scaling: bool,
    pub system: SystemConfig,
}

/// The system's part of the menu.
#[derive(Debug, Clone)]
pub struct SystemConfig {
    /// Whether the battery's percentage shows next to its icon.
    pub battery_percent: bool,
    /// Whether the menu offers to put the computer to sleep (outside games).
    pub standby: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            side: Side::Right,
            dim: 0.6,
            icons: true,
            animation_ms: 150,
            hold: Duration::from_millis(DEFAULT_HOLD_MS),
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
            system: SystemConfig {
                battery_percent: true,
                standby: true,
            },
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
            if let Some(icons) = section.get("icons") {
                self.icons = !is_off(icons);
            }
            if let Some(ms) = section
                .get("animation_ms")
                .and_then(|v| v.trim().parse().ok())
            {
                self.animation_ms = ms;
            }
            if let Some(ms) = section
                .get("hold_ms")
                .and_then(|v| v.trim().parse::<u64>().ok())
            {
                self.hold = Duration::from_millis(ms.clamp(MIN_HOLD_MS, MAX_HOLD_MS));
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
        if let Some(standby) = ini
            .section(Some("system"))
            .and_then(|section| section.get("standby"))
        {
            self.system.standby = !is_off(standby);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_files_win() {
        let dir = std::env::temp_dir().join(format!("overlay-config-{}", std::process::id()));
        let (system, user) = (dir.join("etc"), dir.join("user"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(system.join(FILE_NAME), "[overlay]\nside=left\ndim=0.3\n").unwrap();
        std::fs::write(
            user.join(FILE_NAME),
            "[overlay]\ndim=2\nhold_ms=50\n[clock]\nposition=top\nsize=small\n\
             [status]\nbattery_percent=no\n",
        )
        .unwrap();

        let config = Config::load(&[system, user]);
        assert_eq!(config.side, Side::Left);
        assert!((config.dim - 1.0).abs() < f32::EPSILON);
        assert_eq!(config.clock.position, VerticalPosition::Top);
        assert_eq!(config.hold, Duration::from_millis(MIN_HOLD_MS));
        assert!(!config.clock.big && !config.system.battery_percent && config.system.standby);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
