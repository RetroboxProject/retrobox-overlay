//! The hotkey's shield, for the emulators that do not keep the hotkey's
//! shortcuts from the game themselves.
//!
//! This process reads the controllers without taking them, so the buttons
//! of a shortcut reach the game too: Hotkey+Start, which quits, presses
//! Start in the game. RetroArch blocks the game's input while its own
//! hotkey is held (the launcher has it block at once), and
//! EmulationStation does the same (a patch of Retrobox's); the other
//! emulators do not, and there the shield does it: while a standalone
//! emulator runs, the dedicated hotkey (Home/Guide) going down alone takes
//! the controller (EVIOCGRAB) until it goes up, so the shortcut's buttons
//! come here only (the mapper's shortcuts still work: they read them here).
//!
//! The emulator saw the hotkey go down, before the shield, but not go up:
//! when it goes up, the shield replays it going down and up (written to the
//! controller, which hands the events to every reader), which leaves the
//! emulator as it would have been without the shield; this process skips
//! the replay. The shield only goes up with nothing else pressed or pushed,
//! since their releases would not reach the emulator either; and the
//! overlay, which takes the controllers itself, makes it give them back
//! when it opens.

use std::path::Path;

use evdev::{Device, EventType, InputEvent, KeyCode, SynchronizationCode};
use serde_json::Value;

/// The emulator of RetroArch's games, which blocks the hotkey's shortcuts.
const RETROARCH: &str = "libretro";

/// One controller's shield.
#[derive(Debug, Default)]
pub struct Shield {
    /// The controller is taken.
    up: bool,
    /// Replayed hotkey events still to skip.
    replayed: u8,
}

impl Shield {
    /// Take the controller (its hotkey just went down, alone).
    pub fn raise(&mut self, device: &mut Device) {
        if self.up {
            return;
        }
        match device.grab() {
            Ok(()) => self.up = true,
            Err(err) => log::debug!("cannot shield the hotkey: {err}"),
        }
    }

    /// Give the controller back. `released`: the hotkey went up meanwhile,
    /// which the emulator did not see: replay it.
    pub fn lower(&mut self, device: &mut Device, hotkey: KeyCode, released: bool) {
        if !self.up {
            return;
        }
        self.up = false;
        if let Err(err) = device.ungrab() {
            log::debug!("cannot give the controller back: {err}");
        }
        if !released {
            return;
        }
        let key = |value| InputEvent::new(EventType::KEY.0, hotkey.code(), value);
        let report = InputEvent::new(
            EventType::SYNCHRONIZATION.0,
            SynchronizationCode::SYN_REPORT.0,
            0,
        );
        match device.send_events(&[key(1), report, key(0), report]) {
            Ok(()) => self.replayed = 2,
            // read-only (no write access to the controller): the emulator
            // goes on thinking the hotkey is held
            Err(err) => log::debug!("cannot replay the hotkey: {err}"),
        }
    }

    /// Whether `event` is the replay, which this process skips.
    pub fn skips(&mut self, event: &InputEvent, hotkey: KeyCode) -> bool {
        let replayed = self.replayed > 0
            && event.event_type() == EventType::KEY
            && event.code() == hotkey.code();
        if replayed {
            self.replayed -= 1;
        }
        replayed
    }
}

/// Whether the running game's emulator needs the shield (any but
/// RetroArch's); none outside games (EmulationStation needs none).
pub fn wanted(runtime_dir: &Path) -> bool {
    let emulator = std::fs::read_to_string(runtime_dir.join("session.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|session| session.get("emulator")?.as_str().map(str::to_owned));
    emulator.is_some_and(|emulator| emulator != RETROARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_emulators_want_it() {
        let dir = std::env::temp_dir().join(format!("shield-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!wanted(&dir));
        std::fs::write(dir.join("session.json"), r#"{"emulator": "libretro"}"#).unwrap();
        assert!(!wanted(&dir));
        std::fs::write(dir.join("session.json"), r#"{"emulator": "pcsx2"}"#).unwrap();
        assert!(wanted(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_its_replay_only() {
        let mut shield = Shield {
            up: false,
            replayed: 2,
        };
        let key = |code: KeyCode, value| InputEvent::new(EventType::KEY.0, code.code(), value);
        assert!(!shield.skips(&key(KeyCode::BTN_SOUTH, 1), KeyCode::BTN_MODE));
        assert!(shield.skips(&key(KeyCode::BTN_MODE, 1), KeyCode::BTN_MODE));
        assert!(shield.skips(&key(KeyCode::BTN_MODE, 0), KeyCode::BTN_MODE));
        assert!(!shield.skips(&key(KeyCode::BTN_MODE, 1), KeyCode::BTN_MODE));
    }
}
