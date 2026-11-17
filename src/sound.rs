//! The menu's sounds, like EmulationStation's: one to move, one to choose,
//! one to go back (iconic-es-de's, see `assets/sounds/LICENSE`).
//!
//! They are played by the sound server's own player, `pw-play` (`PipeWire`)
//! or `paplay` (`PulseAudio`, and `PipeWire`'s pulse server), at the volume
//! `overlay.ini` sets: no audio library in the overlay, and the sound goes
//! wherever the game's goes. The players read files, so the sounds, built
//! into the program, are written once into the runtime directory. A sound
//! cuts the one before it short (moving fast plays one tick per step), and
//! the last one plays to its end even when the overlay is already gone
//! (choosing "Continue", going back).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::system;

/// What the menu plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// The selection moved.
    Scroll,
    /// Something was chosen.
    Select,
    /// Back, or the menu closed.
    Back,
}

impl Sound {
    const fn file_name(self) -> &'static str {
        match self {
            Self::Scroll => "scroll.wav",
            Self::Select => "select.wav",
            Self::Back => "back.wav",
        }
    }

    const fn data(self) -> &'static [u8] {
        match self {
            Self::Scroll => include_bytes!("../assets/sounds/scroll.wav"),
            Self::Select => include_bytes!("../assets/sounds/select.wav"),
            Self::Back => include_bytes!("../assets/sounds/back.wav"),
        }
    }
}

/// The program that plays them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Player {
    PipeWire,
    Pulse,
}

impl Player {
    fn find() -> Option<Self> {
        if system::installed("pw-play") {
            Some(Self::PipeWire)
        } else if system::installed("paplay") {
            Some(Self::Pulse)
        } else {
            None
        }
    }

    /// The command that plays `file` at `volume` (0 to 1, linear).
    fn command(self, file: &Path, volume: f32) -> Command {
        let mut command = match self {
            Self::PipeWire => {
                let mut command = Command::new("pw-play");
                command.arg(format!("--volume={volume:.2}"));
                command
            }
            Self::Pulse => {
                let mut command = Command::new("paplay");
                // PulseAudio's linear scale: 65536 is 100 %
                command.arg(format!("--volume={}", (volume * 65536.0).round() as u32));
                command
            }
        };
        command
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

/// Plays the menu's sounds.
pub struct Sounds {
    /// None: silent (volume 0, no player, or the player failed).
    player: Option<Player>,
    volume: f32,
    dir: PathBuf,
    /// The sound playing (or played last).
    playing: Option<Child>,
}

impl Sounds {
    /// At `volume` percent (0: silent), written into `dir` when first played.
    pub fn new(volume: u8, dir: PathBuf) -> Self {
        let player = (volume > 0).then(Player::find).flatten();
        if volume > 0 && player.is_none() {
            log::info!("no pw-play nor paplay: the menu is silent");
        }
        Self {
            player,
            volume: f32::from(volume.min(100)) / 100.0,
            dir,
            playing: None,
        }
    }

    /// Play a sound, cutting the one before it short.
    pub fn play(&mut self, sound: Sound) {
        let Some(player) = self.player else {
            return;
        };
        let Some(file) = self.file(sound) else {
            self.player = None;
            return;
        };
        if let Some(mut previous) = self.playing.take() {
            // gone already, or cut short: either way it is reaped
            previous.kill().ok();
            previous.wait().ok();
        }
        match player.command(&file, self.volume).spawn() {
            Ok(child) => self.playing = Some(child),
            Err(err) => {
                log::warn!("cannot play the menu's sounds: {err}");
                self.player = None;
            }
        }
    }

    /// The sound's file, written first when it is not there (or not whole).
    fn file(&self, sound: Sound) -> Option<PathBuf> {
        let file = self.dir.join(sound.file_name());
        let data = sound.data();
        let whole = fs::metadata(&file).is_ok_and(|meta| meta.len() == data.len() as u64);
        if !whole {
            let written = fs::create_dir_all(&self.dir).and_then(|()| {
                // whole or not at all: another overlay may be reading it
                let partial = file.with_extension("wav.part");
                fs::write(&partial, data)?;
                fs::rename(&partial, &file)
            });
            if let Err(err) = written {
                log::warn!("cannot write {}: {err}", file.display());
                return None;
            }
        }
        Some(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sounds_are_wav_files() {
        for sound in [Sound::Scroll, Sound::Select, Sound::Back] {
            let data = sound.data();
            assert_eq!(&data[..4], b"RIFF");
            assert_eq!(&data[8..12], b"WAVE");
        }
    }

    #[test]
    fn writes_each_sound_once() {
        let dir = std::env::temp_dir().join(format!("overlay-sounds-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        let sounds = Sounds {
            player: Some(Player::PipeWire),
            volume: 0.5,
            dir: dir.clone(),
            playing: None,
        };
        let file = sounds.file(Sound::Back).unwrap();
        assert_eq!(fs::read(&file).unwrap(), Sound::Back.data());
        let written = fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        sounds.file(Sound::Back).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), written);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn players_get_the_volume_on_their_scale() {
        let file = Path::new("/x/back.wav");
        let args = |player: Player| {
            player
                .command(file, 0.5)
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(args(Player::PipeWire), ["--volume=0.50", "/x/back.wav"]);
        assert_eq!(args(Player::Pulse), ["--volume=32768", "/x/back.wav"]);
    }
}
