//! Save state slots: what each holds, for the save and load pages, and the
//! automatic one "Save and quit" saves in.
//!
//! The slots are RetroArch's own files, `<dir>/<name>.state` for slot 0 and
//! `.state<N>` for the others, with the preview it saves next to each
//! (`.png`), so EmulationStation's save state list sees the same states.
//! "Save and quit" saves in the automatic slot (-1, `.state.auto`), which
//! EmulationStation lists as the auto save and RetroArch loads on start when
//! the game's auto save is on; quitting without saving forgets it. Saving
//! and loading are requests on the control channel (emulator.rs); the
//! launcher keeps track of the emulator's current slot.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

use crate::session::SaveStatesInfo;

/// The slots the save and load pages offer.
pub const SLOTS: [u32; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
/// The slot "Save and quit" saves in: RetroArch's automatic one.
pub const AUTO_SLOT: i32 = -1;
/// How often, and how long at most, "Save and quit" looks for its state.
const SAVE_POLL: Duration = Duration::from_millis(50);
const SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Save or load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateMode {
    Save,
    Load,
}

/// One slot as the menu shows it.
#[derive(Debug, Clone)]
pub struct Slot {
    pub number: u32,
    /// When the state was saved; None for an empty slot.
    pub saved: Option<SystemTime>,
    pub preview: Option<PathBuf>,
}

/// Where the running game's states are.
#[derive(Debug, Clone)]
pub struct SaveStates {
    info: SaveStatesInfo,
}

impl SaveStates {
    pub fn new(info: &SaveStatesInfo) -> Self {
        Self { info: info.clone() }
    }

    /// A slot's file, as RetroArch names it.
    fn state_path(&self, number: i32) -> PathBuf {
        let suffix = match number {
            AUTO_SLOT => ".auto".to_owned(),
            0 => String::new(),
            number => number.to_string(),
        };
        Path::new(&self.info.dir).join(format!("{}.state{suffix}", self.info.name))
    }

    /// The offered slots, with what each holds now.
    pub fn slots(&self) -> Vec<Slot> {
        SLOTS
            .iter()
            .map(|&number| {
                let path = self.state_path(number.cast_signed());
                let saved = modified(&path);
                let preview = preview_path(&path);
                Slot {
                    number,
                    saved,
                    preview: preview.exists().then_some(preview),
                }
            })
            .collect()
    }

    /// When the automatic state was saved, if there is one.
    pub fn auto_saved(&self) -> Option<SystemTime> {
        modified(&self.state_path(AUTO_SLOT))
    }

    /// Wait until the emulator has written the automatic state (saved after
    /// `before`, and done growing); false when it does not in time.
    pub fn wait_for_auto_save(&self, before: Option<SystemTime>) -> bool {
        let path = self.state_path(AUTO_SLOT);
        let deadline = Instant::now() + SAVE_TIMEOUT;
        let mut last_size = None;
        while Instant::now() < deadline {
            sleep(SAVE_POLL);
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            let newer = meta
                .modified()
                .is_ok_and(|saved| before.is_none_or(|before| saved > before));
            if newer && last_size == Some(meta.len()) {
                return true;
            }
            last_size = newer.then_some(meta.len());
        }
        false
    }

    /// Forget the automatic state, and its preview (quitting without
    /// saving).
    pub fn forget_auto(&self) {
        let path = self.state_path(AUTO_SLOT);
        for file in [preview_path(&path), path] {
            match fs::remove_file(&file) {
                Ok(()) => log::info!("forgot {}", file.display()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => log::warn!("cannot remove {}: {err}", file.display()),
            }
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// The preview saved next to a state (`.state3.png`).
fn preview_path(state: &Path) -> PathBuf {
    let mut preview = state.as_os_str().to_owned();
    preview.push(".png");
    PathBuf::from(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_show_states_and_previews() {
        let dir = std::env::temp_dir().join(format!("overlay-states-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Game (Disc 1).state"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state1"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state1.png"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state.auto"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state.auto.png"), "x").unwrap();
        let states = SaveStates::new(&SaveStatesInfo {
            dir: dir.to_string_lossy().into_owned(),
            name: "Game (Disc 1)".into(),
        });
        let slots = states.slots();
        assert_eq!(slots.len(), SLOTS.len());
        // slot 0 is `.state`, without a number
        assert!(slots[0].saved.is_some() && slots[0].preview.is_none());
        assert!(slots[1].saved.is_some() && slots[1].preview.is_some());
        assert!(slots[2].saved.is_none());
        assert!(states.auto_saved().is_some());
        states.forget_auto();
        assert!(states.auto_saved().is_none());
        assert!(!dir.join("Game (Disc 1).state.auto.png").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn waits_for_the_automatic_state() {
        let dir = std::env::temp_dir().join(format!("overlay-auto-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let states = SaveStates::new(&SaveStatesInfo {
            dir: dir.to_string_lossy().into_owned(),
            name: "Game".into(),
        });
        let file = dir.join("Game.state.auto");
        let writer = std::thread::spawn(move || {
            sleep(Duration::from_millis(100));
            fs::write(file, "state").unwrap();
        });
        assert!(states.wait_for_auto_save(None));
        writer.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
