//! The pad-to-keyboard mapper: what replaces gamepadly.
//!
//! While a game runs, the launcher names its profile in `session.json`
//! (`pad_profile`: evmapy's format with gamepadly's additions, see
//! `profile`) along with the game's controllers (`controllers`: each
//! player's device). Each of those controllers gets its player's mappings,
//! bound to its inputs by `es_input.cfg` (`bind`), and every keyboard the
//! profile's `actions_keyboard`. Their actions go to a virtual keyboard and
//! mouse (`output`). Without a game, or without `pad_profile`, nothing is
//! mapped.
//!
//! It reads the devices the daemon already reads (without grabbing them:
//! the game gets every input too), and stops getting their events while the
//! overlay has them grabbed: what it held is let go when the overlay opens.

mod bind;
mod engine;
mod output;
mod profile;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use evdev::{Device, EventSummary, InputEvent};
use serde_json::{Map, Value};

use self::bind::Ranges;
use self::engine::{Code, Engine};
use self::output::Output;
use self::profile::{Profile, Section};

/// How often the sticks move the pointer, and held combinations are looked
/// at, while there is something to do.
pub const TICK: Duration = Duration::from_millis(5);

/// What the launcher says about the game, for the mapper.
#[derive(Debug, Clone, PartialEq)]
struct Session {
    /// The profile's path and its contents.
    profile: PathBuf,
    data: Map<String, Value>,
    /// The player of each controller (its device, resolved).
    players: HashMap<PathBuf, u32>,
}

impl Session {
    /// The session in `session.json` of `runtime_dir`, if a game with a
    /// profile runs.
    fn load(runtime_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(runtime_dir.join("session.json")).ok()?;
        let session: Value = serde_json::from_str(&text).ok()?;
        let profile = PathBuf::from(session.get("pad_profile")?.as_str()?);
        let data = match std::fs::read_to_string(&profile)
            .map_err(|err| err.to_string())
            .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|err| err.to_string()))
        {
            Ok(Value::Object(data)) => data,
            Ok(_) => {
                log::warn!("{}: not a JSON object", profile.display());
                return None;
            }
            Err(err) => {
                log::warn!("cannot read {}: {err}", profile.display());
                return None;
            }
        };
        let players = session
            .get("controllers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|controller| {
                let player = u32::try_from(controller.get("player")?.as_u64()?).ok()?;
                let device = controller.get("device")?.as_str()?;
                Some((resolved(Path::new(device)), player))
            })
            .collect();
        Some(Self {
            profile,
            data,
            players,
        })
    }
}

/// A path with its links resolved (the launcher and the daemon may name a
/// device differently).
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// A controller being mapped.
struct Pad {
    engine: Engine,
    ranges: Ranges,
    /// Each hat's position, and the direction it presses.
    hats: HashMap<u16, ((i32, i32), Option<Code>)>,
    /// The direction each axis presses.
    axes: HashMap<u16, Option<Code>>,
}

/// The mappings of the game being played.
#[derive(Default)]
pub struct Mapper {
    session: Option<Session>,
    pads: HashMap<PathBuf, Pad>,
    keyboard: Option<Engine>,
    output: Output,
    last_tick: Option<Instant>,
}

impl Mapper {
    /// Read `session.json` again: map the controllers among `devices` as it
    /// says, or stop mapping when no game (with a profile) runs.
    pub fn load<'a>(
        &mut self,
        runtime_dir: &Path,
        devices: impl IntoIterator<Item = (&'a Path, &'a Device)>,
    ) {
        let session = Session::load(runtime_dir);
        if session == self.session {
            return;
        }
        self.release_all();
        self.pads.clear();
        self.keyboard = None;
        self.output = Output::default();
        self.session = session;
        let Some(session) = &self.session else {
            log::info!("mapper: off");
            return;
        };
        let keyboard = profile::read(&session.data, Section::Keyboard);
        let mut players: Vec<u32> = session.players.values().copied().collect();
        players.sort_unstable();
        players.dedup();
        let profiles: Vec<Profile> = players
            .iter()
            .map(|player| profile::read(&session.data, Section::Player(*player)))
            .chain([keyboard.clone()])
            .collect();
        self.output = Output::for_profiles(&profiles);
        let keyboard = bind::keyboard(&keyboard);
        self.keyboard = (!keyboard.is_empty()).then(|| Engine::new(keyboard));
        log::info!(
            "mapper: {} for {} player(s){}",
            session.profile.display(),
            players.len(),
            if self.keyboard.is_some() {
                " and the keyboards"
            } else {
                ""
            }
        );
        for (path, device) in devices {
            self.attach(path, device);
        }
    }

    /// A controller to map, if it is one of the game's.
    pub fn attach(&mut self, path: &Path, device: &Device) {
        let Some(session) = &self.session else {
            return;
        };
        let Some(&player) = session.players.get(&resolved(path)) else {
            return;
        };
        let profile = profile::read(&session.data, Section::Player(player));
        if profile.is_empty() {
            return;
        }
        let Some(mappings) = bind::pad(&profile, device) else {
            log::warn!(
                "{} (player {player}): not in es_input.cfg, not mapped",
                path.display()
            );
            return;
        };
        if mappings.is_empty() {
            return;
        }
        log::info!(
            "mapping {} as player {player}: {} combination(s), {} sequence(s), {} stick(s)",
            path.display(),
            mappings.combos.len(),
            mappings.sequences.len(),
            mappings.sticks.len()
        );
        self.pads.insert(
            path.to_owned(),
            Pad {
                engine: Engine::new(mappings),
                ranges: Ranges::of(device),
                hats: HashMap::new(),
                axes: HashMap::new(),
            },
        );
    }

    /// A controller gone: what it held is let go.
    pub fn detach(&mut self, path: &Path) {
        if let Some(mut pad) = self.pads.remove(path) {
            pad.engine.release_all(Instant::now(), &mut self.output);
        }
    }

    /// A keyboard gone: its keys will never come up, so let go of all.
    pub fn keyboard_gone(&mut self) {
        if let Some(engine) = &mut self.keyboard {
            engine.release_all(Instant::now(), &mut self.output);
        }
    }

    /// Let go of everything (the overlay takes the controllers).
    pub fn release_all(&mut self) {
        let now = Instant::now();
        for pad in self.pads.values_mut() {
            pad.engine.release_all(now, &mut self.output);
            pad.hats.clear();
            pad.axes.clear();
        }
        if let Some(engine) = &mut self.keyboard {
            engine.release_all(now, &mut self.output);
        }
    }

    /// An event of a controller.
    pub fn pad_event(&mut self, path: &Path, event: &InputEvent) {
        let Some(pad) = self.pads.get_mut(path) else {
            return;
        };
        let now = Instant::now();
        let output = &mut self.output;
        match event.destructure() {
            EventSummary::Key(_, key, 1) => pad.engine.press(Code::from(key.code()), now, output),
            EventSummary::Key(_, key, 0) => pad.engine.release(Code::from(key.code()), now, output),
            EventSummary::AbsoluteAxis(_, axis, value) => {
                let (previous, current) = if let Some((hat, vertical)) = bind::hat_of(axis.0) {
                    let (position, pressed) = pad.hats.entry(hat).or_default();
                    if vertical {
                        position.1 = value;
                    } else {
                        position.0 = value;
                    }
                    let current = bind::hat_code(hat, *position);
                    (std::mem::replace(pressed, current), current)
                } else {
                    let value = pad.ranges.normalize(axis.0, value);
                    pad.engine.set_axis(axis.0, value);
                    let current = bind::axis_code(axis.0, value);
                    let pressed = pad.axes.entry(axis.0).or_default();
                    (std::mem::replace(pressed, current), current)
                };
                if previous != current {
                    if let Some(code) = previous {
                        pad.engine.release(code, now, output);
                    }
                    if let Some(code) = current {
                        pad.engine.press(code, now, output);
                    }
                }
            }
            _ => {}
        }
    }

    /// An event of a keyboard (any of them).
    pub fn key_event(&mut self, event: &InputEvent) {
        let Some(engine) = &mut self.keyboard else {
            return;
        };
        let now = Instant::now();
        // 2 is a repeat: the key is already down
        match event.destructure() {
            EventSummary::Key(_, key, 1) => {
                engine.press(
                    Code::from(bind::same_key(key).code()),
                    now,
                    &mut self.output,
                );
            }
            EventSummary::Key(_, key, 0) => {
                engine.release(
                    Code::from(bind::same_key(key).code()),
                    now,
                    &mut self.output,
                );
            }
            _ => {}
        }
    }

    /// Whether `tick` has something to do.
    pub fn needs_tick(&self) -> bool {
        self.pads.values().any(|pad| pad.engine.needs_tick())
            || self.keyboard.as_ref().is_some_and(Engine::needs_tick)
    }

    /// Fire what has been held long enough, move the pointer and the wheel;
    /// whether to come back in a `TICK`.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = self.last_tick.map_or(TICK, |last| {
            now.saturating_duration_since(last).min(TICK * 10)
        });
        for pad in self.pads.values_mut() {
            pad.engine.tick(now, elapsed, &mut self.output);
        }
        if let Some(engine) = &mut self.keyboard {
            engine.tick(now, elapsed, &mut self.output);
        }
        let again = self.needs_tick();
        self.last_tick = again.then_some(now);
        again
    }

    /// Forget the commands that ended.
    pub fn reap(&mut self) {
        self.output.reap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_names_its_profile_and_players() {
        let dir = std::env::temp_dir().join(format!("inputd-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let profile = dir.join("session.keys");
        std::fs::write(&profile, r#"{"actions_player1": []}"#).unwrap();
        // no pad_profile (a launcher that runs gamepadly itself): nothing
        std::fs::write(
            dir.join("session.json"),
            r#"{"controllers": [{"player": 1, "device": "/dev/input/event7"}]}"#,
        )
        .unwrap();
        assert_eq!(Session::load(&dir), None);
        std::fs::write(
            dir.join("session.json"),
            format!(
                r#"{{"pad_profile": "{}", "controllers": [
                    {{"player": 1, "device": "/dev/input/event7"}},
                    {{"player": 2, "device": "/dev/input/event9"}}
                ]}}"#,
                profile.display()
            ),
        )
        .unwrap();
        let session = Session::load(&dir).unwrap();
        assert_eq!(session.players[Path::new("/dev/input/event9")], 2);
        assert!(session.data.contains_key("actions_player1"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
