//! The game being played: `session.json` (written by the launcher) and the
//! game information EmulationStation exported (`/tmp/game.xml`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A controller of the game, whose evdev device the overlay grabs while open.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code, reason = "player and name are for the actions to come")]
pub struct SessionController {
    #[serde(default)]
    pub player: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub device: String,
}

/// The launcher's control channel (see emulator.rs).
#[derive(Debug, Clone, Deserialize)]
pub struct ControlInfo {
    pub socket: String,
    /// Whether the game can be paused.
    #[serde(default)]
    pub pause: bool,
    /// The hotkey actions the emulator can do.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Whether save states go by slot.
    #[serde(default)]
    pub slots: bool,
    /// Whether requests may press keys (which need the game focused).
    #[serde(default)]
    pub focus: bool,
}

/// Where the game's save states are: `<dir>/<name>.state<N>`, each with a
/// `.png` preview.
#[derive(Debug, Clone, Deserialize)]
pub struct SaveStatesInfo {
    pub dir: String,
    pub name: String,
}

/// What `session.json` says (only the parts the overlay uses).
#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub pid: i32,
    #[serde(default)]
    pub system: String,
    #[serde(default)]
    pub emulator: String,
    #[serde(default)]
    pub rom: String,
    #[serde(default)]
    pub controllers: Vec<SessionController>,
    #[serde(default)]
    pub control: Option<ControlInfo>,
    #[serde(default)]
    pub game_info: String,
    #[serde(default)]
    pub menu_theme: String,
    #[serde(default)]
    pub savestates: Option<SaveStatesInfo>,
}

impl Session {
    /// The running game's session, or None when no game runs.
    pub fn load(runtime_dir: &Path) -> Option<Self> {
        let path = runtime_dir.join("session.json");
        let text = fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Self>(&text) {
            Ok(session) if Path::new(&format!("/proc/{}", session.pid)).exists() => Some(session),
            Ok(_) => None,
            Err(err) => {
                log::warn!("cannot read {}: {err}", path.display());
                None
            }
        }
    }

    /// The menu theme file EmulationStation wrote, if the session names one.
    pub fn menu_theme_path(&self) -> Option<PathBuf> {
        (!self.menu_theme.is_empty()).then(|| PathBuf::from(&self.menu_theme))
    }
}

/// The scraped information of the game (EmulationStation's `game.xml`).
#[derive(Debug, Clone, Default)]
pub struct GameInfo {
    pub name: String,
    pub developer: String,
    pub release_year: String,
}

impl GameInfo {
    /// Read game.xml; falls back to the ROM's file name.
    pub fn load(path: &Path, rom: &str) -> Self {
        let mut info = fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::parse(&text))
            .unwrap_or_default();
        if info.name.is_empty() {
            info.name = Path::new(rom)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        info
    }

    fn parse(text: &str) -> Option<Self> {
        let document = roxmltree::Document::parse(text).ok()?;
        let game = document
            .descendants()
            .find(|node| node.has_tag_name("game"))?;
        let field = |name: &str| {
            game.children()
                .find(|node| node.has_tag_name(name))
                .and_then(|node| node.text())
                .map(|text| text.trim().to_owned())
                .unwrap_or_default()
        };
        Some(Self {
            name: field("name"),
            developer: field("developer"),
            // ES stores dates as 19950811T000000
            release_year: field("releasedate").chars().take(4).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_control() {
        let session: Session = serde_json::from_str(
            r#"{"pid": 1, "control": {"socket": "/run/x/control.sock", "pause": true,
                "actions": ["reset"], "slots": true, "focus": false},
                "savestates": {"dir": "/saves/psx", "name": "Game", "slot": 0},
                "unknown": 3}"#,
        )
        .unwrap();
        let control = session.control.unwrap();
        assert!(control.pause && control.slots && !control.focus);
        assert_eq!(control.actions, ["reset"]);
        assert_eq!(session.savestates.unwrap().name, "Game");
    }

    #[test]
    fn parses_game_xml() {
        let info = GameInfo::parse(
            "<gameList><game><path>/r/s.md</path><name>Sonic</name>\
             <developer>Sega</developer><releasedate>19910623T000000</releasedate></game></gameList>",
        )
        .unwrap();
        assert_eq!(info.name, "Sonic");
        assert_eq!(info.release_year, "1991");
    }
}
