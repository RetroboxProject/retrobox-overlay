//! What opens the overlay, anywhere in Retrobox.
//!
//! Retrobox starts it with EmulationStation and it lives as long as the
//! session. It reads every controller and keyboard (evdev, without grabbing
//! them: EmulationStation and the games keep getting every input, but for
//! the hotkey's shield, see `shield`) and opens
//! the overlay (`retrobox-overlay --show`, a process of its own), when
//!
//! * the hotkey is tapped alone (let go within `TAP_MAX`, see
//!   `retrobox_input::press`), on controllers where it is a button of its
//!   own (Home/Guide): `es_input.cfg` gives it an input no other name has;
//! * A (Nintendo's A, the east button) is pressed while the hotkey is held,
//!   on controllers where the hotkey is a button the game uses too (Select,
//!   on controllers without one of their own), which never taps: Hotkey+A
//!   opens it at once. A controller EmulationStation does not know gets
//!   both (Select+A, and its Home/Guide's tap, should it have one). Hotkey+B
//!   is RetroArch's menu, on every controller;
//! * Ctrl+Shift+O (or Shift+F7, where the function keys need Fn) is pressed on
//!   a keyboard.
//!
//! Any other button, the d-pad, a stick or a trigger pushed while the hotkey
//! is down makes that press no tap, and blocks Hotkey+A until everything is
//! released again, so the hotkey's own combinations (Hotkey+Start...) and the
//! hotkey held to move the mouse with a stick never open the overlay by
//! surprise.
//!
//! What the hotkey and A are comes from each controller's layout in
//! `es_input.cfg` (`retrobox_input::es_input`), read once when it appears
//! and again when EmulationStation saves the file; the mapper binds the
//! profiles with that same layout. The overlay, a process of its own, reads
//! the layouts of the controllers it is handed.
//!
//! The same triggers close it, while the overlay itself takes care of the
//! controllers (it grabs them while open, so this process sees nothing). Only
//! one overlay runs at a time; `retrobox-overlay` started on its own (a
//! keyboard shortcut of the desktop, a script) asks this process to open or
//! close it (SIGUSR1); `retrobox-overlay --power` (a power button's binding,
//! the System page's *Power off…*) asks for the power options (SIGUSR2): the
//! overlay in a popup of its own (`--show --power`), the menu closed first
//! if it is open, closed if it is the popup already. Nothing opens while a
//! game is starting or ending
//! (`launch.json`, see `retrobox_input::launch`). New controllers and
//! keyboards are picked up as they appear (inotify on `/dev/input`).
//!
//! While a game runs, the same events go to the mapper too (`mapper`: the
//! game's pad-to-keyboard profile), which follows `session.json` in the
//! runtime directory (inotify as well); with a standalone emulator, the
//! hotkey held alone takes its controller for a moment, so a shortcut's
//! buttons do not reach the game (`shield`).
//! It also suggests a break after a while playing (`breaks`: a toast).

mod breaks;

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

use retrobox_input::es_input::{self, Layout};
use retrobox_input::instance::{self, Instance};
use retrobox_input::launch;
use retrobox_input::paths;
use retrobox_input::press::TAP_MAX;
use retrobox_input::probe::{self, Found, Kind};

use crate::mapper::{self, Mapper};
use crate::parent;
use crate::shield::{self, Shield};

/// Where the input devices appear.
const INPUT_DIR: &str = "/dev/input";
/// Two triggers closer than this are one: a controller seen twice (Steam
/// Input's virtual pad next to the real one) must not open and close at once.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Fraction of an axis' half range past which a stick counts as pushed.
const STICK_THRESHOLD: f32 = 0.5;
/// Fraction of its range past which an axis is not at rest (the shield
/// only goes up with every axis at rest).
const REST_THRESHOLD: f32 = 0.25;
/// What the launcher writes about the game being played.
const SESSION_FILE: &str = "session.json";

/// The hotkey of one controller, and what it opens.
#[derive(Debug)]
struct Hold {
    /// What EmulationStation says the controller's inputs are, read once
    /// for everything here (the mapper binds the profiles with it), and
    /// again when `es_input.cfg` changes.
    layout: Option<Layout>,
    /// Its code, as the controller sends it.
    hotkey: KeyCode,
    /// It is a button of its own (Home/Guide), not one the game uses too:
    /// its tap opens the overlay, and the shield follows it.
    dedicated: bool,
    /// A hotkey that is a game's button (Select) and the button that opens
    /// the overlay with it held (A, Nintendo's east button), as the
    /// controller sends them: where the hotkey is not one of its own, and
    /// on a controller EmulationStation does not know (its Home/Guide,
    /// if it has one, may never come).
    chord: Option<(KeyCode, KeyCode)>,
    /// The buttons down (gamepad buttons and the hotkey).
    held: HashSet<KeyCode>,
    /// When the dedicated hotkey went down alone, with nothing else since:
    /// let go within `TAP_MAX`, it is a tap.
    tapping: Option<Instant>,
    /// Something was pressed with the hotkey (one of its shortcuts), or the
    /// overlay just closed: Hotkey+A does not count until every button is up.
    blocked: bool,
    /// Axis -> (centre, half range)
    axes: HashMap<AbsoluteAxisCode, (i32, i32)>,
    shield: Shield,
}

impl Hold {
    fn new(device: &mut Device) -> Self {
        let mut axes = HashMap::new();
        if let Ok(infos) = device.get_absinfo() {
            for (axis, info) in infos {
                let half = (info.maximum() - info.minimum()) / 2;
                if half > 0 {
                    axes.insert(axis, (info.minimum() + half, half));
                }
            }
        }
        let mut hold = Self::with(KeyCode::BTN_MODE, true, axes);
        hold.relayout(device);
        hold
    }

    /// Read the controller's layout, and take the hotkey and A from it
    /// (EmulationStation's word); without one, Home/Guide where there is
    /// one, and Select with the east button in any case.
    fn relayout(&mut self, device: &mut Device) {
        self.shield.lower(device, self.hotkey, false);
        self.layout = Layout::of(device);
        let layout = self.layout.as_ref();
        (self.hotkey, self.dedicated) = layout.and_then(Layout::hotkey).unwrap_or_else(|| {
            let guide = device
                .supported_keys()
                .is_some_and(|keys| keys.contains(KeyCode::BTN_MODE));
            let hotkey = if guide {
                KeyCode::BTN_MODE
            } else {
                KeyCode::BTN_SELECT
            };
            (hotkey, guide)
        });
        self.chord = match layout {
            Some(_) if self.dedicated => None,
            Some(layout) => Some((self.hotkey, layout.button("a").unwrap_or(KeyCode::BTN_EAST))),
            None => Some((KeyCode::BTN_SELECT, KeyCode::BTN_EAST)),
        };
        self.resync(device);
    }

    fn with(hotkey: KeyCode, dedicated: bool, axes: HashMap<AbsoluteAxisCode, (i32, i32)>) -> Self {
        Self {
            layout: None,
            hotkey,
            dedicated,
            chord: (!dedicated).then_some((hotkey, KeyCode::BTN_EAST)),
            held: HashSet::new(),
            tapping: None,
            blocked: false,
            axes,
            shield: Shield::default(),
        }
    }

    /// Whether `code` is one of the buttons followed here.
    fn follows(&self, code: KeyCode) -> bool {
        code == self.hotkey
            || self
                .chord
                .is_some_and(|(hotkey, button)| code == hotkey || code == button)
            || is_pad_button(code)
    }

    /// Whether a hotkey is down (the tapping one, or the chord's).
    fn hotkey_held(&self) -> bool {
        self.held.contains(&self.hotkey)
            || self
                .chord
                .is_some_and(|(hotkey, _)| self.held.contains(&hotkey))
    }

    /// Read the buttons down from the kernel (after the overlay had the
    /// controller grabbed): whatever is held then must be released first.
    fn resync(&mut self, device: &Device) {
        self.held = device
            .get_key_state()
            .map(|keys| keys.iter().filter(|key| self.follows(*key)).collect())
            .unwrap_or_default();
        self.blocked = !self.held.is_empty();
        self.tapping = None;
    }

    /// A button went down or up, `now`: whether that opens (or closes) the
    /// overlay.
    fn button(&mut self, code: KeyCode, pressed: bool, now: Instant) -> bool {
        if !pressed {
            self.held.remove(&code);
            let tapped = code == self.hotkey
                && self
                    .tapping
                    .take()
                    .is_some_and(|since| now.saturating_duration_since(since) <= TAP_MAX);
            if self.held.is_empty() {
                self.blocked = false;
            }
            return tapped;
        }
        self.held.insert(code);
        let alone = self.held.len() == 1;
        self.tapping =
            (self.dedicated && alone && code == self.hotkey && !self.blocked).then_some(now);
        if alone || !self.hotkey_held() {
            return false;
        }
        // with a hotkey down: Hotkey+A (A pressed last, see `chord`), or a
        // shortcut of its own; either way nothing more until all is up
        let opens = !self.blocked
            && self.held.len() == 2
            && self
                .chord
                .is_some_and(|(hotkey, button)| code == button && self.held.contains(&hotkey));
        self.blocked = true;
        opens
    }

    /// A stick, the d-pad's hat or a trigger moved: pushed far enough while
    /// the hotkey is down alone, that press is no tap.
    fn moved(&mut self, axis: AbsoluteAxisCode, value: i32) {
        if self.tapping.is_some() && self.pushed(axis, value) {
            self.tapping = None;
            self.blocked = true;
        }
    }

    /// Whether an axis value is far from where it rests: the centre, for a
    /// stick or the d-pad's hat; the end, for a trigger.
    fn pushed(&self, axis: AbsoluteAxisCode, value: i32) -> bool {
        self.axes.get(&axis).is_some_and(|&(centre, half)| {
            let away = if is_trigger(axis) {
                (value - (centre - half)) as f32 / (2 * half) as f32
            } else {
                ((value - centre) as f32 / half as f32).abs()
            };
            away >= STICK_THRESHOLD
        })
    }

    /// Whether every axis is at rest: sticks and hat centred, triggers up.
    fn at_rest(&self, device: &Device) -> bool {
        let Ok(state) = device.get_abs_state() else {
            return true;
        };
        self.axes.iter().all(|(axis, &(centre, half))| {
            let value = state
                .get(usize::from(axis.0))
                .map_or(centre, |info| info.value);
            let away = if is_trigger(*axis) {
                // from its end, over its whole range
                (value - (centre - half)) as f32 / (2 * half) as f32
            } else {
                ((value - centre) as f32 / half as f32).abs()
            };
            away < REST_THRESHOLD
        })
    }
}

/// The analog triggers, which rest at one end of their range.
fn is_trigger(axis: AbsoluteAxisCode) -> bool {
    matches!(
        axis,
        AbsoluteAxisCode::ABS_Z
            | AbsoluteAxisCode::ABS_RZ
            | AbsoluteAxisCode::ABS_GAS
            | AbsoluteAxisCode::ABS_BRAKE
    )
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

/// Where the power options (`retrobox-overlay --power`) are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Power {
    /// Not shown (the overlay, if open, is the menu or the keyboard).
    Away,
    /// The overlay shown is their popup.
    Shown,
    /// Asked for while the menu was open: they open once it has closed.
    Next,
}

struct State {
    handle: LoopHandle<'static, State>,
    runtime_dir: PathBuf,
    inputs: HashMap<PathBuf, Input>,
    /// The overlay, while it is shown.
    overlay: Option<Child>,
    /// Where the power options are.
    power: Power,
    last_trigger: Option<Instant>,
    last_power: Option<Instant>,
    stop: bool,
    /// Where events are read into.
    events: Vec<evdev::InputEvent>,
    mapper: Mapper,
    /// The mapper's tick is scheduled.
    ticking: bool,
    /// The running game's emulator needs the hotkey's shield.
    shielding: bool,
    /// The running game's break reminders.
    breaks: breaks::Breaks,
}

/// Run until stopped (SIGTERM, SIGINT), or until `parent` (Retrobox's
/// process) ends: open the overlay on its triggers.
pub fn run(runtime_dir: &Path, parent: Option<i32>) -> Result<()> {
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
        Signal::SIGUSR2,
        Signal::SIGTERM,
        Signal::SIGINT,
        Signal::SIGCHLD,
    ])?;
    handle
        .insert_source(signals, |event, &mut (), state| match event.signal() {
            Signal::SIGUSR1 => state.toggle(None),
            Signal::SIGUSR2 => state.power(),
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
    watch_es_input(&handle);
    if let Some(pid) = parent {
        if let Err(err) = parent::watch(&handle, pid, |state: &mut State| {
            log::info!("Retrobox has ended: stopping");
            state.stop = true;
        }) {
            log::info!("{err:#}: not starting");
            return Ok(());
        }
        log::info!("living with Retrobox's process {pid}");
    }

    let mut state = State {
        handle,
        runtime_dir: runtime_dir.to_owned(),
        inputs: HashMap::new(),
        overlay: None,
        power: Power::Away,
        last_trigger: None,
        last_power: None,
        stop: false,
        events: Vec::new(),
        mapper: Mapper::default(),
        ticking: false,
        shielding: false,
        breaks: breaks::Breaks::default(),
    };
    state.scan();
    state.load_session();
    log::info!(
        "overlay daemon: {} controller(s), {} keyboard(s)",
        state.pads().count(),
        state.inputs.len() - state.pads().count(),
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

/// Follow the user's `es_input.cfg`, where EmulationStation saves a
/// controller configured: the layouts are read again. Without its
/// directory (EmulationStation never ran), the layouts are read as the
/// controllers appear.
fn watch_es_input(handle: &LoopHandle<'static, State>) {
    let Some(file) = es_input::user_config_file() else {
        return;
    };
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return;
    };
    let name = name.to_owned();
    let watched =
        Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).and_then(|inotify| {
            inotify.add_watch(
                dir,
                AddWatchFlags::IN_CLOSE_WRITE | AddWatchFlags::IN_MOVED_TO,
            )?;
            Ok(inotify)
        });
    let inotify = match watched {
        Ok(inotify) => inotify,
        Err(err) => {
            log::info!("not following {}: {err}", file.display());
            return;
        }
    };
    let source = Generic::new(inotify, Interest::READ, Mode::Level);
    let inserted = handle.insert_source(source, move |_, inotify, state| {
        let changed = inotify.read_events().unwrap_or_default();
        if changed
            .iter()
            .any(|event| event.name.as_deref() == Some(name.as_os_str()))
        {
            log::info!("es_input.cfg changed: reading the controllers' layouts again");
            state.relayout();
        }
        Ok(PostAction::Continue)
    });
    if let Err(err) = inserted {
        log::warn!("cannot follow es_input.cfg: {}", err.error);
    }
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
        let Ok(mut device) = Device::open(&path) else {
            return;
        };
        let watched = match found.kind {
            Kind::Pad => Watched::Pad(Hold::new(&mut device)),
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
                Watched::Pad(hold) if hold.dedicated => "controller, its hotkey opens",
                Watched::Pad(_) => "controller, hotkey is a game button: Hotkey+A opens",
                Watched::Keyboard(_) => "keyboard",
            }
        );
        if let Watched::Pad(hold) = &watched {
            self.mapper.attach(&path, &device, hold.layout.as_ref());
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

    /// The game started or ended: map its controllers, or stop; shield its
    /// hotkey, or not.
    fn load_session(&mut self) {
        self.shielding = shield::wanted(&self.runtime_dir);
        let pads = self
            .inputs
            .values()
            .filter_map(|input| match &input.watched {
                Watched::Pad(hold) => {
                    Some((input.path.as_path(), &input.device, hold.layout.as_ref()))
                }
                Watched::Keyboard(_) => None,
            });
        self.mapper.load(&self.runtime_dir, pads);
        self.follow_breaks();
    }

    /// `es_input.cfg` changed (a controller configured in EmulationStation):
    /// read every controller's layout again, and bind the mapper's profiles
    /// to it.
    fn relayout(&mut self) {
        for input in self.inputs.values_mut() {
            if let Watched::Pad(hold) = &mut input.watched {
                hold.relayout(&mut input.device);
                self.mapper.detach(&input.path);
                self.mapper
                    .attach(&input.path, &input.device, hold.layout.as_ref());
            }
        }
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
            if let Watched::Pad(hold) = &mut input.watched
                && hold.shield.skips(event, hold.hotkey)
            {
                continue;
            }
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
                (Watched::Pad(hold), EventSummary::Key(_, code, value @ (0 | 1)))
                    if hold.follows(code) =>
                {
                    self.pad_button(path, code, value == 1);
                }
                (Watched::Pad(hold), EventSummary::AbsoluteAxis(_, axis, value)) => {
                    hold.moved(axis, value);
                }
                _ => {}
            }
        }
        self.events = events;
        self.schedule_tick();
    }

    /// A gamepad button going down or up.
    fn pad_button(&mut self, path: &Path, code: KeyCode, pressed: bool) {
        let shielding = self.shielding && self.overlay.is_none();
        let Some(Input {
            device,
            watched: Watched::Pad(hold),
            ..
        }) = self.inputs.get_mut(path)
        else {
            return;
        };
        let opens = hold.button(code, pressed, Instant::now());
        if code == hold.hotkey && !pressed {
            hold.shield.lower(device, hold.hotkey, true);
        } else if code == hold.hotkey
            && shielding
            && hold.dedicated
            && hold.held.len() == 1
            && hold.at_rest(device)
        {
            hold.shield.raise(device);
        }
        if opens {
            log::info!(
                "{} on {}",
                if pressed { "Hotkey+A" } else { "hotkey" },
                path.display()
            );
            self.toggle(Some(path));
        }
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
        self.show(from, false);
    }

    /// The power options (SIGUSR2): shown, or closed if shown already; the
    /// menu is closed first if it is open.
    fn power(&mut self) {
        let now = Instant::now();
        if self
            .last_power
            .is_some_and(|last| now.duration_since(last) < DEBOUNCE)
        {
            log::debug!("power options ignored: too close to the last ask");
            return;
        }
        self.last_power = Some(now);
        self.reap();
        match &self.overlay {
            Some(child) if self.power == Power::Shown => {
                log::info!("closing the power options");
                signal(child, nix::sys::signal::Signal::SIGUSR1);
            }
            Some(child) => {
                log::info!("closing the overlay for the power options");
                self.power = Power::Next;
                signal(child, nix::sys::signal::Signal::SIGUSR1);
            }
            None => self.show(None, true),
        }
    }

    /// Start the overlay: the menu (or, `power`, the power options), with
    /// the controllers (the one it was opened from first).
    fn show(&mut self, from: Option<&Path>, power: bool) {
        if launch::keeps_away() {
            log::info!(
                "a game is starting or ending, or a frontend (Kodi) is in front: the overlay stays closed"
            );
            return;
        }
        self.breaks.hide_toast();
        // it takes the controllers: what the mapper holds is let go now (their
        // releases will not come here), and the shields give them back
        self.mapper.release_all();
        for input in self.inputs.values_mut() {
            if let Watched::Pad(hold) = &mut input.watched {
                hold.shield.lower(&mut input.device, hold.hotkey, false);
            }
        }
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
        if power {
            command.arg("--power");
        }
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
                log::info!(
                    "{} shown ({})",
                    if power { "power options" } else { "overlay" },
                    child.id()
                );
                self.overlay = Some(child);
                self.power = if power { Power::Shown } else { Power::Away };
            }
            Err(err) => log::error!("cannot show the overlay: {err}"),
        }
    }

    /// The overlay closed: forget it, and read the controllers' buttons
    /// again (it had them grabbed: their releases never came here).
    fn reap(&mut self) {
        self.mapper.reap();
        self.breaks.reap();
        let finished = self
            .overlay
            .as_mut()
            .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
        if !finished {
            return;
        }
        self.overlay = None;
        let next = self.power == Power::Next;
        self.power = Power::Away;
        log::info!("overlay closed");
        for input in self.inputs.values_mut() {
            if let Watched::Pad(hold) = &mut input.watched {
                hold.resync(&input.device);
            }
        }
        if next {
            self.show(None, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A hold with `hotkey`, and the left stick's X axis (0 to 255).
    fn hold(hotkey: KeyCode, dedicated: bool) -> Hold {
        let axes = [(AbsoluteAxisCode::ABS_X, (128, 127))].into();
        Hold::with(hotkey, dedicated, axes)
    }

    #[test]
    fn a_tap_on_the_dedicated_hotkey_opens() {
        let (mode, south, now) = (KeyCode::BTN_MODE, KeyCode::BTN_SOUTH, Instant::now());
        let mut pad = hold(mode, true);
        assert!(!pad.button(mode, true, now));
        assert!(pad.button(mode, false, now));
        // a shortcut of its own: no tap
        assert!(!pad.button(mode, true, now));
        assert!(!pad.button(KeyCode::BTN_START, true, now));
        assert!(!pad.button(KeyCode::BTN_START, false, now));
        assert!(!pad.button(south, true, now));
        assert!(!pad.button(south, false, now));
        assert!(!pad.button(mode, false, now));
        // the stick pushed while it is down: no tap
        assert!(!pad.button(mode, true, now));
        pad.moved(AbsoluteAxisCode::ABS_X, 140);
        pad.moved(AbsoluteAxisCode::ABS_X, 250);
        assert!(!pad.button(mode, false, now));
        // down while something else is held: no tap
        assert!(!pad.button(south, true, now));
        assert!(!pad.button(mode, true, now));
        assert!(!pad.button(south, false, now));
        assert!(!pad.button(mode, false, now));
    }

    #[test]
    fn a_long_press_on_the_hotkey_is_no_tap() {
        let (mode, now) = (KeyCode::BTN_MODE, Instant::now());
        let mut pad = hold(mode, true);
        assert!(!pad.button(mode, true, now));
        assert!(pad.button(mode, false, now + TAP_MAX));
        assert!(!pad.button(mode, true, now));
        assert!(!pad.button(mode, false, now + TAP_MAX * 2));
    }

    #[test]
    fn hotkey_a_opens_where_the_hotkey_is_a_game_button() {
        let (select, east, now) = (KeyCode::BTN_SELECT, KeyCode::BTN_EAST, Instant::now());
        let mut pad = hold(select, false);
        assert!(!pad.button(select, true, now));
        assert!(pad.button(east, true, now));
        assert!(!pad.button(east, false, now));
        // a hotkey the game uses too never taps
        assert!(!pad.button(select, false, now));
        assert!(!pad.button(select, true, now));
        assert!(!pad.button(select, false, now));
        // Hotkey+B is RetroArch's
        assert!(!pad.button(select, true, now));
        assert!(!pad.button(KeyCode::BTN_SOUTH, true, now));
        assert!(!pad.button(KeyCode::BTN_SOUTH, false, now));
        assert!(!pad.button(select, false, now));
        // A first, then the hotkey, is not Hotkey+A
        assert!(!pad.button(east, true, now));
        assert!(!pad.button(select, true, now));
    }

    #[test]
    fn a_controller_es_does_not_know_has_both() {
        let (mode, select, east, now) = (
            KeyCode::BTN_MODE,
            KeyCode::BTN_SELECT,
            KeyCode::BTN_EAST,
            Instant::now(),
        );
        // Home/Guide taps, and Select+A opens too, should Home never come
        let mut pad = hold(mode, true);
        pad.chord = Some((select, east));
        assert!(!pad.button(mode, true, now));
        assert!(pad.button(mode, false, now));
        assert!(!pad.button(select, true, now));
        assert!(pad.button(east, true, now));
        assert!(!pad.button(east, false, now));
        assert!(!pad.button(select, false, now));
    }

    #[test]
    fn a_dedicated_hotkey_only_taps() {
        let (mode, now) = (KeyCode::BTN_MODE, Instant::now());
        let mut pad = hold(mode, true);
        for other in [KeyCode::BTN_EAST, KeyCode::BTN_SOUTH] {
            assert!(!pad.button(mode, true, now));
            assert!(!pad.button(other, true, now));
            assert!(!pad.button(other, false, now));
            assert!(!pad.button(mode, false, now), "no tap either");
        }
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
