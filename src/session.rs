//! The game being played: `session.json` (written by the launcher) and the
//! game information EmulationStation exported (`/tmp/game.xml`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
#[allow(
    clippy::struct_excessive_bools,
    reason = "the flags session.json publishes"
)]
pub struct ControlInfo {
    pub socket: String,
    /// The hotkey actions the emulator can do.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Whether save states go by slot.
    #[serde(default)]
    pub slots: bool,
    /// Whether the channel knows the disc in the drive and the tray (`discs`).
    #[serde(default)]
    pub discs: bool,
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
    /// When the emulator started, in clock ticks since boot (`/proc`'s).
    #[serde(default)]
    pub start_time: u64,
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
    /// Whether the overlay may open over this game (the `overlay` setting,
    /// global, per system or per game).
    #[serde(default = "enabled")]
    pub overlay: bool,
    /// Whether the on-screen keyboard may open over this game (the emulator
    /// lists the `osk` feature, and the setting is not off).
    #[serde(default)]
    pub osk: bool,
    /// The game's pad-to-keyboard profile, if it has one.
    #[serde(default)]
    pub pad_profile: Option<PathBuf>,
}

const fn enabled() -> bool {
    true
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

    /// How long the game has been running: its emulator's age.
    pub fn running_for(&self) -> Option<Duration> {
        let uptime: f64 = fs::read_to_string("/proc/uptime")
            .ok()?
            .split_whitespace()
            .next()?
            .parse()
            .ok()?;
        let started = self.start_time as f64 / rustix::param::clock_ticks_per_second() as f64;
        (self.start_time > 0).then(|| Duration::from_secs_f64((uptime - started).max(0.0)))
    }

    /// The menu theme file EmulationStation wrote, if the session names one.
    pub fn menu_theme_path(&self) -> Option<PathBuf> {
        (!self.menu_theme.is_empty()).then(|| PathBuf::from(&self.menu_theme))
    }
}

/// The scraped information of the game (EmulationStation's `game.xml`):
/// the menu's title and the game's page. Empty when it is not known.
#[derive(Debug, Clone, Default)]
pub struct GameInfo {
    pub name: String,
    pub developer: String,
    pub publisher: String,
    pub genre: String,
    pub players: String,
    /// `19950811T000000`, as ES stores dates.
    pub release_date: String,
    /// 0 to 1.
    pub rating: Option<f32>,
    pub description: String,
    /// The scraped picture: the screenshot (`image`), else the box.
    pub image: Option<PathBuf>,
    /// The scraped manual (a PDF).
    pub manual: Option<PathBuf>,
    pub play_count: u32,
    /// The time played before this game started.
    pub game_time: Duration,
}

impl GameInfo {
    /// Read game.xml; falls back to the ROM's file name. Relative paths are
    /// the ROM's folder's (ES writes full ones, a gamelist may not).
    pub fn load(path: &Path, rom: &str) -> Self {
        let rom = Path::new(rom);
        let mut info = fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::parse(&text, rom.parent()))
            .unwrap_or_default();
        if info.name.is_empty() {
            info.name = rom
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        info
    }

    fn parse(text: &str, rom_folder: Option<&Path>) -> Option<Self> {
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
        let file = |name: &str| resolve(&field(name), rom_folder);
        Some(Self {
            name: field("name"),
            developer: field("developer"),
            publisher: field("publisher"),
            genre: field("genre"),
            players: field("players"),
            release_date: field("releasedate"),
            rating: field("rating")
                .parse()
                .ok()
                .filter(|rating: &f32| *rating > 0.0),
            description: field("desc"),
            image: file("image").or_else(|| file("thumbnail")),
            manual: file("manual"),
            play_count: field("playcount").parse().unwrap_or(0),
            game_time: Duration::from_secs(field("gametime").parse().unwrap_or(0)),
        })
    }

    /// The year it came out (empty when unknown).
    pub fn release_year(&self) -> &str {
        self.release_date.get(..4).unwrap_or_default()
    }
}

/// A file named in game.xml: as it is when absolute, else in the ROM's
/// folder (`./manuals/...`).
fn resolve(name: &str, rom_folder: Option<&Path>) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return Some(path.to_owned());
    }
    Some(rom_folder?.join(path.strip_prefix("./").unwrap_or(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_control() {
        let session: Session = serde_json::from_str(
            r#"{"pid": 1, "control": {"socket": "/run/x/control.sock",
                "actions": ["reset"], "slots": true, "focus": false},
                "savestates": {"dir": "/saves/psx", "name": "Game", "slot": 0},
                "unknown": 3}"#,
        )
        .unwrap();
        let control = session.control.unwrap();
        assert!(control.slots && !control.focus);
        assert_eq!(control.actions, ["reset"]);
        assert_eq!(session.savestates.unwrap().name, "Game");
    }

    #[test]
    fn parses_game_xml() {
        let info = GameInfo::parse(
            "<gameList><game><path>/r/s.md</path><name>Sonic</name>\
             <developer>Sega</developer><releasedate>19910623T000000</releasedate>\
             <manual>/r/manuals/s.pdf</manual><thumbnail>./images/s-thumb.png</thumbnail>\
             <rating>0.8</rating><gametime>3700</gametime><playcount>4</playcount>\
             </game></gameList>",
            Some(Path::new("/roms/md")),
        )
        .unwrap();
        assert_eq!(info.name, "Sonic");
        assert_eq!(info.release_year(), "1991");
        assert_eq!(info.manual.as_deref(), Some(Path::new("/r/manuals/s.pdf")));
        assert_eq!(
            info.image.as_deref(),
            Some(Path::new("/roms/md/images/s-thumb.png"))
        );
        assert_eq!(info.rating, Some(0.8));
        assert_eq!(info.game_time, Duration::from_secs(3700));
        assert_eq!(info.play_count, 4);
    }
}
