//! The menu's sounds, like EmulationStation's: one to move, one to choose,
//! one to go back (iconic-es-de's, see `assets/sounds/LICENSE`).
//!
//! They go through one stream that lives as long as the overlay: the sound
//! server's own player, `pw-cat` (`PipeWire`) or `pacat` (`PulseAudio`, and
//! `PipeWire`'s pulse server), started once and fed raw samples on its
//! standard input, at the volume `overlay.ini` sets: no audio library in
//! the overlay, and the sound goes wherever the game's goes. A player per
//! sound would add a client and a node to the sound server's graph at every
//! step through the menu, next to the game's stream, and take it away a few
//! milliseconds later: the game's audio (and the emulators that pace their
//! frames by it) would stutter.
//!
//! A thread keeps the player's pipe a little ahead (`LEAD`): the sound
//! playing, else silence, so the player never waits for data in the sound
//! server's real-time cycle. A sound cuts the one before it short (moving
//! fast plays one tick per step), and the last one plays to its end even
//! when the overlay is already gone (choosing "Continue", going back): it
//! is written whole, the pipe closed, and the player drains it on its own.

use std::io::Write;
use std::os::fd::AsFd;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::system;

/// The sounds' format (all three alike, see the tests): 16-bit stereo.
const RATE: u32 = 44_100;
const CHANNELS: u32 = 2;
const FRAME_BYTES: usize = 4;

/// How far ahead of the player the pipe is kept, and how often it is
/// topped up.
const LEAD: Duration = Duration::from_millis(60);
const TICK: Duration = Duration::from_millis(15);

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
    const fn wav(self) -> &'static [u8] {
        match self {
            Self::Scroll => include_bytes!("../assets/sounds/scroll.wav"),
            Self::Select => include_bytes!("../assets/sounds/select.wav"),
            Self::Back => include_bytes!("../assets/sounds/back.wav"),
        }
    }

    /// The samples: the WAV file's `data` chunk (empty when there is none).
    fn samples(self) -> &'static [u8] {
        wav_data(self.wav()).unwrap_or_default()
    }
}

/// The `data` chunk of a RIFF/WAVE file.
fn wav_data(wav: &[u8]) -> Option<&[u8]> {
    let mut rest = wav.strip_prefix(b"RIFF")?.get(4..)?.strip_prefix(b"WAVE")?;
    while rest.len() >= 8 {
        let (id, size) = rest.split_at(4);
        let size = u32::from_le_bytes(size[..4].try_into().ok()?) as usize;
        let body = &rest[8..];
        if id == b"data" {
            return body.get(..size.min(body.len()));
        }
        // chunks are padded to an even size
        rest = body.get(size + (size & 1)..)?;
    }
    None
}

/// The program that plays them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Player {
    PipeWire,
    Pulse,
}

impl Player {
    fn find() -> Option<Self> {
        if system::installed("pw-cat") {
            Some(Self::PipeWire)
        } else if system::installed("pacat") {
            Some(Self::Pulse)
        } else {
            None
        }
    }

    /// The command that plays raw samples from its standard input at
    /// `volume` (0 to 1, linear).
    fn command(self, volume: f32) -> Command {
        let (program, args) = match self {
            Self::PipeWire => (
                "pw-cat",
                vec![
                    "--playback".to_owned(),
                    format!("--rate={RATE}"),
                    format!("--channels={CHANNELS}"),
                    "--format=s16".to_owned(),
                    format!("--volume={volume:.2}"),
                    "-".to_owned(),
                ],
            ),
            Self::Pulse => (
                "pacat",
                vec![
                    "--playback".to_owned(),
                    "--raw".to_owned(),
                    format!("--rate={RATE}"),
                    format!("--channels={CHANNELS}"),
                    "--format=s16le".to_owned(),
                    // its default buffer is two seconds long
                    "--latency-msec=100".to_owned(),
                    // PulseAudio's linear scale: 65536 is 100 %
                    format!("--volume={}", (volume * 65536.0).round() as u32),
                    "--client-name=retrobox-overlay".to_owned(),
                ],
            ),
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

/// What the feeding thread is asked.
enum Order {
    Play(Sound),
    /// The overlay is closing: the sound playing to its end, then the pipe
    /// closed.
    Finish,
}

/// Plays the menu's sounds.
pub struct Sounds {
    /// None: silent (volume 0, no player, or it failed).
    feeder: Option<(Sender<Order>, JoinHandle<()>)>,
}

impl Sounds {
    /// At `volume` percent (0: silent): the player starts now.
    pub fn new(volume: u8) -> Self {
        let feeder = (volume > 0).then(|| start(volume)).flatten();
        Self { feeder }
    }

    /// Play a sound, cutting the one before it short.
    pub fn play(&mut self, sound: Sound) {
        let sent = self
            .feeder
            .as_ref()
            .is_some_and(|(orders, _)| orders.send(Order::Play(sound)).is_ok());
        if !sent {
            self.feeder = None;
        }
    }
}

impl Drop for Sounds {
    fn drop(&mut self) {
        if let Some((orders, thread)) = self.feeder.take() {
            orders.send(Order::Finish).ok();
            thread.join().ok();
        }
    }
}

/// The player, and the thread that feeds it.
fn start(volume: u8) -> Option<(Sender<Order>, JoinHandle<()>)> {
    let Some(player) = Player::find() else {
        log::info!("no pw-cat nor pacat: the menu is silent");
        return None;
    };
    let volume = f32::from(volume.min(100)) / 100.0;
    let mut child = match player.command(volume).spawn() {
        Ok(child) => child,
        Err(err) => {
            log::warn!("cannot play the menu's sounds: {err}");
            return None;
        }
    };
    let stdin = child.stdin.take()?;
    // room for the lead and a whole sound: the last one is written at once
    let longest = [Sound::Scroll, Sound::Select, Sound::Back]
        .iter()
        .map(|sound| sound.samples().len())
        .max()
        .unwrap_or(0);
    if let Err(err) = rustix::pipe::fcntl_setpipe_size(stdin.as_fd(), longest + 2 * lead_bytes()) {
        log::debug!("the sounds' pipe keeps its size: {err}");
    }
    let (orders, received) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("sounds".to_owned())
        .spawn(move || {
            feed(stdin, &received);
            // the player drains what it has and ends on its own; init reaps it
            drop(child);
        });
    match thread {
        Ok(thread) => Some((orders, thread)),
        Err(err) => {
            log::warn!("cannot play the menu's sounds: {err}");
            None
        }
    }
}

/// `LEAD` in bytes, whole frames.
const fn lead_bytes() -> usize {
    (RATE as usize * LEAD.as_millis() as usize / 1000) * FRAME_BYTES
}

/// Keep the player's pipe `LEAD` ahead until the overlay closes (or the
/// player goes away).
fn feed(mut pipe: ChildStdin, orders: &Receiver<Order>) {
    let mut playing = Playing::default();
    loop {
        match orders.recv_timeout(TICK) {
            Ok(Order::Play(sound)) => playing = Playing::new(sound),
            Ok(Order::Finish) | Err(RecvTimeoutError::Disconnected) => {
                pipe.write_all(playing.rest()).ok();
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let buffered = rustix::io::ioctl_fionread(pipe.as_fd()).unwrap_or(0);
        let buffered = usize::try_from(buffered).unwrap_or(usize::MAX);
        let wanted = lead_bytes().saturating_sub(buffered);
        if pipe.write_all(&playing.next(wanted)).is_err() {
            log::warn!("the menu's sound player is gone: silent from now on");
            return;
        }
    }
}

/// The sound being written, and how far.
#[derive(Debug, Default)]
struct Playing {
    samples: &'static [u8],
    at: usize,
}

impl Playing {
    fn new(sound: Sound) -> Self {
        Self {
            samples: sound.samples(),
            at: 0,
        }
    }

    /// The next `bytes` (whole frames): the sound, then silence.
    fn next(&mut self, bytes: usize) -> Vec<u8> {
        let bytes = bytes - bytes % FRAME_BYTES;
        let sound = self.rest();
        let taken = sound.len().min(bytes);
        self.at += taken;
        let mut chunk = sound[..taken].to_vec();
        chunk.resize(bytes, 0);
        chunk
    }

    /// What is left of the sound.
    fn rest(&self) -> &'static [u8] {
        &self.samples[self.at..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOUNDS: [Sound; 3] = [Sound::Scroll, Sound::Select, Sound::Back];

    /// A little-endian field of a WAV file.
    fn field(wav: &[u8], at: usize, len: usize) -> u32 {
        wav[at..at + len]
            .iter()
            .rev()
            .fold(0, |value, &byte| value << 8 | u32::from(byte))
    }

    #[test]
    fn the_sounds_are_16_bit_stereo_at_the_stream_s_rate() {
        for sound in SOUNDS {
            let wav = sound.wav();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(field(wav, 20, 2), 1, "PCM");
            assert_eq!(field(wav, 22, 2), CHANNELS);
            assert_eq!(field(wav, 24, 4), RATE);
            assert_eq!(field(wav, 34, 2), 16);
            let samples = sound.samples();
            assert_ne!(samples.len(), 0);
            assert_eq!(samples.len() % FRAME_BYTES, 0);
        }
    }

    #[test]
    fn a_sound_then_silence() {
        let mut playing = Playing {
            samples: &[1, 2, 3, 4, 5, 6, 7, 8],
            at: 0,
        };
        assert_eq!(playing.next(6), [1, 2, 3, 4]);
        assert_eq!(playing.rest(), [5, 6, 7, 8]);
        assert_eq!(playing.next(12), [5, 6, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(playing.next(4), [0, 0, 0, 0]);
        assert_eq!(playing.rest().len(), 0);
        assert_eq!(Playing::default().next(8), [0; 8]);
    }

    #[test]
    fn wav_data_skips_the_other_chunks() {
        let wav = [
            b"RIFF".as_slice(),
            &[0; 4],
            b"WAVE",
            b"LIST",
            &3_u32.to_le_bytes(),
            &[9, 9, 9, 0],
            b"data",
            &4_u32.to_le_bytes(),
            &[1, 2, 3, 4],
        ]
        .concat();
        assert_eq!(wav_data(&wav), Some([1, 2, 3, 4].as_slice()));
        assert_eq!(wav_data(b"RIFF1234WAVE"), None);
        assert_eq!(wav_data(b"not a wav"), None);
    }

    #[test]
    fn players_get_the_format_and_the_volume_on_their_scale() {
        let args = |player: Player| {
            player
                .command(0.5)
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let pipewire = args(Player::PipeWire);
        assert!(pipewire.iter().any(|arg| arg == "--volume=0.50"));
        assert!(pipewire.iter().any(|arg| arg == "--rate=44100"));
        assert_eq!(pipewire.last().map(String::as_str), Some("-"));
        let pulse = args(Player::Pulse);
        assert!(pulse.iter().any(|arg| arg == "--volume=32768"));
        assert!(pulse.iter().any(|arg| arg == "--raw"));
    }
}
