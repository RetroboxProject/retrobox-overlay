//! The system controls and information the overlay shows everywhere, like
//! the Switch's quick settings: the screen's brightness and the system's
//! volume (sliders), the network and the battery (the status line), and
//! putting the computer to sleep (outside games).
//!
//! Nothing here may stall the menu: what takes a program to read (the
//! volume, the primary screen) is read in a thread as soon as the overlay
//! starts, and comes back as a `Msg`; each change is applied by a thread of
//! its own that only ever applies the latest value asked for (a slider held
//! down asks for many). The brightness waits for the slider to rest, then
//! glides there from where it is, one way only, rather than jumping or
//! following every move.

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
/// The brightness changes once the slider has rested this long (moving it
/// again meanwhile starts the wait again)...
const BRIGHTNESS_REST: Duration = Duration::from_millis(250);
/// ...and glides there from where it is, easing in and out, straight there
/// (a move meanwhile stops it where it is, until the slider rests again):
/// `GLIDE_TIME` from one end to the other (less for a short change), a
/// write every `GLIDE_TICK` (a dozen at most; none twice the same).
const GLIDE_TIME: Duration = Duration::from_millis(420);
const GLIDE_TICK: Duration = Duration::from_millis(35);
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

    /// The current value, 0 to 1, and whether it is muted.
    pub fn get(&self) -> Option<(f32, bool)> {
        match self {
            Self::Brightness(screen) => screen.get().map(|value| (value, false)),
            Self::Volume(mixer) => mixer.read(),
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
    pub muted: bool,
    pub setter: Setter,
}

impl Found {
    pub fn new(control: Control) -> Option<Self> {
        let (value, muted) = control.get()?;
        log::info!(
            "{:?}: {:.0} %{}",
            control.kind(),
            value * 100.0,
            if muted { " (muted)" } else { "" }
        );
        Some(Self {
            kind: control.kind(),
            value,
            muted,
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
    /// Mute or unmute (the volume).
    mute: Option<bool>,
    busy: bool,
    /// The overlay is closing: no waiting for the slider to rest.
    closing: bool,
    stop: bool,
}

impl Pending {
    const fn idle(&self) -> bool {
        self.value.is_none() && self.mute.is_none() && !self.busy
    }
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

    /// Mute or unmute soon (the volume).
    pub fn mute(&self, muted: bool) {
        let (lock, changed) = &*self.shared;
        lock.lock().unwrap_or_else(PoisonError::into_inner).mute = Some(muted);
        changed.notify_all();
    }
}

/// What the setter's thread is asked to do.
enum Job {
    Value(f32),
    Mute(bool),
}

/// The setter's thread: wait for a value, apply it, again.
fn work(mut control: Control, shared: &(Mutex<Pending>, Condvar)) {
    let (lock, changed) = shared;
    let mut current = control.get().map(|(value, _)| value);
    let mut last: Option<Instant> = None;
    loop {
        let job = {
            let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if let Some(muted) = pending.mute.take() {
                    pending.busy = true;
                    break Job::Mute(muted);
                }
                if let Some(value) = pending.value.take() {
                    pending.busy = true;
                    break Job::Value(value);
                }
                if pending.stop {
                    return;
                }
                pending = changed
                    .wait(pending)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        match (&mut control, job) {
            (Control::Volume(mixer), Job::Mute(muted)) => mixer.set_mute(muted),
            (Control::Brightness(_), Job::Mute(_)) => {}
            (Control::Brightness(screen), Job::Value(target)) => {
                let target = rested(shared, target);
                let from = current.unwrap_or(target);
                let moved = || {
                    lock.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .value
                        .is_some()
                };
                current = Some(glide(screen, from, target, &moved));
            }
            (Control::Volume(_), Job::Value(target)) => {
                // spaced: the latest value after waiting
                let wait = last.map_or(Duration::ZERO, |last| {
                    VOLUME_SPACING.saturating_sub(last.elapsed())
                });
                let mut target = target;
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                    let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
                    target = pending.value.take().unwrap_or(target);
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

/// Wait for the slider to rest (`BRIGHTNESS_REST` without a newer value,
/// or the overlay closing): its value then.
fn rested((lock, changed): &(Mutex<Pending>, Condvar), mut target: f32) -> f32 {
    let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let mut since = Instant::now();
    loop {
        if let Some(newer) = pending.value.take() {
            target = newer;
            since = Instant::now();
        }
        let left = BRIGHTNESS_REST.saturating_sub(since.elapsed());
        if pending.closing || pending.stop || left.is_zero() {
            return target;
        }
        pending = changed
            .wait_timeout(pending, left)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// Take the backlight from `from` straight to `target` along an ease in
/// and out, a write every `GLIDE_TICK`; a new value asked for (`moved`)
/// stops it where it is. Where it ends.
fn glide(screen: &mut Screen, from: f32, target: f32, moved: &impl Fn() -> bool) -> f32 {
    let distance = (target - from).abs();
    // a short change takes less time; one end to the other, GLIDE_TIME
    let time = GLIDE_TIME.mul_f32(distance.sqrt().clamp(0.35, 1.0));
    let steps = (time.as_secs_f32() / GLIDE_TICK.as_secs_f32())
        .ceil()
        .max(1.0) as u32;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let eased = t * t * (3.0 - 2.0 * t);
        let now = from + (target - from) * eased;
        // refused (NVIDIA's backlight while it still fades): the next one comes soon
        screen.set(now);
        if step < steps {
            std::thread::sleep(GLIDE_TICK);
            if moved() {
                return now;
            }
        }
    }
    // the last value must land: tried again a few times if refused
    for _ in 0..LAST_TRIES {
        if screen.set(target) || moved() {
            break;
        }
        std::thread::sleep(GLIDE_TICK);
    }
    target
}

impl Drop for Setter {
    /// Let the last value be applied (for a while at most), then stop.
    fn drop(&mut self) {
        let (lock, changed) = &*self.shared;
        let deadline = Instant::now() + FLUSH_TIMEOUT;
        let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
        pending.closing = true;
        changed.notify_all();
        while !pending.idle() {
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
