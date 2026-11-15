//! The game's controllers while the overlay is open.
//!
//! Each controller's evdev device (from session.json) is grabbed, so neither
//! the emulator nor gamepadly's mappers see a press until the overlay closes.
//! A grab starts and ends with the buttons up: grabbing while the button
//! that opened the overlay is still held would hide its release from the
//! emulator, which would then keep it pressed (a stuck hotkey). A button that
//! still reads as down after `GRAB_WAIT` (some drivers report one that never
//! comes up) is taken as stuck and ignored from then on, so the controller is
//! always grabbed within that time.
//! Events are read with the Linux gamepad conventions (`BTN_SOUTH`/`BTN_EAST`,
//! d-pad buttons or hat, left stick, shoulders and triggers, `BTN_MODE` for
//! the guide button), which the usual drivers follow (xpad, xone,
//! hid-playstation, hid-nintendo...).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use evdev::AttributeSet;

use crate::es_input::{self, Translation};
use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};

/// What the overlay does with a controller or keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    /// The shoulder buttons (L, R): zoom in the manual, like ES's viewer.
    ZoomOut,
    ZoomIn,
    /// The triggers (L2, R2) and Page Up/Down: pages in the manual.
    PrevPage,
    NextPage,
    Confirm,
    Back,
    /// Home (or F7): the shortcut that opens the overlay, now closing it.
    Close,
    /// Nintendo's X (the top face button): the options of what is selected.
    Options,
    /// F2: the audio page, wherever the selection is.
    Audio,
    /// The on-screen keyboard's keys on the face buttons: backspace (west),
    /// space (north) and enter (Start). Only a controller in keyboard mode
    /// (`Pad::keyboard_mode`) sends them.
    Backspace,
    Space,
    Enter,
}

/// An input going down or coming back up: the manual moves and zooms while
/// a direction or a shoulder is held, as ES's viewer does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    pub nav: Nav,
    pub pressed: bool,
}

impl Input {
    pub const fn press(nav: Nav) -> Self {
        Self { nav, pressed: true }
    }

    pub const fn release(nav: Nav) -> Self {
        Self {
            nav,
            pressed: false,
        }
    }
}

/// Fraction of an axis' half-range past which the stick counts as pushed.
const STICK_THRESHOLD: f32 = 0.6;

/// How long opening waits for the buttons to be released before grabbing:
/// the overlay opens while the hotkey that opened it is still held.
pub const GRAB_WAIT: Duration = Duration::from_millis(1500);
/// How long closing waits for the buttons to be released before ungrabbing.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(2);
/// Polling interval while waiting for that release.
const RELEASE_POLL: Duration = Duration::from_millis(10);

/// What a controller drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// The menu.
    Menu,
    /// The on-screen keyboard.
    Keyboard,
    /// A physical keyboard, while the on-screen one is open: the arrow keys,
    /// Enter and Escape, and nothing else.
    Typewriter,
}

/// A grabbed controller.
pub struct Pad {
    /// Where it was opened from; what tells the controllers apart.
    pub path: PathBuf,
    pub device: Device,
    /// Reading failed for good (unplugged, or its driver unloaded): it must be
    /// dropped at once, see `is_gone`.
    gone: bool,
    grabbed: bool,
    opened: Instant,
    /// Buttons that read as down for longer than `GRAB_WAIT` before the grab.
    stuck: AttributeSet<KeyCode>,
    swap_confirm: bool,
    /// Axis -> (centre, half range, last direction)
    axes: HashMap<AbsoluteAxisCode, (i32, i32, i32)>,
    role: Role,
    faces: Faces,
    /// The buttons as `EmulationStation` numbers them, as standard codes
    /// (see `es_input`); none when it does not know the controller.
    translation: Option<Translation>,
    /// The analog triggers, in keyboard mode: axis -> (lowest, highest, down).
    triggers: HashMap<AbsoluteAxisCode, (i32, i32, bool)>,
}

impl Pad {
    /// Open and grab a controller; None (logged) when it cannot be.
    pub fn open(path: &Path, swap_confirm: bool) -> Option<Self> {
        let started = Instant::now();
        let device = match Device::open(path) {
            Ok(device) => device,
            Err(err) => {
                log::warn!("cannot open {}: {err}", path.display());
                return None;
            }
        };
        if let Err(err) = device.set_nonblocking(true) {
            log::warn!("cannot make {} non-blocking: {err}", path.display());
            return None;
        }
        let mut axes = HashMap::new();
        if let Ok(infos) = device.get_absinfo() {
            for (axis, info) in infos {
                if [
                    AbsoluteAxisCode::ABS_X,
                    AbsoluteAxisCode::ABS_Y,
                    AbsoluteAxisCode::ABS_HAT0X,
                    AbsoluteAxisCode::ABS_HAT0Y,
                ]
                .contains(&axis)
                {
                    let centre = i32::midpoint(info.minimum(), info.maximum());
                    let half = ((info.maximum() - info.minimum()) / 2).max(1);
                    axes.insert(axis, (centre, half, 0));
                }
            }
        }
        let device_id = (
            device.input_id().vendor(),
            device.name().unwrap_or_default().to_lowercase(),
        );
        let mut pad = Self {
            path: path.to_owned(),
            device,
            gone: false,
            grabbed: false,
            opened: Instant::now(),
            stuck: AttributeSet::new(),
            swap_confirm,
            axes,
            role: Role::Menu,
            faces: Faces::of(&device_id),
            translation: None,
            triggers: HashMap::new(),
        };
        pad.translation = es_input::translation(&pad.device);
        if pad.translation.is_some() {
            // the codes are the standard ones now, whatever the driver says
            pad.faces = Faces::Standard;
        }
        let opened = started.elapsed();
        pad.try_grab();
        log::info!(
            "{}: opened in {} ms, grabbed in {} ms",
            path.display(),
            opened.as_millis(),
            started.elapsed().saturating_sub(opened).as_millis()
        );
        Some(pad)
    }

    /// Read the controller for the on-screen keyboard: the face buttons
    /// type, and the analog triggers (L2, R2) count as buttons, which pads
    /// like Xbox's only have as axes.
    pub fn keyboard_mode(&mut self) {
        self.role = Role::Keyboard;
        if let Ok(infos) = self.device.get_absinfo() {
            for (axis, info) in infos {
                if matches!(
                    axis,
                    AbsoluteAxisCode::ABS_Z
                        | AbsoluteAxisCode::ABS_RZ
                        | AbsoluteAxisCode::ABS_BRAKE
                        | AbsoluteAxisCode::ABS_GAS
                ) {
                    self.triggers
                        .insert(axis, (info.minimum(), info.maximum(), false));
                }
            }
        }
    }

    /// How long ago the controller was opened.
    pub fn age(&self) -> Duration {
        self.opened.elapsed()
    }

    /// The buttons down right now, leaving out the stuck ones.
    fn held(&self) -> Vec<KeyCode> {
        self.device.get_key_state().map_or_else(
            |_| Vec::new(),
            |keys| {
                keys.iter()
                    .filter(|key| !self.stuck.contains(*key))
                    .collect()
            },
        )
    }

    fn buttons_held(&self) -> bool {
        !self.held().is_empty()
    }

    /// Grab the controller once its buttons are up, or once `GRAB_WAIT` has
    /// passed (what is still down then counts as stuck). Called on every
    /// event until grabbed, and by a timer at `GRAB_WAIT`.
    pub fn try_grab(&mut self) {
        if self.grabbed {
            return;
        }
        let held = self.held();
        if !held.is_empty() {
            if self.opened.elapsed() < GRAB_WAIT {
                return;
            }
            log::warn!("buttons still down, taken as stuck: {held:?}");
            for key in held {
                self.stuck.insert(key);
            }
        }
        let name = self.device.name().unwrap_or("controller").to_owned();
        match self.device.grab() {
            Ok(()) => log::info!("grabbed {name}"),
            // still usable: the game just sees the presses too
            Err(err) => log::warn!("cannot grab {name}: {err}"),
        }
        self.grabbed = true;
    }

    /// Whether the controller is gone (`ENODEV` once it is unplugged or its
    /// driver unloads). The owner must then drop it, and stop polling its
    /// descriptor, without waiting for the overlay to close: the kernel only
    /// frees the input device (running its driver's cleanup code) when the
    /// last descriptor is closed, and a driver unloaded in the meantime leaves
    /// that code gone, which crashes the kernel.
    pub const fn is_gone(&self) -> bool {
        self.gone
    }

    /// The presses and releases in the pending events.
    pub fn read(&mut self) -> Vec<Input> {
        let events: Vec<_> = match self.device.fetch_events() {
            Ok(events) => events.collect(),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                return Vec::new();
            }
            Err(err) => {
                // the error would come back on every poll: the controller is
                // done for
                log::warn!("controller {} gone: {err}", self.path.display());
                self.gone = true;
                return Vec::new();
            }
        };
        if !self.grabbed {
            // the press that opened the overlay is still being released
            self.try_grab();
            return Vec::new();
        }
        let mut inputs = Vec::new();
        for event in events {
            match event.destructure() {
                // 2 is the kernel's auto-repeat: holding is followed here
                EventSummary::Key(_, code, value @ (0 | 1)) => {
                    let code = self.standard(code);
                    inputs.extend(self.button(code).map(|nav| Input {
                        nav,
                        pressed: value == 1,
                    }));
                }
                EventSummary::AbsoluteAxis(_, axis, value) if self.triggers.contains_key(&axis) => {
                    inputs.extend(self.trigger(axis, value));
                }
                EventSummary::AbsoluteAxis(_, axis, value) => inputs.extend(self.axis(axis, value)),
                _ => {}
            }
        }
        inputs
    }

    /// A button code as the standard layout calls it, going by what
    /// `EmulationStation` says the controller's buttons are.
    fn standard(&self, code: KeyCode) -> KeyCode {
        self.translation
            .as_ref()
            .and_then(|translation| translation.get(&code))
            .copied()
            .unwrap_or(code)
    }

    fn button(&self, code: KeyCode) -> Option<Nav> {
        let (confirm, back) = if self.swap_confirm {
            (KeyCode::BTN_EAST, KeyCode::BTN_SOUTH)
        } else {
            (KeyCode::BTN_SOUTH, KeyCode::BTN_EAST)
        };
        if self.role == Role::Typewriter {
            return match code {
                KeyCode::KEY_UP => Some(Nav::Up),
                KeyCode::KEY_DOWN => Some(Nav::Down),
                KeyCode::KEY_LEFT => Some(Nav::Left),
                KeyCode::KEY_RIGHT => Some(Nav::Right),
                KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER => Some(Nav::Confirm),
                KeyCode::KEY_ESC => Some(Nav::Close),
                _ => None,
            };
        }
        if self.role == Role::Keyboard {
            // by position: the left face button deletes, the top one is space
            let (top, left) = if self.faces == Faces::Xbox {
                (KeyCode::BTN_WEST, KeyCode::BTN_NORTH)
            } else {
                (KeyCode::BTN_NORTH, KeyCode::BTN_WEST)
            };
            match code {
                code if code == left => return Some(Nav::Backspace),
                code if code == top => return Some(Nav::Space),
                KeyCode::BTN_START => return Some(Nav::Enter),
                // the accents key
                code if code == back => return Some(Nav::Options),
                _ => {}
            }
        }
        match code {
            KeyCode::BTN_DPAD_UP => Some(Nav::Up),
            KeyCode::BTN_DPAD_DOWN => Some(Nav::Down),
            KeyCode::BTN_DPAD_LEFT => Some(Nav::Left),
            KeyCode::BTN_DPAD_RIGHT => Some(Nav::Right),
            KeyCode::BTN_TL => Some(Nav::ZoomOut),
            KeyCode::BTN_TR => Some(Nav::ZoomIn),
            KeyCode::BTN_TL2 => Some(Nav::PrevPage),
            KeyCode::BTN_TR2 => Some(Nav::NextPage),
            KeyCode::BTN_MODE => Some(Nav::Close),
            KeyCode::BTN_START => Some(Nav::Confirm),
            // the top button: north on Nintendo and PlayStation pads, west on
            // Xbox ones (their Y), whose X is north
            KeyCode::BTN_NORTH | KeyCode::BTN_WEST => Some(Nav::Options),
            code if code == confirm => Some(Nav::Confirm),
            code if code == back => Some(Nav::Back),
            _ => None,
        }
    }

    /// An analog trigger going past half way (or back): L2 and R2 pressed or
    /// released.
    fn trigger(&mut self, axis: AbsoluteAxisCode, value: i32) -> Option<Input> {
        let (low, high, down) = self.triggers.get_mut(&axis)?;
        let position = (value - *low) as f32 / (*high - *low).max(1) as f32;
        let now = position >= 0.5;
        if now == *down {
            return None;
        }
        *down = now;
        let nav = if matches!(axis, AbsoluteAxisCode::ABS_Z | AbsoluteAxisCode::ABS_BRAKE) {
            Nav::PrevPage
        } else {
            Nav::NextPage
        };
        Some(Input { nav, pressed: now })
    }

    /// A stick or hat moving: the direction it left goes up, the one it
    /// reaches goes down.
    fn axis(&mut self, axis: AbsoluteAxisCode, value: i32) -> Vec<Input> {
        let Some((centre, half, last)) = self.axes.get_mut(&axis) else {
            return Vec::new();
        };
        let position = (value - *centre) as f32 / *half as f32;
        let direction = if position <= -STICK_THRESHOLD {
            -1
        } else {
            i32::from(position >= STICK_THRESHOLD)
        };
        if direction == *last {
            return Vec::new();
        }
        let left = std::mem::replace(last, direction);
        let horizontal = [AbsoluteAxisCode::ABS_X, AbsoluteAxisCode::ABS_HAT0X].contains(&axis);
        let nav = |direction| match (direction, horizontal) {
            (-1, false) => Some(Nav::Up),
            (1, false) => Some(Nav::Down),
            (-1, true) => Some(Nav::Left),
            (1, true) => Some(Nav::Right),
            _ => None,
        };
        nav(left)
            .map(Input::release)
            .into_iter()
            .chain(nav(direction).map(Input::press))
            .collect()
    }
}

/// How Linux names a pad's face buttons: by position, as Sony and
/// Nintendo pads do, or the Xbox way, which calls the left button
/// `BTN_NORTH` and the top one `BTN_WEST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Faces {
    Standard,
    Xbox,
}

impl Faces {
    /// The layout of a pad from its vendor and lower-case name (8BitDo and
    /// others in `XInput` mode say they are Xbox ones).
    fn of((vendor, name): &(u16, String)) -> Self {
        const MICROSOFT: u16 = 0x045e;
        let xbox = *vendor == MICROSOFT
            || ["xbox", "x-box", "xinput", "x360"]
                .iter()
                .any(|word| name.contains(word));
        if xbox { Self::Xbox } else { Self::Standard }
    }
}

/// Open and grab the physical keyboards, for the on-screen keyboard to be
/// moved with: not the virtual ones (the launcher's, which is what the
/// on-screen keyboard types on, would be grabbed from itself).
pub fn open_keyboards() -> Vec<Pad> {
    keyboard_nodes()
        .into_iter()
        .filter_map(|path| Pad::open(&path, false))
        .map(|mut pad| {
            pad.role = Role::Typewriter;
            pad
        })
        .collect()
}

/// The device nodes of the physical keyboards, found in sysfs: opening a
/// device just to look at it is slow (closing it waits for the kernel's RCU
/// grace period, tens of milliseconds each, for every device in the machine).
fn keyboard_nodes() -> Vec<PathBuf> {
    /// `BUS_VIRTUAL`
    const BUS_VIRTUAL: u64 = 0x06;
    let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("event"))
        .filter(|entry| {
            let device = entry.path().join("device");
            let read = |file: &str| std::fs::read_to_string(device.join(file)).unwrap_or_default();
            let keys = key_bits(&read("capabilities/key"));
            let has = |code: KeyCode| bit(&keys, code.code());
            let bus = u64::from_str_radix(read("id/bustype").trim(), 16).unwrap_or(BUS_VIRTUAL);
            has(KeyCode::KEY_A)
                && has(KeyCode::KEY_ENTER)
                && has(KeyCode::KEY_LEFT)
                && !has(KeyCode::BTN_SOUTH)
                && bus != BUS_VIRTUAL
                && !read("name").to_lowercase().contains("retrobox")
        })
        .map(|entry| Path::new("/dev/input").join(entry.file_name()))
        .collect();
    nodes.sort();
    nodes
}

/// A sysfs capability bitmap (`capabilities/key`: hexadecimal words, the
/// highest first) as words, the lowest first.
fn key_bits(text: &str) -> Vec<u64> {
    text.split_whitespace()
        .rev()
        .map(|word| u64::from_str_radix(word, 16).unwrap_or(0))
        .collect()
}

/// Whether bit `index` of the words is set.
fn bit(words: &[u64], index: u16) -> bool {
    words
        .get(usize::from(index) / 64)
        .is_some_and(|word| word >> (index % 64) & 1 == 1)
}

/// Open and grab every controller (a device with the south button), for a
/// screen that runs outside a game and so has no list of its own.
pub fn open_all_pads(swap_confirm: bool) -> Vec<Pad> {
    gamepad_nodes()
        .into_iter()
        .filter_map(|path| Pad::open(&path, swap_confirm))
        .collect()
}

/// The device nodes of the controllers (sysfs, see `keyboard_nodes`).
fn gamepad_nodes() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("event"))
        .filter(|entry| {
            let device = entry.path().join("device");
            let read = |file: &str| std::fs::read_to_string(device.join(file)).unwrap_or_default();
            let name = read("name").to_lowercase();
            bit(
                &key_bits(&read("capabilities/key")),
                KeyCode::BTN_SOUTH.code(),
            ) && !name.contains("retrobox")
        })
        .map(|entry| Path::new("/dev/input").join(entry.file_name()))
        .collect();
    nodes.sort();
    nodes
}

/// Whether the user holds what asks for an emulator to be installed again
/// while a game loads: L1 and R1 on a controller, or Ctrl and Shift on a
/// keyboard.
pub fn reinstall_held() -> bool {
    shoulders_held() || modifiers_held()
}

/// Whether Ctrl and Shift (either of each) are down.
fn ctrl_and_shift(keys: &evdev::AttributeSet<KeyCode>) -> bool {
    (keys.contains(KeyCode::KEY_LEFTCTRL) || keys.contains(KeyCode::KEY_RIGHTCTRL))
        && (keys.contains(KeyCode::KEY_LEFTSHIFT) || keys.contains(KeyCode::KEY_RIGHTSHIFT))
}

/// Whether some keyboard has Ctrl and Shift down right now (the state comes
/// from the kernel, nothing is grabbed).
fn modifiers_held() -> bool {
    keyboard_nodes().iter().any(|path| {
        Device::open(path)
            .ok()
            .and_then(|device| device.get_key_state().ok())
            .is_some_and(|keys| ctrl_and_shift(&keys))
    })
}

/// Whether some controller has both shoulder buttons (L1 and R1) down right
/// now, as EmulationStation numbers them. Reads the state from the kernel,
/// without grabbing anything.
fn shoulders_held() -> bool {
    gamepad_nodes().iter().any(|path| {
        let Ok(device) = Device::open(path) else {
            return false;
        };
        let Ok(keys) = device.get_key_state() else {
            return false;
        };
        let translation = es_input::translation(&device);
        let standard: Vec<KeyCode> = keys
            .iter()
            .map(|key| {
                translation
                    .as_ref()
                    .and_then(|table| table.get(&key).copied())
                    .unwrap_or(key)
            })
            .collect();
        standard.contains(&KeyCode::BTN_TL) && standard.contains(&KeyCode::BTN_TR)
    })
}

/// Wait (a few seconds at most) until no controller has a button down, so the
/// press that closed the overlay is not followed by a lone release in the game.
pub fn wait_released(pads: &mut [Pad]) {
    let deadline = Instant::now() + RELEASE_TIMEOUT;
    while pads.iter().any(Pad::buttons_held) {
        if Instant::now() >= deadline {
            log::warn!("buttons still held, releasing the controllers anyway");
            return;
        }
        for pad in pads.iter_mut() {
            // keep the queue empty; the state comes from the kernel
            drop(pad.device.fetch_events());
        }
        std::thread::sleep(RELEASE_POLL);
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        if self.grabbed
            && let Err(err) = self.device.ungrab()
        {
            log::debug!("ungrab: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_and_shift_are_both_needed() {
        let keys = |codes: &[KeyCode]| codes.iter().copied().collect::<evdev::AttributeSet<_>>();
        assert!(ctrl_and_shift(&keys(&[
            KeyCode::KEY_LEFTCTRL,
            KeyCode::KEY_RIGHTSHIFT
        ])));
        assert!(!ctrl_and_shift(&keys(&[KeyCode::KEY_LEFTSHIFT])));
        assert!(!ctrl_and_shift(&keys(&[KeyCode::KEY_RIGHTCTRL])));
        assert!(!ctrl_and_shift(&keys(&[])));
    }

    #[test]
    fn reads_a_sysfs_key_bitmap() {
        // KEY_A (30) and KEY_LEFT (105), the highest word first
        let keys = key_bits("0 20000000000 40000000");
        assert!(bit(&keys, 30));
        assert!(bit(&keys, 105));
        assert!(!bit(&keys, 29));
        assert!(!bit(&keys, 1000));
    }
}
