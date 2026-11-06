//! Save state slots: what each holds, for the save and load pages.
//!
//! The slots are the emulator's own `<dir>/<name>.state<N>` files, with the
//! preview it saves next to each (`.state<N>.png`), so EmulationStation's
//! save state list sees the same states. Saving and loading are requests on
//! the control channel (emulator.rs); the launcher keeps track of the
//! emulator's current slot.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::session::SaveStatesInfo;

/// The slots the overlay offers.
pub const SLOTS: [u32; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

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

    fn state_path(&self, number: u32) -> PathBuf {
        Path::new(&self.info.dir).join(format!("{}.state{number}", self.info.name))
    }

    /// The offered slots, with what each holds now.
    pub fn slots(&self) -> Vec<Slot> {
        SLOTS
            .iter()
            .map(|&number| {
                let path = self.state_path(number);
                let saved = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
                let mut preview = path.into_os_string();
                preview.push(".png");
                let preview = PathBuf::from(preview);
                Slot {
                    number,
                    saved,
                    preview: preview.exists().then_some(preview),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_show_states_and_previews() {
        let dir = std::env::temp_dir().join(format!("overlay-states-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Game (Disc 1).state2"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state2.png"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state3"), "x").unwrap();
        let states = SaveStates::new(&SaveStatesInfo {
            dir: dir.to_string_lossy().into_owned(),
            name: "Game (Disc 1)".into(),
        });
        let slots = states.slots();
        assert_eq!(slots.len(), SLOTS.len());
        assert!(slots[0].saved.is_none());
        assert!(slots[1].saved.is_some() && slots[1].preview.is_some());
        assert!(slots[2].saved.is_some() && slots[2].preview.is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
