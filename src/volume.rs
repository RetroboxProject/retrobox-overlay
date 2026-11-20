//! The volume keys (`retrobox-overlay --volume up|down|mute`, what the
//! session's key bindings run): each press steps the default output's
//! volume by 5 % (never above 100 %) or mutes and unmutes it, through the
//! sound server's tool (`wpctl` or `pactl`, see `system::audio`), and, when
//! `overlay.ini` asks for it (`[volume] toast`), shows it: a toast with the
//! volume's icon and a bar, at the top of the screen on the menu's side,
//! that stays until 3 seconds after the last press and takes no input at
//! all (not while the menu is open: its own slider shows the volume).
//!
//! One toast shows every press: the first press shows it, the next ones
//! tell it their volume (a datagram on a socket in the runtime directory)
//! and it redraws its bar. Which press shows it is settled by a lock that
//! the toast holds as long as it runs; the presses themselves take turns
//! (another lock), so that two of them close together never step from the
//! same volume and the toast gets them in order.

use std::fs::{File, OpenOptions};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use nix::fcntl::{Flock, FlockArg};

use crate::system::audio::Mixer;

/// How much a press steps the volume, in percent (EmulationStation's step).
const STEP: u8 = 5;
/// How long the toast stays after the last press.
pub const STAYS: Duration = Duration::from_secs(3);
/// The toast's lock and socket, and the presses' lock, in the runtime
/// directory.
const TOAST_LOCK: &str = "volume-toast.lock";
const SOCKET: &str = "volume-toast.sock";
const PRESS_LOCK: &str = "volume-press.lock";
/// How long a press tries to reach the toast (one starting or ending), and
/// how often.
const HAND_OVER: Duration = Duration::from_secs(1);
const RETRY: Duration = Duration::from_millis(20);

/// A volume key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Mute,
}

impl Key {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "mute" => Some(Self::Mute),
            _ => None,
        }
    }
}

/// The volume a toast shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub percent: u8,
    pub muted: bool,
}

impl Level {
    /// 0 to 1.
    pub fn value(self) -> f32 {
        f32::from(self.percent) / 100.0
    }

    /// As a press sends it: `45 0` (percent, muted).
    fn encode(self) -> String {
        format!("{} {}", self.percent, u8::from(self.muted))
    }

    pub fn decode(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace();
        let percent: u8 = words.next()?.parse().ok()?;
        let muted = match words.next()? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        Some(Self {
            percent: percent.min(100),
            muted,
        })
    }
}

/// The volume after a step from `percent`: the next multiple of `STEP` up
/// or down (a volume set elsewhere falls back in step), within 0 to 100.
fn stepped(percent: u8, up: bool) -> u8 {
    let percent = percent.min(100);
    if up {
        (percent / STEP + 1).saturating_mul(STEP).min(100)
    } else {
        percent
            .div_ceil(STEP)
            .saturating_sub(1)
            .saturating_mul(STEP)
    }
}

/// Press a key: the volume changed, and what it is now. Presses take turns
/// (`turn`, held until the toast has been told).
pub fn press(key: Key, runtime_dir: &Path) -> Result<(Level, Flock<File>)> {
    let turn = lock(&runtime_dir.join(PRESS_LOCK), FlockArg::LockExclusive)?;
    let mut mixer = Mixer::detect().context("no PipeWire or PulseAudio output")?;
    let (value, muted) = mixer.read().context("cannot read the volume")?;
    mixer.knows_muted(muted);
    let percent = (value * 100.0).round() as u8;
    let level = match key {
        Key::Mute => {
            mixer.set_mute(!muted);
            Level {
                percent,
                muted: !muted,
            }
        }
        Key::Up | Key::Down => {
            let percent = stepped(percent, key == Key::Up);
            // (unmuted, but at 0)
            mixer.set(f32::from(percent) / 100.0);
            Level {
                percent,
                muted: percent == 0,
            }
        }
    };
    Ok((level, turn))
}

/// Who shows a press.
pub enum Turn {
    /// This process: the toast, with the socket the next presses come on;
    /// keep the lock while it shows.
    Toast(Toast),
    /// The toast on screen, told.
    Told,
}

/// The toast's lock and socket.
pub struct Toast {
    lock: Flock<File>,
    pub socket: UnixDatagram,
    path: PathBuf,
}

impl Toast {
    /// The toast is gone: the next press shows a new one (the socket goes
    /// first, then the lock).
    pub fn end(self) {
        std::fs::remove_file(&self.path).ok();
        drop(self.socket);
        drop(self.lock);
    }
}

/// Tell the toast on screen, or become it.
pub fn hand_over(level: Level, runtime_dir: &Path) -> Result<Turn> {
    let path = runtime_dir.join(SOCKET);
    let message = level.encode();
    let sender = UnixDatagram::unbound().context("cannot create a socket")?;
    let tries = HAND_OVER.as_millis() / RETRY.as_millis();
    for _ in 0..tries {
        if let Ok(lock) = lock(
            &runtime_dir.join(TOAST_LOCK),
            FlockArg::LockExclusiveNonblock,
        ) {
            // a toast that ended badly may have left its socket
            std::fs::remove_file(&path).ok();
            let socket = UnixDatagram::bind(&path)
                .with_context(|| format!("cannot listen on {}", path.display()))?;
            socket
                .set_nonblocking(true)
                .context("cannot make the socket non-blocking")?;
            return Ok(Turn::Toast(Toast { lock, socket, path }));
        }
        // the toast runs: it listens, unless it is just starting or ending
        if sender.send_to(message.as_bytes(), &path).is_ok() {
            return Ok(Turn::Told);
        }
        std::thread::sleep(RETRY);
    }
    bail!("the volume toast does not answer")
}

/// The latest volume the presses sent, if any (the socket is read empty).
pub fn latest(socket: &UnixDatagram) -> Option<Level> {
    let mut buffer = [0_u8; 32];
    let mut latest = None;
    while let Ok(length) = socket.recv(&mut buffer) {
        if let Some(level) = std::str::from_utf8(&buffer[..length])
            .ok()
            .and_then(Level::decode)
        {
            latest = Some(level);
        }
    }
    latest
}

fn lock(path: &Path, how: FlockArg) -> Result<Flock<File>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    Flock::lock(file, how).map_err(|(_, err)| anyhow!("cannot lock {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_of_five_within_the_range() {
        assert_eq!(stepped(40, true), 45);
        assert_eq!(stepped(40, false), 35);
        assert_eq!(stepped(42, true), 45);
        assert_eq!(stepped(42, false), 40);
        assert_eq!(stepped(98, true), 100);
        assert_eq!(stepped(100, true), 100);
        assert_eq!(stepped(3, false), 0);
        assert_eq!(stepped(0, false), 0);
        assert_eq!(stepped(0, true), 5);
    }

    #[test]
    fn levels_go_through_the_socket_as_text() {
        let level = Level {
            percent: 45,
            muted: true,
        };
        assert_eq!(Level::decode(&level.encode()), Some(level));
        assert_eq!(Level::decode("150 0").map(|level| level.percent), Some(100));
        assert_eq!(Level::decode("45"), None);
        assert_eq!(Level::decode("45 2"), None);
        assert_eq!(Key::parse("mute"), Some(Key::Mute));
        assert_eq!(Key::parse("louder"), None);
    }

    #[test]
    fn one_toast_takes_every_press() {
        let dir = std::env::temp_dir().join(format!("volume-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let level = |percent| Level {
            percent,
            muted: false,
        };
        let Turn::Toast(toast) = hand_over(level(10), &dir).unwrap() else {
            panic!("the first press shows the toast");
        };
        assert!(matches!(hand_over(level(15), &dir).unwrap(), Turn::Told));
        assert!(matches!(hand_over(level(20), &dir).unwrap(), Turn::Told));
        assert_eq!(latest(&toast.socket), Some(level(20)));
        assert_eq!(latest(&toast.socket), None);
        toast.end();
        assert!(matches!(
            hand_over(level(25), &dir).unwrap(),
            Turn::Toast(_)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }
}
