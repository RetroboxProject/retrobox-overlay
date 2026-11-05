//! Save states by slot, for emulators whose states the overlay can manage
//! (RetroArch, through its command pipe: see `SaveStatesInfo`).
//!
//! The overlay offers a fixed set of slots. Each is the emulator's own
//! `<dir>/<name>.state<N>`, with the preview the emulator saves next to it
//! (`.state<N>.png`), so EmulationStation's save state list sees the same
//! files. Loading names the slot (`LOAD_STATE_SLOT N`); saving can only go to
//! RetroArch's current slot, so the overlay moves it there first
//! (`STATE_SLOT_PLUS`/`MINUS`) and remembers where it left it. Nothing else
//! moves that slot: Retrobox leaves RetroArch's slot hotkeys unbound.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::session::SaveStatesInfo;

/// The slots the overlay offers.
pub const SLOTS: [u32; 4] = [1, 2, 3, 4];

/// Where the overlay remembers RetroArch's current slot between openings.
const SLOT_FILE: &str = "savestate-slot";

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

/// The save states of the running game.
#[derive(Debug, Clone)]
pub struct SaveStates {
    info: SaveStatesInfo,
    /// The game's pid: the remembered slot belongs to this game only.
    pid: i32,
    slot_file: PathBuf,
}

impl SaveStates {
    /// None when the emulator's command pipe is not there.
    pub fn new(info: &SaveStatesInfo, pid: i32, runtime_dir: &Path) -> Option<Self> {
        if !Path::new(&info.control).exists() {
            log::warn!("no command pipe at {}: save states disabled", info.control);
            return None;
        }
        Some(Self {
            info: info.clone(),
            pid,
            slot_file: runtime_dir.join(SLOT_FILE),
        })
    }

    fn state_path(&self, number: u32) -> PathBuf {
        Path::new(&self.info.dir).join(format!("{}.state{number}", self.info.name))
    }

    /// The offered slots, with what each holds.
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

    /// Load the state of a slot.
    pub fn load(&self, number: u32) {
        log::info!("loading the state of slot {number}");
        self.send(&[format!("LOAD_STATE_SLOT {number}")]);
    }

    /// Save the game into a slot.
    pub fn save(&self, number: u32) {
        let current = self.current_slot();
        let target = i64::from(number);
        let step = if target > current {
            "STATE_SLOT_PLUS"
        } else {
            "STATE_SLOT_MINUS"
        };
        let mut commands: Vec<String> = (0..(target - current).unsigned_abs())
            .map(|_| step.to_owned())
            .collect();
        commands.push("SAVE_STATE".to_owned());
        log::info!("saving into slot {number} (slot was {current})");
        if self.send(&commands) {
            self.remember_slot(number);
        }
    }

    /// RetroArch's current slot: where the overlay left it in this game, or
    /// the one the game started with.
    fn current_slot(&self) -> i64 {
        fs::read_to_string(&self.slot_file)
            .ok()
            .and_then(|text| {
                let (pid, slot) = text.trim().split_once(' ')?;
                (pid.parse::<i32>().ok()? == self.pid).then(|| slot.parse::<i64>().ok())?
            })
            .unwrap_or_else(|| i64::from(self.info.slot))
    }

    fn remember_slot(&self, number: u32) {
        if let Err(err) = fs::write(&self.slot_file, format!("{} {number}\n", self.pid)) {
            log::warn!("cannot write {}: {err}", self.slot_file.display());
        }
    }

    /// Write commands to the emulator's pipe, one per line; false on failure.
    fn send(&self, commands: &[String]) -> bool {
        let mut text = commands.join("\n");
        text.push('\n');
        let written = OpenOptions::new()
            .write(true)
            .custom_flags(nix::fcntl::OFlag::O_NONBLOCK.bits())
            .open(&self.info.control)
            .and_then(|mut pipe| pipe.write_all(text.as_bytes()));
        if let Err(err) = &written {
            log::error!("cannot write to {}: {err}", self.info.control);
        }
        written.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (PathBuf, SaveStates) {
        let dir =
            std::env::temp_dir().join(format!("overlay-states-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let control = dir.join("cmd");
        fs::write(&control, "").unwrap(); // a plain file stands in for the pipe
        let info = SaveStatesInfo {
            control: control.to_string_lossy().into_owned(),
            dir: dir.to_string_lossy().into_owned(),
            name: "Game (Disc 1)".into(),
            slot: 0,
        };
        let states = SaveStates::new(&info, 42, &dir).unwrap();
        (dir, states)
    }

    #[test]
    fn slots_show_states_and_previews() {
        let (dir, states) = fixture("slots");
        fs::write(dir.join("Game (Disc 1).state2"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state2.png"), "x").unwrap();
        fs::write(dir.join("Game (Disc 1).state3"), "x").unwrap();
        let slots = states.slots();
        assert_eq!(slots.len(), 4);
        assert!(slots[0].saved.is_none());
        assert!(slots[1].saved.is_some() && slots[1].preview.is_some());
        assert!(slots[2].saved.is_some() && slots[2].preview.is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_walks_the_slot_and_remembers_it() {
        let (dir, states) = fixture("save");
        let control = dir.join("cmd");
        states.save(3);
        assert_eq!(
            fs::read_to_string(&control).unwrap(),
            "STATE_SLOT_PLUS\nSTATE_SLOT_PLUS\nSTATE_SLOT_PLUS\nSAVE_STATE\n"
        );
        assert_eq!(states.current_slot(), 3);
        fs::write(&control, "").unwrap();
        states.save(1);
        assert_eq!(
            fs::read_to_string(&control).unwrap(),
            "STATE_SLOT_MINUS\nSTATE_SLOT_MINUS\nSAVE_STATE\n"
        );
        // another game's remembered slot is not ours
        fs::write(dir.join(SLOT_FILE), "7 4\n").unwrap();
        assert_eq!(states.current_slot(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}
