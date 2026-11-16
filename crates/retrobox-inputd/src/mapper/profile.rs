//! Pad-to-keyboard profiles: evmapy's format, with gamepadly's additions.
//!
//! A profile is a JSON object whose `actions_player<N>` list (or `mappings`,
//! for any player) holds the mappings of player N, and `actions_keyboard`
//! the keyboard's. Every mapping has a `trigger` (one or more
//! EmulationStation input names, or evdev key names for the keyboard), a
//! `type` and the type's own fields:
//!
//! * `key`: `target` is a key chord (`KEY_*`) or a mouse button (`BTN_*`),
//!   held while the trigger is held;
//! * `exec`: `target` is a shell command line, or a list of them, run when
//!   the trigger fires;
//! * `mouse` / `scroll`: an analog stick moves the pointer or the wheel
//!   (`speed`, `deadzone`, `acceleration`).
//!
//! As in evmapy, `key` and `exec` mappings take a `mode` (`all`, the
//! default: the trigger is a combination; `any`: each input on its own;
//! `sequence`: one input after the other) and a `hold` (seconds the trigger
//! is held before it fires). Gamepadly added `condition` (inputs held as
//! well), `cooldown` (seconds before it fires again; the profile's own
//! `cooldown` for combinations), `tap` (a single input that fires when let
//! go, if nothing else was pressed and it was not held long) and
//! `_disabled`. Every mapping that cannot be used is left out with a
//! warning, as gamepadly does.

use std::str::FromStr as _;
use std::time::Duration;

use evdev::KeyCode;
use serde_json::{Map, Value};

/// The cooldown of combinations when the profile sets none.
const DEFAULT_COOLDOWN: f64 = 0.5;

/// The sections of the keyboard's mappings.
pub const KEYBOARD_SECTION: &str = "actions_keyboard";

/// What a mapping does when its trigger fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Keys held down while the trigger is (pressed in order, let go in
    /// reverse order).
    Keys(Vec<KeyCode>),
    /// A mouse button held down while the trigger is.
    Click(KeyCode),
    /// Shell command lines, run in the background.
    Exec(Vec<String>),
}

/// A combination of inputs (one or more) and what it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Hotkey {
    pub trigger: Vec<String>,
    pub action: Action,
    pub cooldown: Duration,
    pub hold: Duration,
    pub tap: bool,
}

/// Inputs pressed one after the other.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    pub trigger: Vec<String>,
    pub action: Action,
    pub cooldown: Duration,
}

/// What a stick moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Pointer,
    Wheel,
}

/// A stick that moves the pointer or the wheel.
#[derive(Debug, Clone, PartialEq)]
pub struct Stick {
    pub motion: Motion,
    /// The horizontal axis (optional for the wheel) and the vertical one.
    pub axis_x: Option<String>,
    pub axis_y: String,
    pub speed: f32,
    pub deadzone: f32,
    pub acceleration: f32,
    /// Inputs that have to be held for it to move anything.
    pub condition: Vec<String>,
}

/// The mappings of one player, or of the keyboard.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    pub hotkeys: Vec<Hotkey>,
    pub sequences: Vec<Sequence>,
    pub sticks: Vec<Stick>,
}

impl Profile {
    pub fn is_empty(&self) -> bool {
        self.hotkeys.is_empty() && self.sequences.is_empty() && self.sticks.is_empty()
    }
}

/// Whose mappings to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `actions_player<N>`, else `mappings`.
    Player(u32),
    /// `actions_keyboard` (no fallback).
    Keyboard,
}

/// The mappings of `section` in a profile (its JSON object). Mappings that
/// cannot be used are left out (and logged).
pub fn read(profile: &Map<String, Value>, section: Section) -> Profile {
    let global_cooldown = profile
        .get("cooldown")
        .and_then(number)
        .unwrap_or(DEFAULT_COOLDOWN);
    let entries = match section {
        Section::Player(player) => profile
            .get(&format!("actions_player{player}"))
            .filter(|entries| truthy(entries))
            .or_else(|| profile.get("mappings")),
        Section::Keyboard => profile.get(KEYBOARD_SECTION),
    };
    let entries = entries
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut read = Profile::default();
    for (index, entry) in entries.iter().enumerate() {
        if let Err(why) = add(&mut read, entry, global_cooldown) {
            log::warn!("mapping #{index}: {why}: ignored");
        }
    }
    read
}

/// Add one mapping.
fn add(profile: &mut Profile, entry: &Value, global_cooldown: f64) -> Result<(), String> {
    let entry = entry.as_object().ok_or("not an object")?;
    if entry.get("_disabled").is_some_and(truthy) {
        return Ok(());
    }
    let kind = entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let triggers = names(entry.get("trigger"));
    if kind.is_empty() || triggers.is_empty() {
        return Err("no type or trigger".into());
    }
    let condition = names(entry.get("condition"));
    match kind {
        "key" | "exec" => add_triggered(profile, entry, triggers, &condition, global_cooldown),
        "mouse" => add_stick(profile, entry, Motion::Pointer, &triggers, condition),
        "scroll" => add_stick(profile, entry, Motion::Wheel, &triggers, condition),
        _ => Err(format!("unknown type {kind:?}")),
    }
}

/// A `key` or `exec` mapping: combinations, or a sequence.
fn add_triggered(
    profile: &mut Profile,
    entry: &Map<String, Value>,
    triggers: Vec<String>,
    condition: &[String],
    global_cooldown: f64,
) -> Result<(), String> {
    let mode = entry.get("mode").map_or_else(|| "all".to_owned(), text);
    let hold = entry
        .get("hold")
        .map_or(Some(0.0), number)
        .ok_or("bad hold")?;
    let tap = entry.get("tap").is_some_and(truthy);
    if !["all", "any", "sequence"].contains(&mode.as_str()) {
        return Err(format!("invalid mode {mode:?}"));
    }
    if hold < 0.0 {
        return Err("negative hold".into());
    }
    if mode == "sequence" && hold > 0.0 {
        return Err("a sequence cannot be held".into());
    }
    if tap && (hold > 0.0 || mode == "sequence") {
        return Err("a tap cannot be held nor a sequence".into());
    }
    let action = action(entry)?;
    let cooldown = |size: usize| -> Result<Duration, String> {
        let seconds = match entry.get("cooldown") {
            Some(value) => number(value).ok_or("bad cooldown")?,
            None if size == 1 => 0.0,
            None => global_cooldown,
        };
        seconds_duration(seconds)
    };
    if mode == "sequence" {
        if triggers.len() < 2 {
            return Err("a sequence needs two inputs or more".into());
        }
        let cooldown = cooldown(triggers.len())?;
        // the same sequence again replaces the first one
        let sequence = Sequence {
            trigger: triggers,
            action,
            cooldown,
        };
        match profile
            .sequences
            .iter_mut()
            .find(|known| known.trigger == sequence.trigger)
        {
            Some(known) => *known = sequence,
            None => profile.sequences.push(sequence),
        }
        return Ok(());
    }
    if has_duplicates(&triggers) {
        return Err("an input twice in the trigger".into());
    }
    // "any": each input of the trigger is a combination of its own
    let combos: Vec<Vec<String>> = if mode == "any" {
        triggers.into_iter().map(|name| vec![name]).collect()
    } else {
        vec![triggers]
    };
    for mut combo in combos {
        combo.extend(condition.iter().cloned());
        if tap && combo.len() != 1 {
            return Err("a tap is a single input".into());
        }
        let hotkey = Hotkey {
            cooldown: cooldown(combo.len())?,
            trigger: combo,
            action: action.clone(),
            hold: seconds_duration(hold)?,
            tap,
        };
        match profile
            .hotkeys
            .iter_mut()
            .find(|known| known.trigger == hotkey.trigger && known.tap == hotkey.tap)
        {
            Some(known) => {
                log::warn!(
                    "combination {:?} twice: the last one is kept",
                    hotkey.trigger
                );
                *known = hotkey;
            }
            None => profile.hotkeys.push(hotkey),
        }
    }
    Ok(())
}

/// The action of a `key` or `exec` mapping.
fn action(entry: &Map<String, Value>) -> Result<Action, String> {
    let targets = names(entry.get("target"));
    if targets.is_empty() {
        return Err("no target".into());
    }
    if entry.get("type").and_then(Value::as_str) == Some("exec") {
        return Ok(Action::Exec(targets));
    }
    if has_duplicates(&targets) {
        return Err("a key twice in the target".into());
    }
    let codes = targets
        .iter()
        .map(|name| key_code(name).ok_or_else(|| format!("unknown key {name:?}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match codes.as_slice() {
        [button] if targets[0].starts_with("BTN_") => Action::Click(*button),
        _ => Action::Keys(codes),
    })
}

/// A `mouse` or `scroll` mapping.
fn add_stick(
    profile: &mut Profile,
    entry: &Map<String, Value>,
    motion: Motion,
    triggers: &[String],
    condition: Vec<String>,
) -> Result<(), String> {
    // a whole stick, or its axes
    let whole = |name: &str| match name {
        "joystick1" => Some(("joystick1left", "joystick1up")),
        "joystick2" => Some(("joystick2left", "joystick2up")),
        _ => None,
    };
    let (axis_x, axis_y) = match (motion, triggers) {
        (Motion::Pointer, [stick]) => {
            let (x, y) = whole(stick).ok_or("the pointer needs a whole stick or two axes")?;
            (Some(x.to_owned()), y.to_owned())
        }
        (Motion::Pointer, [x, y]) => (Some(x.clone()), y.clone()),
        (Motion::Wheel, [stick]) => (
            None,
            whole(stick).map_or_else(|| stick.clone(), |(_, y)| y.to_owned()),
        ),
        // the wheel's vertical axis comes first
        (Motion::Wheel, [vertical, horizontal]) => (Some(horizontal.clone()), vertical.clone()),
        _ => return Err("one or two axes".into()),
    };
    let (speed, acceleration) = match motion {
        Motion::Pointer => (800.0, 1.5),
        Motion::Wheel => (8.0, 1.2),
    };
    let field = |name: &str, default: f64| -> Result<f32, String> {
        let value = entry.get(name).map_or(Some(default), number);
        value
            .map(|value| value as f32)
            .ok_or_else(|| format!("bad {name}"))
    };
    profile.sticks.push(Stick {
        motion,
        axis_x,
        axis_y,
        speed: field("speed", speed)?,
        deadzone: field("deadzone", 0.15)?,
        acceleration: field("acceleration", acceleration)?,
        condition,
    });
    Ok(())
}

/// An evdev key or button by name (`KEY_F4`, `BTN_LEFT`).
pub fn key_code(name: &str) -> Option<KeyCode> {
    KeyCode::from_str(name.trim()).ok()
}

/// A JSON value as a list of names, as gamepadly reads `trigger`, `target`
/// and `condition`: nothing for null or a false value, a string as itself,
/// a list as its items.
fn names(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().map(text).collect(),
        Some(value) if !truthy(value) => Vec::new(),
        Some(value) => vec![text(value)],
    }
}

/// A JSON value as text (a string without its quotes).
fn text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// A number, or a string holding one.
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .filter(|number| number.is_finite())
}

/// Python's truth value of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

fn seconds_duration(seconds: f64) -> Result<Duration, String> {
    Duration::try_from_secs_f64(seconds.max(0.0)).map_err(|err| err.to_string())
}

fn has_duplicates(names: &[String]) -> bool {
    names
        .iter()
        .enumerate()
        .any(|(index, name)| names[..index].contains(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(json: &str, section: Section) -> Profile {
        let value: Value = serde_json::from_str(json).unwrap();
        read(value.as_object().unwrap(), section)
    }

    #[test]
    fn reads_combinations_with_their_cooldowns() {
        let read = profile(
            r#"{"cooldown": 0.3, "actions_player1": [
                {"trigger": ["hotkey", "start"], "type": "key", "target": ["KEY_LEFTALT", "KEY_F4"]},
                {"trigger": "a", "type": "key", "target": "KEY_X"},
                {"trigger": ["b"], "type": "exec", "target": "echo hi", "hold": "1.5"},
                {"trigger": ["r3"], "type": "key", "target": "BTN_LEFT", "condition": "hotkey"}
            ]}"#,
            Section::Player(1),
        );
        assert_eq!(read.hotkeys.len(), 4);
        let exit = &read.hotkeys[0];
        assert_eq!(exit.cooldown, Duration::from_millis(300));
        assert_eq!(
            exit.action,
            Action::Keys(vec![KeyCode::KEY_LEFTALT, KeyCode::KEY_F4])
        );
        assert_eq!(read.hotkeys[1].cooldown, Duration::ZERO);
        assert_eq!(read.hotkeys[2].hold, Duration::from_millis(1500));
        assert_eq!(read.hotkeys[2].action, Action::Exec(vec!["echo hi".into()]));
        assert_eq!(read.hotkeys[3].trigger, ["r3", "hotkey"]);
        assert_eq!(read.hotkeys[3].action, Action::Click(KeyCode::BTN_LEFT));
    }

    #[test]
    fn modes_taps_and_bad_mappings() {
        let read = profile(
            r#"{"mappings": [
                {"trigger": ["l2", "r2"], "type": "key", "target": "KEY_A", "mode": "any"},
                {"trigger": ["up", "up", "down"], "type": "exec", "target": "x", "mode": "sequence"},
                {"trigger": "hotkey", "type": "exec", "target": "y", "tap": true},
                {"trigger": ["a", "b"], "type": "key", "target": "KEY_A", "tap": true},
                {"trigger": "a", "type": "key", "target": "KEY_NOPE"},
                {"trigger": "a", "type": "key", "target": "KEY_A", "_disabled": true},
                {"trigger": "a", "type": "key", "target": "KEY_A", "mode": "sequence"},
                {"trigger": "a", "type": "magic"}
            ]}"#,
            Section::Player(3),
        );
        let triggers: Vec<_> = read
            .hotkeys
            .iter()
            .map(|hotkey| hotkey.trigger.clone())
            .collect();
        assert_eq!(triggers, [vec!["l2"], vec!["r2"], vec!["hotkey"]]);
        assert!(read.hotkeys[2].tap);
        assert_eq!(read.sequences.len(), 1);
        assert_eq!(read.sequences[0].trigger, ["up", "up", "down"]);
        assert_eq!(read.sequences[0].cooldown, Duration::from_millis(500));
    }

    #[test]
    fn sticks_and_sections() {
        let json = r#"{
            "actions_player1": [],
            "mappings": [
                {"trigger": "joystick2", "type": "mouse", "speed": 600, "condition": ["hotkey"]},
                {"trigger": ["joystick1up"], "type": "scroll"}
            ],
            "actions_keyboard": [
                {"trigger": ["KEY_LEFTSHIFT", "KEY_ESC"], "type": "exec", "target": "retrobox-emukill"}
            ]
        }"#;
        // an empty player list falls back to "mappings"
        let read = profile(json, Section::Player(1));
        assert_eq!(read.sticks.len(), 2);
        let pointer = &read.sticks[0];
        assert_eq!(pointer.motion, Motion::Pointer);
        assert_eq!(pointer.axis_x.as_deref(), Some("joystick2left"));
        assert_eq!(pointer.axis_y, "joystick2up");
        assert!((pointer.speed - 600.0).abs() < f32::EPSILON);
        assert_eq!(pointer.condition, ["hotkey"]);
        assert_eq!(read.sticks[1].axis_x, None);
        assert!((read.sticks[1].speed - 8.0).abs() < f32::EPSILON);
        let keyboard = profile(json, Section::Keyboard);
        assert_eq!(keyboard.hotkeys.len(), 1);
        assert!(profile(r#"{"mappings": []}"#, Section::Keyboard).is_empty());
    }
}
