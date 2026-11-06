//! Talking to the emulator: through the launcher's control channel.
//!
//! The launcher serves a Unix socket while the game runs (session.json's
//! `control`) and carries out each request the best way the emulator allows:
//! its command pipe (RetroArch), a key chord on a virtual keyboard of its
//! own, or a command line. The overlay only asks, one line per request.
//!
//! Key chords only reach an emulator that has the keyboard focus. When the
//! channel says it may press keys (`focus`), the overlay asks for the pause
//! before it shows itself, and for everything else after it is gone and the
//! focus is back with the game.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::thread::sleep;
use std::time::Duration;

use crate::session::{ControlInfo, Session};

/// After the overlay closed, for the compositor to give the keyboard focus
/// back to the game and repaint it (screenshots).
const FOCUS_RETURN: Duration = Duration::from_millis(150);
/// How long an answer may take (a chord is pressed in well under that).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);

/// What the menu can ask of the emulator, besides leaving the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmuAction {
    SaveState,
    LoadState,
    DiscTray,
    NextDisc,
    PrevDisc,
    Reset,
    Menu,
    Screenshot,
}

impl EmuAction {
    /// In menu order.
    pub const ALL: [Self; 8] = [
        Self::SaveState,
        Self::LoadState,
        Self::DiscTray,
        Self::NextDisc,
        Self::PrevDisc,
        Self::Reset,
        Self::Menu,
        Self::Screenshot,
    ];

    /// The key of the entry's label in the translations.
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::SaveState => "menu-save-state",
            Self::LoadState => "menu-load-state",
            Self::DiscTray => "menu-disc-tray",
            Self::NextDisc => "menu-next-disc",
            Self::PrevDisc => "menu-prev-disc",
            Self::Reset => "menu-reset",
            Self::Menu => "menu-emulator-menu",
            Self::Screenshot => "menu-screenshot",
        }
    }

    /// Only for games made of several discs.
    const fn is_disc(self) -> bool {
        matches!(self, Self::DiscTray | Self::NextDisc | Self::PrevDisc)
    }

    /// The action's name on the control channel (Retrobox's hotkey actions).
    const fn name(self) -> &'static str {
        match self {
            Self::SaveState => "save_state",
            Self::LoadState => "load_state",
            Self::DiscTray => "disk_eject",
            Self::NextDisc => "disk_next",
            Self::PrevDisc => "disk_prev",
            Self::Reset => "reset",
            Self::Menu => "menu",
            Self::Screenshot => "screenshot",
        }
    }
}

/// What the menu asked for, done once the overlay is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Action(EmuAction),
    SaveState(u32),
    LoadState(u32),
}

impl Request {
    fn line(self) -> String {
        match self {
            Self::Action(action) => format!("do {}", action.name()),
            Self::SaveState(slot) => format!("save_slot {slot}"),
            Self::LoadState(slot) => format!("load_slot {slot}"),
        }
    }
}

/// Whether the game is a playlist of discs (the only case the disc entries mean anything).
fn is_playlist(rom: &str) -> bool {
    std::path::Path::new(rom)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("m3u"))
}

/// The emulator of the running game, as far as the overlay can drive it.
pub struct Emulator {
    control: Option<ControlInfo>,
    actions: Vec<EmuAction>,
    pause_game: bool,
    paused: bool,
}

impl Emulator {
    /// No emulator: the demo, or a game without a control channel.
    pub fn none() -> Self {
        Self {
            control: None,
            actions: Vec::new(),
            pause_game: false,
            paused: false,
        }
    }

    pub fn new(session: &Session, pause_game: bool) -> Self {
        let Some(control) = session.control.clone() else {
            log::warn!("no control channel in session.json: the menu can only leave the game");
            return Self::none();
        };
        let playlist = is_playlist(&session.rom);
        let actions = EmuAction::ALL
            .into_iter()
            .filter(|action| control.actions.iter().any(|name| name == action.name()))
            .filter(|action| playlist || !action.is_disc())
            // by slot, in their own pages, when the emulator allows it
            .filter(|action| {
                !(control.slots && matches!(action, EmuAction::SaveState | EmuAction::LoadState))
            })
            .collect();
        Self {
            pause_game: pause_game && control.pause,
            control: Some(control),
            actions,
            paused: false,
        }
    }

    /// The actions the menu can offer, in menu order.
    pub fn actions(&self) -> &[EmuAction] {
        &self.actions
    }

    /// Whether save states go by slot (the save and load pages).
    pub fn has_slots(&self) -> bool {
        self.control.as_ref().is_some_and(|control| control.slots)
    }

    /// Pause the game; called before the overlay shows itself, while the
    /// game still has the keyboard focus.
    pub fn pause(&mut self) {
        if self.pause_game && self.ask("pause") {
            self.paused = true;
        }
    }

    /// Once the overlay is gone: resume the game and do what the menu asked.
    pub fn after_close(&mut self, request: Option<Request>) {
        let focus = self.control.as_ref().is_some_and(|control| control.focus);
        if focus && (self.paused || request.is_some()) {
            sleep(FOCUS_RETURN);
        }
        if self.paused {
            self.ask("resume");
            self.paused = false;
        }
        if let Some(request) = request {
            self.ask(&request.line());
        }
    }

    /// Send one request and wait for its answer; true when it was done.
    fn ask(&self, line: &str) -> bool {
        let Some(control) = &self.control else {
            return false;
        };
        let answer = UnixStream::connect(&control.socket).and_then(|mut stream| {
            stream.set_read_timeout(Some(ANSWER_TIMEOUT))?;
            stream.write_all(format!("{line}\n").as_bytes())?;
            let mut answer = String::new();
            BufReader::new(stream).read_line(&mut answer)?;
            Ok(answer.trim().to_owned())
        });
        match answer {
            Ok(answer) if answer == "ok" => {
                log::info!("{line}: ok");
                true
            }
            Ok(answer) => {
                log::warn!("{line}: {answer}");
                false
            }
            Err(err) => {
                log::error!("control channel {}: {err}", control.socket);
                false
            }
        }
    }
}

/// Run a command detached from the overlay, which exits right after.
pub fn spawn(argv: &[String]) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let Some((program, arguments)) = argv.split_first() else {
        return;
    };
    if let Err(err) = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
    {
        log::error!("cannot run {program}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(rom: &str, control: &str) -> Session {
        serde_json::from_str(&format!(
            r#"{{"pid": 1, "rom": "{rom}", "control": {control}}}"#
        ))
        .unwrap()
    }

    const RETROARCH: &str = r#"{"socket": "/run/x/control.sock", "pause": true, "slots": true,
        "focus": false, "actions": ["save_state", "load_state", "reset", "disk_eject",
        "disk_next", "disk_prev", "screenshot"]}"#;

    #[test]
    fn menu_keeps_what_applies() {
        let emulator = Emulator::new(&session("/roms/psx/ff7.m3u", RETROARCH), true);
        assert_eq!(
            emulator.actions(),
            [
                EmuAction::DiscTray,
                EmuAction::NextDisc,
                EmuAction::PrevDisc,
                EmuAction::Reset,
                EmuAction::Screenshot
            ]
        );
        assert!(emulator.has_slots() && emulator.pause_game);

        let single = Emulator::new(&session("/roms/psx/game.chd", RETROARCH), false);
        assert!(!single.actions().contains(&EmuAction::NextDisc));
        assert!(!single.pause_game);
    }

    #[test]
    fn plain_states_without_slots() {
        let control = r#"{"socket": "s", "pause": false, "slots": false, "focus": true,
            "actions": ["save_state", "load_state"]}"#;
        let emulator = Emulator::new(&session("/roms/gc/game.rvz", control), true);
        assert_eq!(
            emulator.actions(),
            [EmuAction::SaveState, EmuAction::LoadState]
        );
        assert!(!emulator.pause_game);
    }

    #[test]
    fn requests_as_lines() {
        assert_eq!(Request::Action(EmuAction::NextDisc).line(), "do disk_next");
        assert_eq!(Request::SaveState(3).line(), "save_slot 3");
        assert_eq!(Request::LoadState(2).line(), "load_slot 2");
    }
}
