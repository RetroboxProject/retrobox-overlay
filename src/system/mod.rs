//! The system controls and information the overlay shows everywhere, like
//! the Switch's quick settings: the screen's brightness and the system's
//! volume (sliders), the network and the battery (the status line), and
//! putting the computer to sleep (outside games).
//!
//! Nothing here may stall the menu: what takes a program to read (the
//! volume, a monitor's brightness over DDC/CI) is read in a thread as soon
//! as the overlay starts, and comes back as a `Msg`; each change is applied
//! by a thread of its own that only ever applies the latest value asked for
//! (a slider held down asks for many).

pub mod audio;
pub mod brightness;
pub mod status;

use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use audio::Mixer;
use brightness::{DdcMonitor, Screen};

/// What a slider controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliderKind {
    Brightness,
    Volume,
}

/// How much Left/Right move a slider.
pub const STEP: f32 = 0.05;
/// How often a slider's changes reach the system, at most: each one runs a
/// program, and the backlight or the sound server do some work.
const VOLUME_SPACING: Duration = Duration::from_millis(50);
/// NVIDIA's backlight fades to each value it is given, and refuses another
/// one meanwhile.
const BACKLIGHT_SPACING: Duration = Duration::from_millis(300);
/// A backlight changes once the slider has stopped this long, never while
/// it moves: each change stalls NVIDIA's display a little, and changes on
/// the way (each one fading on its own) made the screen go the other way
/// when the slider went back and forth.
const BACKLIGHT_SETTLE: Duration = Duration::from_millis(250);
/// A refused change is tried again this much later, this many times (unless
/// a newer one comes).
const RETRY_AFTER: Duration = Duration::from_millis(200);
const RETRIES: u32 = 3;
/// How long closing waits for the last change to be applied.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// A slider's control, found and read.
pub enum Control {
    Brightness(Screen),
    Volume(Mixer),
}

impl Control {
    pub const fn kind(&self) -> SliderKind {
        match self {
            Self::Brightness(_) => SliderKind::Brightness,
            Self::Volume(_) => SliderKind::Volume,
        }
    }

    /// The current value, 0 to 1.
    pub fn get(&self) -> Option<f32> {
        match self {
            Self::Brightness(screen) => screen.get(),
            Self::Volume(mixer) => mixer.get(),
        }
    }

    /// Apply a value; false when it was refused for now.
    fn set(&mut self, value: f32) -> bool {
        match self {
            Self::Brightness(screen) => screen.set(value),
            Self::Volume(mixer) => {
                mixer.set(value);
                true
            }
        }
    }

    /// How far apart its changes go, at least: a backlight's driver has
    /// work to do on each one (and NVIDIA's refuses one too close to the
    /// last); a monitor over DDC/CI is slow enough by itself.
    fn spacing(&self) -> Duration {
        match self {
            Self::Brightness(screen) if screen.is_ddc() => Duration::ZERO,
            Self::Brightness(_) => BACKLIGHT_SPACING,
            Self::Volume(_) => VOLUME_SPACING,
        }
    }

    /// How long the slider has to stay still before a change goes (see
    /// `BACKLIGHT_SETTLE`).
    fn settle(&self) -> Duration {
        match self {
            Self::Brightness(screen) if !screen.is_ddc() => BACKLIGHT_SETTLE,
            _ => Duration::ZERO,
        }
    }
}

/// A slider's control, found and read.
pub struct Found {
    pub kind: SliderKind,
    pub value: f32,
    pub setter: Setter,
}

impl Found {
    pub fn new(control: Control) -> Option<Self> {
        let value = control.get()?;
        log::info!("{:?}: {:.0} %", control.kind(), value * 100.0);
        Some(Self {
            kind: control.kind(),
            value,
            setter: Setter::new(control),
        })
    }
}

/// What the threads that read find.
pub enum Msg {
    /// A slider's control.
    Found(Found),
    /// A slider's value read again (a monitor over DDC/CI).
    Value(SliderKind, f32),
    /// X11's primary output (None: none set).
    Primary(Option<String>),
    /// The monitors that answer DDC/CI, found again.
    Ddc(Vec<DdcMonitor>),
}

/// Read the volume in a thread (`wpctl` or `pactl`).
pub fn read_volume(sender: calloop::channel::Sender<Msg>) {
    spawn("volume", move || {
        if let Some(found) = Mixer::detect().and_then(|mixer| Found::new(Control::Volume(mixer))) {
            sender.send(Msg::Found(found)).ok();
        }
    });
}

/// Read X11's primary output in a thread (nothing under Wayland).
pub fn read_x11_primary(sender: calloop::channel::Sender<Msg>) {
    spawn("primary", move || {
        sender.send(Msg::Primary(brightness::x11_primary())).ok();
    });
}

/// Read a monitor's brightness again in a thread (DDC/CI: slow).
pub fn read_again(screen: Screen, sender: calloop::channel::Sender<Msg>) {
    spawn("ddc", move || {
        if let Some(value) = screen.read_again() {
            sender.send(Msg::Value(SliderKind::Brightness, value)).ok();
        }
    });
}

/// Find the monitors that answer DDC/CI in a thread (slow).
pub fn detect_ddc(runtime_dir: std::path::PathBuf, sender: calloop::channel::Sender<Msg>) {
    spawn("ddc", move || {
        sender
            .send(Msg::Ddc(brightness::detect_ddc(&runtime_dir)))
            .ok();
    });
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().name(name.into()).spawn(work) {
        log::warn!("cannot start a thread: {err}");
    }
}

/// Applies a control's values in a thread, the latest one only.
pub struct Setter {
    kind: SliderKind,
    shared: Arc<(Mutex<Pending>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Pending {
    value: Option<f32>,
    busy: bool,
    stop: bool,
}

impl Setter {
    fn new(mut control: Control) -> Self {
        let kind = control.kind();
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name(format!("{kind:?}"))
            .spawn(move || {
                let (lock, changed) = &*worker;
                let mut last: Option<Instant> = None;
                loop {
                    let mut value = {
                        let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
                        loop {
                            if let Some(value) = pending.value.take() {
                                pending.busy = true;
                                break value;
                            }
                            if pending.stop {
                                return;
                            }
                            pending = changed
                                .wait(pending)
                                .unwrap_or_else(PoisonError::into_inner);
                        }
                    };
                    // changes spaced (a drag asks for dozens a second): the
                    // latest one, after waiting; a refused one again later
                    let spacing = control.spacing();
                    let newer = || {
                        lock.lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .value
                            .take()
                    };
                    let wait = last.map_or(Duration::ZERO, |last: Instant| {
                        spacing.saturating_sub(last.elapsed())
                    });
                    if !wait.is_zero() {
                        std::thread::sleep(wait);
                        value = newer().unwrap_or(value);
                    }
                    // still moving: wait for it to stop
                    let settle = control.settle();
                    if !settle.is_zero() {
                        loop {
                            std::thread::sleep(settle);
                            match newer() {
                                Some(moved) => value = moved,
                                None => break,
                            }
                        }
                    }
                    let mut tries = 0;
                    while !control.set(value) && tries < RETRIES {
                        tries += 1;
                        std::thread::sleep(RETRY_AFTER);
                        value = newer().unwrap_or(value);
                    }
                    last = Some(Instant::now());
                    let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
                    pending.busy = false;
                    changed.notify_all();
                }
            })
            .ok();
        Self {
            kind,
            shared,
            thread,
        }
    }

    pub const fn kind(&self) -> SliderKind {
        self.kind
    }

    /// Apply a value soon, replacing one not applied yet.
    pub fn set(&self, value: f32) {
        let (lock, changed) = &*self.shared;
        lock.lock().unwrap_or_else(PoisonError::into_inner).value = Some(value.clamp(0.0, 1.0));
        changed.notify_all();
    }
}

impl Drop for Setter {
    /// Let the last value be applied (for a while at most), then stop.
    fn drop(&mut self) {
        let (lock, changed) = &*self.shared;
        let deadline = Instant::now() + FLUSH_TIMEOUT;
        let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
        while pending.value.is_some() || pending.busy {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                log::warn!("{:?}: the last change is still being applied", self.kind);
                return;
            }
            pending = changed
                .wait_timeout(pending, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        pending.stop = true;
        drop(pending);
        changed.notify_all();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// Run a command and return its standard output, when it succeeds.
pub fn output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        log::debug!("{program} {args:?}: {}", output.status);
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run a command for its effect; true when it succeeded.
pub fn run(program: &str, args: &[&str]) -> bool {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => true,
        Ok(status) => {
            log::warn!("{program} {args:?}: {status}");
            false
        }
        Err(err) => {
            log::warn!("cannot run {program}: {err}");
            false
        }
    }
}

/// Put the computer to sleep (logind lets the active session do it).
pub fn standby() {
    log::info!("going to sleep");
    if !run("systemctl", &["suspend"]) {
        run("loginctl", &["suspend"]);
    }
}
