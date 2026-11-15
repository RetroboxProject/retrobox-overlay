//! The sound server's changes, followed as they happen, and the audio menu's
//! requests, done in the background.
//!
//! Nothing here polls. A child process tells when something changes:
//! `pw-dump --monitor` with `PipeWire` (the same JSON it dumps, object by
//! object, which is also what the menu's outputs and profiles are read
//! from), `pactl subscribe` with `PulseAudio`. A thread reads it and a second one,
//! the service, collects the changes that come together and then:
//!
//! * asks the volume's thread to read the volume again (`Refresher`: it
//!   tells the slider only when the volume really is not what it knew, so
//!   the volume's own changes that come back as events are no news);
//! * while the audio page is open, sends the outputs and profiles as they
//!   are (`Msg::Devices`), when they changed.
//!
//! The menu's requests (a default output, a profile) go through the service
//! too, so they are done one at a time, in order.

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::value::RawValue;

use super::Tool;
use super::devices::{Action, Snapshot};
use super::pipewire::{self, Graph, Touched};
use super::pulse::{self, Facility};
use crate::system::{Msg, Refresher, installed};

/// How long the service waits for more changes before acting on those it
/// has (a volume key held down, a profile switch that adds and removes
/// objects), and for how long at most.
const COALESCE: Duration = Duration::from_millis(15);
const COALESCE_MAX: Duration = Duration::from_millis(80);
/// How long a profile change is waited for before the menu is told whatever
/// state the card is in.
const SETTLE: Duration = Duration::from_millis(2500);

/// The sound server's interface that is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    /// `PipeWire`'s own tools, for the volume's `wpctl` and `pw-dump` for the rest.
    PipeWire,
    /// `pactl`: `PulseAudio`, or `PipeWire`'s `PulseAudio` server when its own
    /// tools are not there. Its lists need JSON output (`PulseAudio` 16).
    Pulse { json: bool },
}

impl Backend {
    /// `PipeWire` first, whenever its tools are there.
    fn detect(volume: Tool) -> Option<Self> {
        if volume == Tool::Wpctl && installed("pw-dump") {
            return Some(Self::PipeWire);
        }
        installed("pactl").then(|| Self::Pulse {
            json: pulse::supports_json(),
        })
    }

    const fn lists_devices(self) -> bool {
        matches!(self, Self::PipeWire | Self::Pulse { json: true })
    }
}

enum Input {
    /// `PipeWire` objects that changed (the first batch is everything).
    Graph(Vec<pipewire::Update>),
    /// Something changed in `PulseAudio`.
    Pulse(Facility),
    /// The audio page is open (or not).
    Want(bool),
    Do(Action),
    /// The watcher's output ended: the sound server is gone.
    Lost,
    Stop,
}

/// The process that tells what changes, and the thread reading it.
struct Watcher {
    child: Child,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    fn start(backend: Backend, input: &mpsc::Sender<Input>) -> Option<Self> {
        let (program, args): (&str, &[&str]) = match backend {
            Backend::PipeWire => ("pw-dump", &["--monitor", "--no-colors"]),
            Backend::Pulse { .. } => ("pactl", &["subscribe"]),
        };
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .inspect_err(|err| log::warn!("cannot run {program}: {err}"))
            .ok()?;
        let stdout = child.stdout.take()?;
        let input = input.clone();
        let thread = std::thread::Builder::new()
            .name("audio-watch".into())
            .spawn(move || {
                match backend {
                    Backend::PipeWire => read_pipewire(stdout, &input),
                    Backend::Pulse { .. } => read_pulse(stdout, &input),
                }
                input.send(Input::Lost).ok();
            })
            .inspect_err(|err| log::warn!("cannot start a thread: {err}"));
        let Ok(thread) = thread else {
            child.kill().ok();
            child.wait().ok();
            return None;
        };
        Some(Self {
            child,
            thread: Some(thread),
        })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // the reader ends with the process
        self.child.kill().ok();
        self.child.wait().ok();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// `pw-dump --monitor` prints a JSON array for the first dump and one for
/// each change; the objects are parsed one by one.
fn read_pipewire(stdout: ChildStdout, input: &mpsc::Sender<Input>) {
    let batches = serde_json::Deserializer::from_reader(BufReader::new(stdout))
        .into_iter::<Vec<Box<RawValue>>>();
    let mut first = true;
    for batch in batches {
        let batch = match batch {
            Ok(batch) => batch,
            Err(err) => {
                log::warn!("pw-dump: {err}");
                return;
            }
        };
        let updates: Vec<_> = batch
            .iter()
            .filter_map(|object| pipewire::parse_object(object.get()))
            .collect();
        // the first one is sent even when empty: it says the graph is known
        if (first || !updates.is_empty()) && input.send(Input::Graph(updates)).is_err() {
            return;
        }
        first = false;
    }
}

fn read_pulse(stdout: ChildStdout, input: &mpsc::Sender<Input>) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            return;
        };
        if let Some(facility) = pulse::parse_event(&line)
            && input.send(Input::Pulse(facility)).is_err()
        {
            return;
        }
    }
}

/// The audio menu's backend: follows the sound server, and does what the menu
/// asks of it. Dropping it stops them.
pub struct Hub {
    input: mpsc::Sender<Input>,
    devices: bool,
    watcher: Option<Watcher>,
    service: Option<JoinHandle<()>>,
}

impl Hub {
    /// Start following the sound server; None when there is no way to (no
    /// `pw-dump` or `pactl`, or no sound server running).
    pub fn start(
        volume: Tool,
        refresher: Refresher,
        sender: calloop::channel::Sender<Msg>,
    ) -> Option<Self> {
        let backend = Backend::detect(volume)?;
        let (input, receiver) = mpsc::channel();
        let watcher = Watcher::start(backend, &input)?;
        let service = Service {
            backend,
            graph: Graph::default(),
            loaded: false,
            wanted: false,
            sent: None,
            settling: None,
            refresher,
            sender,
        };
        let service = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || service.run(&receiver))
            .inspect_err(|err| log::warn!("cannot start a thread: {err}"))
            .ok()?;
        log::info!("following the sound server through {backend:?}");
        Some(Self {
            input,
            devices: backend.lists_devices(),
            watcher: Some(watcher),
            service: Some(service),
        })
    }

    /// Whether the outputs and profiles can be listed and changed.
    pub const fn lists_devices(&self) -> bool {
        self.devices
    }

    /// The audio page opens or closes: while it is open, the outputs and
    /// profiles are sent (`Msg::Devices`) now and whenever they change.
    pub fn watch_devices(&self, wanted: bool) {
        self.input.send(Input::Want(wanted)).ok();
    }

    /// Do what the menu asked, soon.
    pub fn apply(&self, action: Action) {
        self.input.send(Input::Do(action)).ok();
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        drop(self.watcher.take());
        self.input.send(Input::Stop).ok();
        if let Some(service) = self.service.take() {
            service.join().ok();
        }
    }
}

struct Service {
    backend: Backend,
    graph: Graph,
    /// `PipeWire`'s first dump has come: the graph is known.
    loaded: bool,
    wanted: bool,
    /// What the menu was last sent.
    sent: Option<Snapshot>,
    /// A profile was asked for: until it shows (or time runs out), what the
    /// card goes through meanwhile (its output gone, then back) is not sent.
    settling: Option<Settling>,
    refresher: Refresher,
    sender: calloop::channel::Sender<Msg>,
}

struct Settling {
    until: Instant,
    action: Action,
}

impl Service {
    fn run(mut self, input: &mpsc::Receiver<Input>) {
        loop {
            let first = match &self.settling {
                None => match input.recv() {
                    Ok(first) => first,
                    Err(_) => return,
                },
                Some(settling) => {
                    match input
                        .recv_timeout(settling.until.saturating_duration_since(Instant::now()))
                    {
                        Ok(first) => first,
                        Err(RecvTimeoutError::Timeout) => {
                            // it did not come: what there is is what there is
                            self.settling = None;
                            if self.wanted {
                                self.publish(true);
                            }
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            };
            let mut batch = vec![first];
            // what comes together is dealt with together
            let deadline = Instant::now() + COALESCE_MAX;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match input.recv_timeout(left.min(COALESCE)) {
                    Ok(next) => batch.push(next),
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
                }
            }
            let mut touched = Touched::default();
            let mut force = false;
            for item in batch {
                match item {
                    Input::Stop => return,
                    Input::Graph(updates) => {
                        self.loaded = true;
                        for update in updates {
                            let change = self.graph.update(update);
                            touched.volume |= change.volume;
                            touched.devices |= change.devices;
                        }
                    }
                    Input::Pulse(facility) => {
                        touched.volume |= facility != Facility::Card;
                        touched.devices = true;
                    }
                    Input::Want(wanted) => {
                        self.wanted = wanted;
                        force |= wanted;
                    }
                    // what was shown as done is put right when it was not
                    Input::Do(action) => {
                        let done = self.perform(&action);
                        force |= !done;
                        self.settling =
                            (done && matches!(action, Action::SetProfile { .. })).then(|| {
                                Settling {
                                    until: Instant::now() + SETTLE,
                                    action,
                                }
                            });
                    }
                    Input::Lost => {
                        log::warn!("the sound server's changes are no longer followed");
                        self.loaded = false;
                        self.graph = Graph::default();
                        self.send(Snapshot::default());
                    }
                }
            }
            if touched.volume {
                self.refresher.refresh();
            }
            if self.wanted && (touched.devices || force) {
                self.publish(force);
            }
        }
    }

    fn perform(&self, action: &Action) -> bool {
        let done = match (self.backend, action) {
            (Backend::PipeWire, Action::SetDefault(id)) => pipewire::set_default(id),
            (Backend::PipeWire, Action::SetProfile { card, profile }) => {
                pipewire::set_profile(card, profile)
            }
            (Backend::Pulse { .. }, Action::SetDefault(id)) => pulse::set_default(id),
            (Backend::Pulse { .. }, Action::SetProfile { card, profile }) => {
                pulse::set_profile(card, profile)
            }
        };
        if !done {
            log::warn!("the sound server refused {action:?}");
        }
        done
    }

    /// Send the outputs and profiles as they are, when they are not what
    /// the menu has (or `force`).
    fn publish(&mut self, force: bool) {
        let snapshot = match self.backend {
            Backend::PipeWire => self.loaded.then(|| self.graph.snapshot()),
            Backend::Pulse { json: true } => pulse::snapshot(),
            Backend::Pulse { json: false } => None,
        };
        if let (Some(settling), Some(snapshot)) = (&self.settling, &snapshot) {
            if snapshot.default_card().is_none() || !snapshot.is_current(&settling.action) {
                return;
            }
            self.settling = None;
        }
        if let Some(snapshot) = snapshot
            && (force || self.sent.as_ref() != Some(&snapshot))
        {
            self.send(snapshot);
        }
    }

    fn send(&mut self, snapshot: Snapshot) {
        self.sent = Some(snapshot.clone());
        self.sender.send(Msg::Devices(snapshot)).ok();
    }
}
