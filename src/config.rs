//! `overlay.ini`: the user's settings, read from the system configuration
//! directory first and the user's one last (the later file wins, key by key).
//!
//! ```ini
//! [overlay]
//! side = right            ; right | left
//! dim = 0.6               ; opacity of the black layer over the game
//! animation_ms = 150
//!
//! [clock]
//! enabled = true
//! time_format = %H:%M:%S  ; strftime; %p etc. follow the language
//! date_format = %A, %-d %B %Y
//! position = center       ; top | center | bottom
//! color = FFFFFFFF
//! ```

use std::path::Path;

use ini::Ini;

use crate::theme::Rgba;

pub const FILE_NAME: &str = "overlay.ini";

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
    pub pause_game: bool,
    pub animation_ms: u64,
    pub clock: ClockConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            side: Side::Right,
            dim: 0.6,
            pause_game: true,
            animation_ms: 150,
            clock: ClockConfig {
                enabled: true,
                time_format: None,
                date_format: None,
                position: VerticalPosition::Center,
                color: Rgba(0xFFFF_FFFF),
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
            if let Some(pause) = section.get("pause_game") {
                self.pause_game = !is_off(pause);
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
            if let Some(format) = section.get("time_format").filter(|f| !f.trim().is_empty()) {
                clock.time_format = Some(format.trim().to_owned());
            }
            if let Some(format) = section.get("date_format").filter(|f| !f.trim().is_empty()) {
                clock.date_format = Some(format.trim().to_owned());
            }
            if let Some(position) = section.get("position") {
                clock.position = match position.trim().to_ascii_lowercase().as_str() {
                    "top" => VerticalPosition::Top,
                    "bottom" => VerticalPosition::Bottom,
                    _ => VerticalPosition::Center,
                };
            }
            if let Some(color) = section.get("color").and_then(Rgba::parse) {
                clock.color = color;
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
            "[overlay]\ndim=2\n[clock]\nposition=top\n",
        )
        .unwrap();

        let config = Config::load(&[system, user]);
        assert_eq!(config.side, Side::Left);
        assert!((config.dim - 1.0).abs() < f32::EPSILON);
        assert_eq!(config.clock.position, VerticalPosition::Top);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
