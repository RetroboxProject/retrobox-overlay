//! What opens the overlay, anywhere in Retrobox.
//!
//! Retrobox starts it with EmulationStation and it lives as long as the
//! session. It reads every controller and keyboard (evdev, without grabbing
//! them: EmulationStation and the games keep getting every input) and opens
//! the overlay (`retrobox-overlay --show`, a process of its own), when
//!
//! * B (Nintendo's B, the south button) is pressed while the hotkey is held,
//!   on any controller: Hotkey+B opens it at once;
//! * the hotkey is held alone for `hold_ms` (750 ms by default), on
//!   controllers with a dedicated one (Home/Guide, `BTN_MODE`). Where the
//!   hotkey is a button the game uses (Select, as in EmulationStation),
//!   holding it opens nothing;
//! * Ctrl+Shift+O (or Shift+F7, where the function keys need Fn) is pressed on
//!   a keyboard.
//!
//! Any other button, the d-pad or a stick pushed while those are held cancels
//! the hold until everything is released again, so the hotkey's own
//! combinations (Hotkey+Start...) and the hotkey held to move the mouse with
//! a stick never open the overlay by surprise.
//!
//! The same triggers close it, while the overlay itself takes care of the
//! controllers (it grabs them while open, so this process sees nothing). Only
//! one overlay runs at a time; `retrobox-overlay` started on its own (a
//! keyboard shortcut of the desktop, a script) asks this process to open or
//! close it (SIGUSR1). New controllers and keyboards are picked up as they
//! appear (inotify on `/dev/input`).
//!
//! While a game runs, the same events go to the mapper too (`mapper`: the
//! game's pad-to-keyboard profile), which follows `session.json` in the
//! runtime directory (inotify as well).

use std::collections::{HashMap, HashSet};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use calloop::generic::Generic;
use calloop::signals::{Signal, Signals};
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, InotifyEvent};
use nix::sys::signal::kill;
use nix::unistd::Pid;

use retrobox_input::instance::{self, Instance};
use retrobox_input::paths;
use retrobox_input::probe::{self, Found, Kind};

use crate::mapper::{self, Mapper};

/// Where the input devices appear.
const INPUT_DIR: &str = "/dev/input";
/// Two triggers closer than this are one: a controller seen twice (Steam
/// Input's virtual pad next to the real one) must not open and close at once.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Fraction of an axis' half range past which a stick counts as pushed.
const STICK_THRESHOLD: f32 = 0.5;
/// What the launcher writes about the game being played.
const SESSION_FILE: &str = "session.json";

/// Which held buttons open the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chord {
    /// The dedicated hotkey alone, held for a moment.
    Hotkey,
    /// The hotkey and B (Nintendo's B: the south button, Xbox's A), on any
    /// controller: opens it at once, when B goes down with the hotkey already held.
    HotkeyB,
}

/// What `held` makes of a controller's buttons.
fn chord_of(held: &HashSet<KeyCode>, hotkey: KeyCode, dedicated: bool) -> Option<Chord> {
    if held.len() == 2 && held.contains(&hotkey) && held.contains(&KeyCode::BTN_SOUTH) {
        return Some(Chord::HotkeyB);
    }
    // a hotkey that is also a game button (Select) is never held to open it
    (dedicated && held.len() == 1 && held.contains(&hotkey)).then_some(Chord::Hotkey)
}

/// The hold of one controller.
#[derive(Debug)]
struct Hold {
    hotkey: KeyCode,
    dedicated: bool,
    /// The buttons down (gamepad buttons only).
    held: HashSet<KeyCode>,
    /// The chord being held, its timer and since when.
    armed: Option<(Chord, RegistrationToken)>,
    /// Something else was pressed (or the overlay just closed): no chord
    /// counts until every button is up.
    blocked: bool,
    /// Axis -> (centre, half range)
    axes: HashMap<AbsoluteAxisCode, (i32, i32)>,
}

impl Hold {
    fn new(device: &Device) -> Self {
        let dedicated = device
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::BTN_MODE));
        let mut axes = HashMap::new();
        if let Ok(infos) = device.get_absinfo() {
            for (axis, info) in infos {
                let half = (info.maximum() - info.minimum()) / 2;
                if half > 0 {
                    axes.insert(axis, (info.minimum() + half, half));
                }
            }
        }
        let mut hold = Self {
            hotkey: if dedicated {
                KeyCode::BTN_MODE
            } else {
                KeyCode::BTN_SELECT
            },
            dedicated,
            held: HashSet::new(),
            armed: None,
            blocked: false,
            axes,
        };
        hold.resync(device);
        hold
    }

    /// Read the buttons down from the kernel (after the overlay had the
    /// controller grabbed): whatever is held then must be released first.
    fn resync(&mut self, device: &Device) {
        self.held = device
            .get_key_state()
            .map(|keys| keys.iter().filter(|key| is_pad_button(*key)).collect())
            .unwrap_or_default();
        self.blocked = !self.held.is_empty();
    }

    /// Whether an axis value is far from its centre (a stick or the d-pad's hat).
    fn pushed(&self, axis: AbsoluteAxisCode, value: i32) -> bool {
        // the analog triggers rest at one end: they never cancel a hold
        if matches!(
            axis,
            AbsoluteAxisCode::ABS_Z
                | AbsoluteAxisCode::ABS_RZ
                | AbsoluteAxisCode::ABS_GAS
                | AbsoluteAxisCode::ABS_BRAKE
        ) {
            return false;
        }
        self.axes.get(&axis).is_some_and(|(centre, half)| {
            ((value - centre) as f32 / *half as f32).abs() >= STICK_THRESHOLD
        })
    }
}

/// Gamepad buttons: `BTN_JOYSTICK` to `BTN_THUMBR`, and the d-pad buttons.
fn is_pad_button(code: KeyCode) -> bool {
    (0x120..=0x13f).contains(&code.code()) || (0x220..=0x223).contains(&code.code())
}

/// A watched device.
enum Watched {
    Pad(Hold),
    /// A keyboard, with the modifiers that are down.
    Keyboard(Modifiers),
}

/// Which Shift and Ctrl keys are down (each side on its own: both may be
/// held and one let go).
#[derive(Debug, Clone, Copy, Default)]
struct Modifiers(u8);

impl Modifiers {
    const fn bit(code: KeyCode) -> u8 {
        match code {
            KeyCode::KEY_LEFTSHIFT => 1,
            KeyCode::KEY_RIGHTSHIFT => 2,
            KeyCode::KEY_LEFTCTRL => 4,
            KeyCode::KEY_RIGHTCTRL => 8,
            _ => 0,
        }
    }

    /// A key went down or up (only the modifiers count).
    fn update(&mut self, code: KeyCode, down: bool) {
        if down {
            self.0 |= Self::bit(code);
        } else {
            self.0 &= !Self::bit(code);
        }
    }

    const fn shift(self) -> bool {
        self.0 & 0b11 != 0
    }

    const fn ctrl(self) -> bool {
        self.0 & 0b1100 != 0
    }

    /// Whether pressing `code` now (`value` 1: a press, not a repeat) is
    /// the keyboard shortcut of the overlay: Ctrl+Shift+O, or Shift+F7.
    fn opens_overlay(self, code: KeyCode, value: i32) -> bool {
        value == 1
            && self.shift()
            && match code {
                KeyCode::KEY_F7 => true,
                KeyCode::KEY_O => self.ctrl(),
                _ => false,
            }
    }
}

struct Input {
    path: PathBuf,
    device: Device,
    token: RegistrationToken,
    watched: Watched,
}

struct State {
    handle: LoopHandle<'static, State>,
    runtime_dir: PathBuf,
    hold: Duration,
    inputs: HashMap<PathBuf, Input>,
    /// The overlay, while it is shown.
    overlay: Option<Child>,
    last_trigger: Option<Instant>,
    stop: bool,
    /// Where events are read into.
    events: Vec<evdev::InputEvent>,
    mapper: Mapper,
    /// The mapper's tick is scheduled.
    ticking: bool,
}

/// Run until stopped (SIGTERM, SIGINT): open the overlay on its triggers.
pub fn run(runtime_dir: &Path, hold: Duration) -> Result<()> {
    let _lock = match instance::acquire_named(runtime_dir, instance::DAEMON_LOCK_FILE)? {
        Instance::First(lock) => lock,
        Instance::Running(pid) => {
            log::info!("the overlay daemon already runs ({pid:?})");
            return Ok(());
        }
    };
    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("cannot create the event loop")?;
    let handle = event_loop.handle();

    let signals = Signals::new(&[
        Signal::SIGUSR1,
        Signal::SIGTERM,
        Signal::SIGINT,
        Signal::SIGCHLD,
    ])?;
    handle
        .insert_source(signals, |event, &mut (), state| match event.signal() {
            Signal::SIGUSR1 => state.toggle(None),
            Signal::SIGCHLD => state.reap(),
            _ => state.stop = true,
        })
        .map_err(|err| anyhow!("cannot watch signals: {}", err.error))?;

    let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
        .context("cannot watch /dev/input")?;
    inotify
        .add_watch(
            INPUT_DIR,
            AddWatchFlags::IN_CREATE | AddWatchFlags::IN_ATTRIB | AddWatchFlags::IN_DELETE,
        )
        .context("cannot watch /dev/input")?;
    handle
        .insert_source(
            Generic::new(inotify, Interest::READ, Mode::Level),
            |_, inotify, state| {
                // which device changed does not matter: look at them all
                let changed = inotify.read_events().unwrap_or_default();
                if !changed.is_empty() {
                    state.scan();
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| anyhow!("cannot watch /dev/input: {}", err.error))?;

    watch_session(&handle, runtime_dir)?;

    let mut state = State {
        handle,
        runtime_dir: runtime_dir.to_owned(),
        hold,
        inputs: HashMap::new(),
        overlay: None,
        last_trigger: None,
        stop: false,
        events: Vec::new(),
        mapper: Mapper::default(),
        ticking: false,
    };
    state.scan();
    state.load_session();
    log::info!(
        "overlay daemon: {} controller(s), {} keyboard(s), hold {} ms",
        state.pads().count(),
        state.inputs.len() - state.pads().count(),
        hold.as_millis()
    );

    while !state.stop {
        event_loop
            .dispatch(None, &mut state)
            .context("event loop error")?;
    }
    state.mapper.release_all();
    if let Some(mut child) = state.overlay.take() {
        signal(&child, nix::sys::signal::Signal::SIGTERM);
        child.wait().ok();
    }
    Ok(())
}

/// Follow `session.json`: written, replaced or removed, the mapper reads it
/// again.
fn watch_session(handle: &LoopHandle<'static, State>, runtime_dir: &Path) -> Result<()> {
    let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
        .context("cannot watch the runtime directory")?;
    inotify
        .add_watch(
            runtime_dir,
            AddWatchFlags::IN_CLOSE_WRITE
                | AddWatchFlags::IN_MOVED_TO
                | AddWatchFlags::IN_MOVED_FROM
                | AddWatchFlags::IN_DELETE,
        )
        .with_context(|| format!("cannot watch {}", runtime_dir.display()))?;
    handle
        .insert_source(
            Generic::new(inotify, Interest::READ, Mode::Level),
            |_, inotify, state| {
                let changed = inotify.read_events().unwrap_or_default();
                if changed.iter().any(is_session) {
                    state.load_session();
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| anyhow!("cannot watch the runtime directory: {}", err.error))?;
    Ok(())
}

fn is_session(event: &InotifyEvent) -> bool {
    event
        .name
        .as_deref()
        .is_some_and(|name| name == SESSION_FILE)
}

/// Send a signal to the overlay.
fn signal(child: &Child, signal: nix::sys::signal::Signal) {
    if let Ok(pid) = i32::try_from(child.id())
        && let Err(err) = kill(Pid::from_raw(pid), signal)
    {
        log::debug!("cannot signal the overlay: {err}");
    }
}

impl State {
    fn pads(&self) -> impl Iterator<Item = &Input> {
        self.inputs
            .values()
            .filter(|input| matches!(input.watched, Watched::Pad(_)))
    }

    /// Open the devices not watched yet; forget the ones gone.
    fn scan(&mut self) {
        let gone: Vec<PathBuf> = self
            .inputs
            .keys()
            .filter(|path| !path.exists())
            .cloned()
            .collect();
        for path in gone {
            self.forget(&path);
        }
        for found in probe::scan() {
            if !self.inputs.contains_key(&found.path) {
                self.watch(found);
            }
        }
    }

    /// Watch a controller or a keyboard.
    fn watch(&mut self, found: Found) {
        let path = found.path;
        // not readable yet (udev sets the permissions just after): IN_ATTRIB
        // brings it back
        let Ok(device) = Device::open(&path) else {
            return;
        };
        let watched = match found.kind {
            Kind::Pad => Watched::Pad(Hold::new(&device)),
            Kind::Keyboard => Watched::Keyboard(Modifiers::default()),
        };
        if device.set_nonblocking(true).is_err() {
            return;
        }
        let Ok(fd) = device.as_fd().try_clone_to_owned() else {
            return;
        };
        let source_path = path.clone();
        let token = self.handle.insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            move |_, _, state| {
                state.read(&source_path);
                Ok(PostAction::Continue)
            },
        );
        let Ok(token) = token else {
            return;
        };
        log::info!(
            "watching {} ({}): {}",
            path.display(),
            device.name().unwrap_or("?"),
            match &watched {
                Watched::Pad(hold) if hold.dedicated => "controller, hold Home/Guide",
                Watched::Pad(_) => "controller, hotkey is a game button: no hold",
                Watched::Keyboard(_) => "keyboard",
            }
        );
        if matches!(watched, Watched::Pad(_)) {
            self.mapper.attach(&path, &device);
        }
        self.inputs.insert(
            path.clone(),
            Input {
                path,
                device,
                token,
                watched,
            },
        );
    }

    /// The game started or ended: map its controllers, or stop.
    fn load_session(&mut self) {
        let pads = self
            .inputs
            .values()
            .filter(|input| matches!(input.watched, Watched::Pad(_)))
            .map(|input| (input.path.as_path(), &input.device));
        self.mapper.load(&self.runtime_dir, pads);
    }

    /// Have the mapper ticked while it has something to do (a combination
    /// held for its time, a stick moving the pointer).
    fn schedule_tick(&mut self) {
        if self.ticking || !self.mapper.needs_tick() {
            return;
        }
        let timer =
            self.handle
                .insert_source(Timer::from_duration(mapper::TICK), |_, &mut (), state| {
                    if state.mapper.tick() {
                        TimeoutAction::ToDuration(mapper::TICK)
                    } else {
                        state.ticking = false;
                        TimeoutAction::Drop
                    }
                });
        self.ticking = timer.is_ok();
    }

    fn forget(&mut self, path: &Path) {
        if let Some(input) = self.inputs.remove(path) {
            log::info!("{} gone", input.path.display());
            match input.watched {
                Watched::Pad(_) => self.mapper.detach(path),
                Watched::Keyboard(_) => self.mapper.keyboard_gone(),
            }
            self.handle.remove(input.token);
            if let Watched::Pad(Hold {
                armed: Some((_, timer)),
                ..
            }) = input.watched
            {
                self.handle.remove(timer);
            }
        }
    }

    /// The pending events of a device.
    fn read(&mut self, path: &Path) {
        let Some(input) = self.inputs.get_mut(path) else {
            return;
        };
        // one buffer for every read: sticks send hundreds of events a second
        let mut events = std::mem::take(&mut self.events);
        events.clear();
        let gone = match input.device.fetch_events() {
            Ok(fetched) => {
                events.extend(fetched);
                false
            }
            Err(err) => err.kind() != std::io::ErrorKind::WouldBlock,
        };
        if gone {
            // Unplugged, or its driver reloaded: close it now (the kernel
            // frees the input device, and runs its driver's cleanup code, when
            // the last descriptor closes, so a driver unloaded meanwhile would
            // crash it), then look for a node that took its place: the
            // inotify scan may have run first and found the new node under
            // the old one's path, which it took for the watched device.
            self.forget(path);
            self.scan();
        }
        for event in &events {
            // looked up again: a trigger may have changed things
            let Some(input) = self.inputs.get_mut(path) else {
                return;
            };
            match &input.watched {
                Watched::Pad(_) => self.mapper.pad_event(path, event),
                Watched::Keyboard(_) => self.mapper.key_event(event),
            }
            let Some(input) = self.inputs.get_mut(path) else {
                return;
            };
            match (&mut input.watched, event.destructure()) {
                (Watched::Keyboard(modifiers), EventSummary::Key(_, code, value)) => {
                    modifiers.update(code, value != 0);
                    if modifiers.opens_overlay(code, value) {
                        log::info!("keyboard shortcut");
                        self.toggle(None);
                    }
                }
                (Watched::Pad(_), EventSummary::Key(_, code, value @ (0 | 1)))
                    if is_pad_button(code) =>
                {
                    self.pad_button(path, code, value == 1);
                }
                // a stick only matters while a chord is held
                (Watched::Pad(hold), EventSummary::AbsoluteAxis(_, axis, value))
                    if hold.armed.is_some() && hold.pushed(axis, value) =>
                {
                    self.cancel(path);
                }
                _ => {}
            }
        }
        self.events = events;
        self.schedule_tick();
    }

    /// A gamepad button going down or up.
    fn pad_button(&mut self, path: &Path, code: KeyCode, pressed: bool) {
        let Some(Input {
            watched: Watched::Pad(hold),
            ..
        }) = self.inputs.get_mut(path)
        else {
            return;
        };
        if pressed {
            hold.held.insert(code);
        } else {
            hold.held.remove(&code);
        }
        if hold.held.is_empty() {
            hold.blocked = false;
        }
        let chord = (!hold.blocked)
            .then(|| chord_of(&hold.held, hold.hotkey, hold.dedicated))
            .flatten()
            // B pressed first, then the hotkey, is not Hotkey+B
            .filter(|chord| *chord != Chord::HotkeyB || (pressed && code == KeyCode::BTN_SOUTH));
        let armed = hold.armed.map(|(chord, _)| chord);
        if chord == armed {
            return;
        }
        // something other than a chord, or another chord: start again
        if let Some((_, timer)) = hold.armed.take() {
            self.handle.remove(timer);
        }
        let Some(chord) = chord else {
            // a press that is not part of a chord blocks until all is up
            if pressed && armed.is_some() {
                hold.blocked = true;
            }
            return;
        };
        if chord == Chord::HotkeyB {
            hold.blocked = true;
            log::info!("{chord:?} on {}", path.display());
            self.toggle(Some(path));
            return;
        }
        let target = path.to_owned();
        let timer =
            self.handle
                .insert_source(Timer::from_duration(self.hold), move |_, &mut (), state| {
                    state.held_long_enough(&target);
                    TimeoutAction::Drop
                });
        if let Ok(timer) = timer {
            hold.armed = Some((chord, timer));
        }
    }

    /// A stick pushed while a chord was held: not a chord any more.
    fn cancel(&mut self, path: &Path) {
        if let Some(Input {
            watched: Watched::Pad(hold),
            ..
        }) = self.inputs.get_mut(path)
            && let Some((_, timer)) = hold.armed.take()
        {
            self.handle.remove(timer);
            hold.blocked = true;
        }
    }

    /// The chord's timer ran out: it has been held all along (any change
    /// removes the timer).
    fn held_long_enough(&mut self, path: &Path) {
        let Some(Input {
            watched: Watched::Pad(hold),
            ..
        }) = self.inputs.get_mut(path)
        else {
            return;
        };
        let Some((chord, _)) = hold.armed.take() else {
            return;
        };
        hold.blocked = true;
        log::info!("{chord:?} held on {}", path.display());
        self.toggle(Some(path));
    }

    /// Open the overlay, or close it when it is open.
    fn toggle(&mut self, from: Option<&Path>) {
        let now = Instant::now();
        if self
            .last_trigger
            .is_some_and(|last| now.duration_since(last) < DEBOUNCE)
        {
            log::debug!("trigger ignored: too close to the last one");
            return;
        }
        self.last_trigger = Some(now);
        self.reap();
        if let Some(child) = &self.overlay {
            log::info!("closing the overlay");
            signal(child, nix::sys::signal::Signal::SIGUSR1);
            return;
        }
        // it takes the controllers: what the mapper holds is let go now (their
        // releases will not come here)
        self.mapper.release_all();
        // the controller that asked first, then every other one: outside a
        // game, the overlay grabs them all
        let mut pads: Vec<&Path> = from.into_iter().collect();
        pads.extend(
            self.pads()
                .map(|input| input.path.as_path())
                .filter(|path| Some(*path) != from),
        );
        let mut command = Command::new(paths::sibling_command("retrobox-overlay"));
        command.arg("--show");
        if from.is_some() {
            // the controller opened it: the game may keep the keyboard
            command.arg("--from-pad");
        }
        for pad in pads {
            command.arg("--pad").arg(pad);
        }
        match command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                log::info!("overlay shown ({})", child.id());
                self.overlay = Some(child);
            }
            Err(err) => log::error!("cannot show the overlay: {err}"),
        }
    }

    /// The overlay closed: forget it, and read the controllers' buttons
    /// again (it had them grabbed: their releases never came here).
    fn reap(&mut self) {
        self.mapper.reap();
        let finished = self
            .overlay
            .as_mut()
            .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
        if !finished {
            return;
        }
        self.overlay = None;
        log::info!("overlay closed");
        for input in self.inputs.values_mut() {
            if let Watched::Pad(hold) = &mut input.watched {
                if let Some((_, timer)) = hold.armed.take() {
                    self.handle.remove(timer);
                }
                hold.resync(&input.device);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(codes: &[KeyCode]) -> HashSet<KeyCode> {
        codes.iter().copied().collect()
    }

    fn pressed(codes: &[KeyCode]) -> Modifiers {
        let mut modifiers = Modifiers::default();
        for &code in codes {
            modifiers.update(code, true);
        }
        modifiers
    }

    #[test]
    fn the_keyboard_shortcuts_open_the_overlay() {
        let shift_ctrl = pressed(&[KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTCTRL]);
        assert!(shift_ctrl.opens_overlay(KeyCode::KEY_O, 1));
        assert!(!shift_ctrl.opens_overlay(KeyCode::KEY_O, 2), "a repeat");
        assert!(!shift_ctrl.opens_overlay(KeyCode::KEY_P, 1));
        assert!(pressed(&[KeyCode::KEY_RIGHTSHIFT]).opens_overlay(KeyCode::KEY_F7, 1));
        assert!(!pressed(&[KeyCode::KEY_RIGHTSHIFT]).opens_overlay(KeyCode::KEY_O, 1));
        assert!(!pressed(&[KeyCode::KEY_LEFTCTRL]).opens_overlay(KeyCode::KEY_O, 1));
        assert!(!pressed(&[]).opens_overlay(KeyCode::KEY_F7, 1));
    }

    #[test]
    fn letting_go_of_one_shift_keeps_the_other() {
        let mut modifiers = pressed(&[KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTSHIFT]);
        modifiers.update(KeyCode::KEY_LEFTSHIFT, false);
        assert!(modifiers.shift());
        modifiers.update(KeyCode::KEY_RIGHTSHIFT, false);
        assert!(!modifiers.shift());
    }

    #[test]
    fn chords() {
        let mode = KeyCode::BTN_MODE;
        assert_eq!(chord_of(&held(&[mode]), mode, true), Some(Chord::Hotkey));
        assert_eq!(
            chord_of(&held(&[mode, KeyCode::BTN_SOUTH]), mode, true),
            Some(Chord::HotkeyB)
        );
        assert_eq!(
            chord_of(&held(&[mode, KeyCode::BTN_START]), mode, true),
            None
        );
        // Select is the game's: holding it alone opens nothing, Select+A does
        let select = KeyCode::BTN_SELECT;
        assert_eq!(chord_of(&held(&[select]), select, false), None);
        assert_eq!(
            chord_of(&held(&[select, KeyCode::BTN_SOUTH]), select, false),
            Some(Chord::HotkeyB)
        );
        assert_eq!(chord_of(&held(&[KeyCode::BTN_SOUTH]), select, false), None);
    }

    #[test]
    fn pad_buttons() {
        assert!(is_pad_button(KeyCode::BTN_SOUTH));
        assert!(is_pad_button(KeyCode::BTN_MODE));
        assert!(is_pad_button(KeyCode::BTN_DPAD_UP));
        assert!(!is_pad_button(KeyCode::KEY_A));
        assert!(!is_pad_button(KeyCode::BTN_LEFT));
    }
}
