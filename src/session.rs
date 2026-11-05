//! The game being played: `session.json` (written by the launcher) and the
//! game information EmulationStation exported (`/tmp/game.xml`).

use std::collections::HashMap;
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

/// How the emulator performs one standard hotkey action.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Binding {
    Keys {
        keys: Vec<String>,
    },
    Exec {
        argv: Vec<String>,
    },
    Native {
        #[serde(default)]
        keys: Vec<String>,
    },
}

/// Where the game's save states are and how to ask the emulator for them
/// (RetroArch: its command pipe; states are `<dir>/<name>.state<N>`).
#[derive(Debug, Clone, Deserialize)]
pub struct SaveStatesInfo {
    pub control: String,
    pub dir: String,
    pub name: String,
    #[serde(default)]
    pub slot: i32,
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
    pub hotkeys: HashMap<String, Binding>,
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
    fn parses_session_bindings() {
        let session: Session = serde_json::from_str(
            r#"{"pid": 1, "hotkeys": {
                "exit": {"type": "exec", "argv": ["retrobox-emukill"]},
                "save_state": {"type": "native", "keys": ["KEY_LEFTSHIFT", "KEY_F3"]},
                "pause": {"type": "keys", "keys": ["KEY_F10"]}},
                "unknown": 3}"#,
        )
        .unwrap();
        assert!(matches!(session.hotkeys["exit"], Binding::Exec { .. }));
        assert!(
            matches!(&session.hotkeys["save_state"], Binding::Native { keys } if keys.len() == 2)
        );
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
