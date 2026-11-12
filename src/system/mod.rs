//! The system controls and information the overlay shows everywhere, like
//! the Switch's quick settings: the screen's brightness and the system's
//! volume (sliders), the network and the battery (the status line), and
//! putting the computer to sleep (outside games).
//!
//! Nothing here may stall the menu: what takes a program to read (the
//! volume, the primary screen) is read in a thread as soon as the overlay
//! starts, and comes back as a `Msg`; each change is applied by a thread of
//! its own that only ever applies the latest value asked for (a slider held
//! down asks for many). The brightness glides to it, a few backlight steps
//! at a time, rather than jumping.

pub mod audio;
pub mod brightness;
pub mod status;

use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use audio::Mixer;
use brightness::Screen;

/// What a slider controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliderKind {
    Brightness,
    Volume,
}

/// How much Left/Right move a slider.
pub const STEP: f32 = 0.05;
/// How often the volume's changes reach the sound server, at most: each one
/// runs a program.
const VOLUME_SPACING: Duration = Duration::from_millis(50);
/// The brightness glides to the slider's value: every `GLIDE_TICK`, this
/// part of the way left (at least one backlight step), the latest value
/// always the goal. About 0.4 s from one end to the other.
const GLIDE_TICK: Duration = Duration::from_millis(30);
const GLIDE_EASE: f32 = 0.3;
/// How many times the last value of a glide is tried when refused.
const LAST_TRIES: u32 = 10;
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
    /// The display settings' primary screen (None: none set, or no way
    /// to ask).
    Primary(Option<String>),
}

/// Read the volume in a thread (`wpctl` or `pactl`).
pub fn read_volume(sender: calloop::channel::Sender<Msg>) {
    spawn("volume", move || {
        if let Some(found) = Mixer::detect().and_then(|mixer| Found::new(Control::Volume(mixer))) {
            sender.send(Msg::Found(found)).ok();
        }
    });
}

/// Read the display settings' primary screen in a thread.
pub fn read_primary(sender: calloop::channel::Sender<Msg>) {
    spawn("primary", move || {
        sender.send(Msg::Primary(brightness::primary_output())).ok();
    });
}

/// Whether a program is installed (in `PATH`).
pub fn installed(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
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
    fn new(control: Control) -> Self {
        let kind = control.kind();
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name(format!("{kind:?}"))
            .spawn(move || work(control, &worker))
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

/// The setter's thread: wait for a value, apply it, again.
fn work(mut control: Control, shared: &(Mutex<Pending>, Condvar)) {
    let (lock, changed) = shared;
    let newer = || {
        lock.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .value
            .take()
    };
    let mut current = control.get();
    let mut last: Option<Instant> = None;
    loop {
        let target = {
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
        match &mut control {
            Control::Brightness(screen) => {
                current = Some(glide(screen, current.unwrap_or(target), target, &newer));
            }
            Control::Volume(_) => {
                // spaced: the latest value after waiting
                let wait = last.map_or(Duration::ZERO, |last| {
                    VOLUME_SPACING.saturating_sub(last.elapsed())
                });
                let mut target = target;
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                    target = newer().unwrap_or(target);
                }
                control.set(target);
                last = Some(Instant::now());
            }
        }
        let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
        pending.busy = false;
        changed.notify_all();
    }
}

/// Take the backlight from `from` to `target` (or to a newer one, as they
/// come) a few steps at a time; where it ends.
fn glide(screen: &mut Screen, from: f32, mut target: f32, newer: &impl Fn() -> Option<f32>) -> f32 {
    let one_step = screen.step();
    let mut now = from;
    let mut refused = 0;
    loop {
        if let Some(moved) = newer() {
            target = moved;
        }
        let left = target - now;
        if left.abs() <= one_step / 2.0 {
            // the last value must land: tried again a few times if refused
            if screen.set(target) || refused >= LAST_TRIES {
                return target;
            }
            refused += 1;
            now = target;
            std::thread::sleep(GLIDE_TICK);
            continue;
        }
        let step = (left * GLIDE_EASE).abs().max(one_step).min(left.abs());
        now += step.copysign(left);
        // refused (NVIDIA's backlight while it still fades): the next one comes soon
        screen.set(now);
        std::thread::sleep(GLIDE_TICK);
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
