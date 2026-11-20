//! Toasts: a short notice at the top of the screen, on the menu's side,
//! over whatever runs (a game, EmulationStation), that takes neither the
//! keyboard nor the controllers and goes away on its own. Any program can
//! show one (`retrobox-overlay --toast`):
//!
//! ```text
//! retrobox-overlay --toast --title TEXT [--text TEXT] [--icon NAME]
//!                  [--sound NAME|none] [--seconds N]
//! retrobox-overlay --toast break
//! ```
//!
//! The icon is one of the overlay's Lucide icons, by name (`trophy`,
//! `coffee`, `wifi-off`...); the sound one of the menu's (`select`, the
//! default, `back`, `scroll`) or `none`. The texts are shown as given: the
//! caller translates them. A preset is a toast the overlay writes itself:
//! `break`, the break reminder (the input daemon asks for it, see
//! `retrobox_input::breaks`).
//!
//! Toasts never overlap: one waits for the one before it to go (a lock in
//! the runtime directory), so a burst of them shows one after another.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use nix::fcntl::{Flock, FlockArg};

use crate::sound::Sound;
use crate::ui::icons::Icon;
use crate::volume::{self, Level};

/// How long a toast stays when nothing says it, and at most.
const DEFAULT_SECONDS: u64 = 6;
const MAX_SECONDS: u64 = 60;
/// The break reminder's: a little longer, it has more to read.
const BREAK_SECONDS: u64 = 8;
const LOCK_FILE: &str = "toast.lock";

/// What a toast shows, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub icon: Icon,
    pub title: String,
    pub text: String,
    /// Played as it shows (None: silent).
    pub sound: Option<Sound>,
    /// How long it stays, faded in.
    pub shown: Duration,
    /// The volume, as a bar in place of the texts (the volume keys' toast,
    /// see `volume`).
    pub level: Option<Level>,
}

/// What `--toast` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The break reminder, written here (it needs the game's play time and
    /// the language).
    Break,
    /// A toast as given.
    Given(Toast),
}

impl Request {
    /// The request of the arguments after `--toast`.
    pub fn parse(args: &[String]) -> Result<Self> {
        if let [preset] = args
            && !preset.starts_with("--")
        {
            return match preset.as_str() {
                "break" => Ok(Self::Break),
                other => Err(anyhow!("unknown toast {other:?} (break)")),
            };
        }
        let mut toast = Toast {
            icon: Icon::Info,
            title: String::new(),
            text: String::new(),
            sound: Some(Sound::Select),
            shown: Duration::from_secs(DEFAULT_SECONDS),
            level: None,
        };
        let mut args = args.iter();
        while let Some(option) = args.next() {
            let value = args
                .next()
                .with_context(|| format!("{option} needs a value"))?;
            match option.as_str() {
                "--title" => toast.title.clone_from(value),
                "--text" => toast.text.clone_from(value),
                "--icon" => {
                    toast.icon =
                        Icon::named(value).with_context(|| format!("no icon {value:?}"))?;
                }
                "--sound" if value == "none" => toast.sound = None,
                "--sound" => {
                    toast.sound =
                        Some(Sound::named(value).with_context(|| format!("no sound {value:?}"))?);
                }
                "--seconds" => {
                    let seconds: u64 = value
                        .parse()
                        .with_context(|| format!("--seconds: not a number: {value:?}"))?;
                    toast.shown = Duration::from_secs(seconds.clamp(1, MAX_SECONDS));
                }
                other => bail!("unknown option {other}"),
            }
        }
        if toast.title.trim().is_empty() {
            bail!("a toast needs a --title");
        }
        Ok(Self::Given(toast))
    }
}

impl Toast {
    /// The break reminder: `title`, and `text` saying how long the game has
    /// been played.
    pub fn break_reminder(title: String, text: String) -> Self {
        Self {
            icon: Icon::Coffee,
            title,
            text,
            sound: Some(Sound::Select),
            shown: Duration::from_secs(BREAK_SECONDS),
            level: None,
        }
    }

    /// The volume keys' toast, silent: the volume's icon and a bar (see
    /// `volume`).
    pub fn volume(level: Level) -> Self {
        Self {
            icon: Self::volume_icon(level),
            title: String::new(),
            text: String::new(),
            sound: None,
            shown: volume::STAYS,
            level: Some(level),
        }
    }

    /// The same toast at another volume.
    pub fn set_level(&mut self, level: Level) {
        self.icon = Self::volume_icon(level);
        self.level = Some(level);
    }

    fn volume_icon(level: Level) -> Icon {
        if level.muted {
            Icon::VolumeX
        } else {
            Icon::volume(level.value())
        }
    }
}

/// Wait for the toasts before this one to go: the turn lasts as long as
/// what this returns is kept.
pub fn wait_turn(runtime_dir: &Path) -> Result<Flock<File>> {
    let path = runtime_dir.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    Flock::lock(file, FlockArg::LockExclusive)
        .map_err(|(_, err)| anyhow!("cannot lock {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn a_preset_or_a_toast_as_given() {
        assert_eq!(Request::parse(&args(&["break"])).unwrap(), Request::Break);
        assert!(Request::parse(&args(&["party"])).is_err());
        let given = Request::parse(&args(&[
            "--title",
            "Achievement",
            "--text",
            "First blood",
            "--icon",
            "trophy",
            "--sound",
            "none",
            "--seconds",
            "90",
        ]))
        .unwrap();
        assert_eq!(
            given,
            Request::Given(Toast {
                icon: Icon::Trophy,
                title: "Achievement".to_owned(),
                text: "First blood".to_owned(),
                sound: None,
                shown: Duration::from_secs(MAX_SECONDS),
                level: None,
            })
        );
    }

    #[test]
    fn defaults_and_mistakes() {
        let Request::Given(toast) = Request::parse(&args(&["--title", "Hi"])).unwrap() else {
            panic!("not a given toast");
        };
        assert_eq!(toast.icon, Icon::Info);
        assert_eq!(toast.sound, Some(Sound::Select));
        assert_eq!(toast.shown, Duration::from_secs(DEFAULT_SECONDS));
        for wrong in [
            &["--text", "no title"][..],
            &["--title"],
            &["--title", "x", "--icon", "nope"],
            &["--title", "x", "--sound", "boom"],
            &["--title", "x", "--seconds", "soon"],
            &["--title", "x", "--colour", "red"],
        ] {
            assert!(Request::parse(&args(wrong)).is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn toasts_take_turns() {
        let dir = std::env::temp_dir().join(format!("retrobox-toast-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let turn = wait_turn(&dir).unwrap();
        let path = dir.join(LOCK_FILE);
        let other = File::open(&path).unwrap();
        // while the first is shown, the next one waits
        assert!(Flock::lock(other, FlockArg::LockExclusiveNonblock).is_err());
        drop(turn);
        let other = File::open(&path).unwrap();
        assert!(Flock::lock(other, FlockArg::LockExclusiveNonblock).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }
}
