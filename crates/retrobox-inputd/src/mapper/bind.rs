//! Bind a profile's input names to the inputs of a device.
//!
//! A controller's names (`a`, `hotkey`, `joystick1up`...) come from its
//! `inputConfig` in `es_input.cfg`, which numbers them as SDL does
//! (EmulationStation reads the controllers through SDL's evdev driver):
//! buttons in the order of their key codes from `BTN_JOYSTICK` up, axes in
//! the order of their codes leaving out the hats, hats by their number. The
//! engine numbers the inputs on its own (see `Code`): a button by its key
//! code, a hat direction and an axis direction above them.
//!
//! The keyboard's names are evdev key names; a right-hand modifier counts
//! as the left-hand one, so `KEY_LEFTSHIFT` means either Shift.

use std::collections::{BTreeSet, HashMap};

use evdev::{AbsoluteAxisCode, Device, KeyCode};
use retrobox_input::es_input::{EsInput, EsKind, Layout};

use super::engine::{Axis, Code, Combo, Mappings, Sequence, Stick};
use retrobox_input::pad_profile::{self as profile, Profile};

/// Where the hat directions start in the engine's numbering.
const HAT_BASE: Code = 0x1000;
/// Where the axis directions start.
const AXIS_BASE: Code = 0x2000;
/// The hat axes: `ABS_HAT0X` to `ABS_HAT3Y`.
const HATS: std::ops::RangeInclusive<u16> = 0x10..=0x17;
/// Past the last absolute axis code.
const ABS_COUNT: u16 = 0x40;
/// How far an axis is pushed to count as pressed (of -1 to 1).
pub const AXIS_THRESHOLD: f32 = 0.5;

/// A hat direction, as SDL numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HatDirection {
    Up = 1,
    Right = 2,
    Down = 4,
    Left = 8,
}

impl HatDirection {
    fn from_sdl(value: i32) -> Option<Self> {
        Some(match value {
            1 => Self::Up,
            2 => Self::Right,
            4 => Self::Down,
            8 => Self::Left,
            _ => return None,
        })
    }

    const fn index(self) -> Code {
        match self {
            Self::Up => 0,
            Self::Right => 1,
            Self::Down => 2,
            Self::Left => 3,
        }
    }
}

/// An input of a controller.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PadInput {
    Button(KeyCode),
    Hat(u16, HatDirection),
    /// An axis and the direction it is pushed to (-1 or 1).
    Axis(u16, i32),
}

impl PadInput {
    fn code(self) -> Code {
        match self {
            Self::Button(key) => Code::from(key.code()),
            Self::Hat(hat, direction) => HAT_BASE + Code::from(hat) * 4 + direction.index(),
            Self::Axis(axis, direction) => {
                AXIS_BASE + Code::from(axis) * 2 + Code::from(direction > 0)
            }
        }
    }
}

/// The engine's number of a hat direction from the hat's position (x, y as
/// evdev reports them): one direction at a time, the horizontal one first,
/// as gamepadly did.
pub fn hat_code(hat: u16, (x, y): (i32, i32)) -> Option<Code> {
    let direction = match (x.signum(), y.signum()) {
        (1, _) => HatDirection::Right,
        (-1, _) => HatDirection::Left,
        (_, -1) => HatDirection::Up,
        (_, 1) => HatDirection::Down,
        _ => return None,
    };
    Some(PadInput::Hat(hat, direction).code())
}

/// The engine's number of an axis pushed past the threshold, if it is.
pub fn axis_code(axis: u16, value: f32) -> Option<Code> {
    if value > AXIS_THRESHOLD {
        Some(PadInput::Axis(axis, 1).code())
    } else if value < -AXIS_THRESHOLD {
        Some(PadInput::Axis(axis, -1).code())
    } else {
        None
    }
}

/// The hat an axis belongs to, and whether it is its vertical one.
pub fn hat_of(axis: u16) -> Option<(u16, bool)> {
    HATS.contains(&axis)
        .then(|| ((axis - HATS.start()) / 2, (axis - HATS.start()) % 2 == 1))
}

/// A controller's inputs by EmulationStation name.
type Names = HashMap<String, PadInput>;

/// The names of a controller's inputs, from its `inputConfig`.
fn pad_names(inputs: &[EsInput], buttons: &[KeyCode], axes: &[u16]) -> Names {
    let mut names: Names = inputs
        .iter()
        .filter_map(|input| {
            let pad_input = match input.kind {
                EsKind::Button => PadInput::Button(*buttons.get(usize::from(input.id))?),
                EsKind::Hat => PadInput::Hat(input.id, HatDirection::from_sdl(input.value)?),
                EsKind::Axis => {
                    PadInput::Axis(*axes.get(usize::from(input.id))?, input.value.signum())
                }
            };
            Some((input.name.clone(), pad_input))
        })
        .collect();
    // EmulationStation keeps one direction of each stick axis; the profiles
    // name the other one too
    for (name, opposite) in [
        ("joystick1up", "joystick1down"),
        ("joystick1left", "joystick1right"),
        ("joystick2up", "joystick2down"),
        ("joystick2left", "joystick2right"),
    ] {
        if let Some(PadInput::Axis(axis, direction)) = names.get(name).copied() {
            names
                .entry(opposite.to_owned())
                .or_insert(PadInput::Axis(axis, -direction));
        }
    }
    names
}

/// A controller's absolute axes in SDL's numbering (hats left out).
fn sdl_axes(device: &Device) -> Vec<u16> {
    device
        .supported_absolute_axes()
        .map_or_else(Vec::new, |axes| {
            (0..ABS_COUNT)
                .filter(|code| !HATS.contains(code) && axes.contains(AbsoluteAxisCode(*code)))
                .collect()
        })
}

/// How a controller's axes read: their range, to bring them to -1..1.
#[derive(Debug, Clone, Default)]
pub struct Ranges(HashMap<u16, (i32, i32)>);

impl Ranges {
    pub fn of(device: &Device) -> Self {
        Self(
            device
                .get_absinfo()
                .map(|infos| {
                    infos
                        .filter(|(_, info)| info.maximum() > info.minimum())
                        .map(|(axis, info)| (axis.0, (info.minimum(), info.maximum())))
                        .collect()
                })
                .unwrap_or_default(),
        )
    }

    /// A raw value of `axis` in -1..1, as SDL scales it.
    pub fn normalize(&self, axis: u16, value: i32) -> f32 {
        self.0.get(&axis).map_or(0.0, |&(min, max)| {
            let span = f64::from(max) - f64::from(min);
            ((f64::from(value) - f64::from(min)) * 2.0 / span - 1.0).clamp(-1.0, 1.0) as f32
        })
    }
}

/// A controller's mappings: the player's profile bound to its inputs, as
/// its `es_input.cfg` layout names them.
pub fn pad(profile: &Profile, layout: &Layout, device: &Device) -> Mappings {
    let names = pad_names(layout.inputs(), layout.buttons(), &sdl_axes(device));
    bind_pad(profile, &names, |name| layout.has_two_names(name))
}

/// `two_names`: whether a name's input has another name too.
fn bind_pad(profile: &Profile, names: &Names, two_names: impl Fn(&str) -> bool) -> Mappings {
    let code = |name: &String| names.get(name).map(|input| input.code());
    let mut mappings = bind(profile, |trigger| {
        let codes: Option<Vec<Code>> = trigger.iter().map(code).collect();
        if codes.is_none() {
            log::warn!("{trigger:?}: not in this controller's es_input.cfg: ignored");
        }
        codes
    });
    // the hotkey cannot tap: a tap on one of its own opens the overlay, and
    // one on a game's button (Select, the same input under two names) would
    // take the game's button; nor can that button
    let untappable: BTreeSet<Code> = names
        .iter()
        .filter(|(name, _)| *name == "hotkey" || two_names(name))
        .map(|(_, input)| input.code())
        .collect();
    mappings.combos.retain(|combo| {
        let keep = !combo.tap || combo.codes.is_disjoint(&untappable);
        if !keep {
            log::info!(
                "tap on {:x?} ignored: the game's or the overlay's",
                combo.codes
            );
        }
        keep
    });
    let axis = |name: &str, motion_scale: i32| match names.get(name) {
        Some(PadInput::Axis(code, direction)) => Some(Axis {
            code: *code,
            scale: (direction * motion_scale) as f32,
        }),
        _ => None,
    };
    for stick in &profile.sticks {
        // the pointer moves against the direction the name says (up is
        // negative), the wheel with it (up scrolls up)
        let scale = match stick.motion {
            profile::Motion::Pointer => -1,
            profile::Motion::Wheel => 1,
        };
        let Some(y) = axis(&stick.axis_y, scale) else {
            log::warn!("{}: not an axis of this controller: ignored", stick.axis_y);
            continue;
        };
        let x = stick.axis_x.as_deref().and_then(|name| axis(name, scale));
        if stick.motion == profile::Motion::Pointer && x.is_none() {
            log::warn!(
                "{:?}: not an axis of this controller: ignored",
                stick.axis_x
            );
            continue;
        }
        let Some(condition) = stick.condition.iter().map(code).collect::<Option<_>>() else {
            log::warn!(
                "condition {:?}: not on this controller: ignored",
                stick.condition
            );
            continue;
        };
        mappings.sticks.push(Stick {
            motion: stick.motion,
            x,
            y,
            speed: stick.speed,
            deadzone: stick.deadzone,
            acceleration: stick.acceleration,
            condition,
        });
    }
    mappings
}

/// A right-hand modifier as the left-hand one.
pub fn same_key(key: KeyCode) -> KeyCode {
    match key {
        KeyCode::KEY_RIGHTSHIFT => KeyCode::KEY_LEFTSHIFT,
        KeyCode::KEY_RIGHTCTRL => KeyCode::KEY_LEFTCTRL,
        KeyCode::KEY_RIGHTALT => KeyCode::KEY_LEFTALT,
        KeyCode::KEY_RIGHTMETA => KeyCode::KEY_LEFTMETA,
        other => other,
    }
}

/// The keyboards' mappings (no sticks there).
pub fn keyboard(profile: &Profile) -> Mappings {
    if !profile.sticks.is_empty() {
        log::warn!("mouse and scroll mappings of the keyboard are ignored");
    }
    bind(profile, |trigger| {
        let codes: Option<Vec<Code>> = trigger
            .iter()
            .map(|name| profile::key_code(name).map(|key| Code::from(same_key(key).code())))
            .collect();
        if codes.is_none() {
            log::warn!("keyboard {trigger:?}: unknown key: ignored");
        }
        codes
    })
}

/// The combinations and sequences of a profile, with `codes` naming the
/// inputs (None leaves the mapping out).
fn bind(profile: &Profile, codes: impl Fn(&[String]) -> Option<Vec<Code>>) -> Mappings {
    let mut combos: Vec<Combo> = Vec::new();
    for hotkey in &profile.hotkeys {
        let Some(list) = codes(&hotkey.trigger) else {
            continue;
        };
        let set: BTreeSet<Code> = list.iter().copied().collect();
        // two names of one physical input cannot be told apart
        if set.len() < hotkey.trigger.len() {
            log::debug!("{:?}: the same input twice: ignored", hotkey.trigger);
            continue;
        }
        let combo = Combo {
            codes: set,
            action: hotkey.action.clone(),
            cooldown: hotkey.cooldown,
            hold: hotkey.hold,
            tap: hotkey.tap,
        };
        match combos
            .iter_mut()
            .find(|known| known.codes == combo.codes && known.tap == combo.tap)
        {
            Some(known) => *known = combo,
            None => combos.push(combo),
        }
    }
    let sequences = profile
        .sequences
        .iter()
        .filter_map(|sequence| {
            Some(Sequence {
                codes: codes(&sequence.trigger)?,
                action: sequence.action.clone(),
                cooldown: sequence.cooldown,
            })
        })
        .collect();
    Mappings {
        combos,
        sequences,
        sticks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::profile::{Section, read};

    fn es(name: &str, kind: EsKind, id: u16, value: i32) -> EsInput {
        EsInput {
            name: name.into(),
            kind,
            id,
            value,
        }
    }

    /// An xpad-like controller: A B X Y TL TR SELECT START MODE THUMBL THUMBR
    /// as SDL numbers them; axes X Y Z RX RY RZ; the d-pad a hat.
    fn names() -> Names {
        let buttons: Vec<KeyCode> = [
            0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e,
        ]
        .into_iter()
        .map(KeyCode::new)
        .collect();
        let axes = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
        let inputs = [
            es("b", EsKind::Button, 0, 1),
            es("a", EsKind::Button, 1, 1),
            es("select", EsKind::Button, 6, 1),
            es("start", EsKind::Button, 7, 1),
            es("hotkey", EsKind::Button, 8, 1),
            es("up", EsKind::Hat, 0, 1),
            es("left", EsKind::Hat, 0, 8),
            es("joystick2left", EsKind::Axis, 3, -1),
            es("joystick2up", EsKind::Axis, 4, -1),
            es("l2", EsKind::Axis, 2, 1),
        ];
        pad_names(&inputs, &buttons, &axes)
    }

    #[test]
    fn names_are_bound_to_evdev_inputs() {
        let names = names();
        assert_eq!(names["hotkey"], PadInput::Button(KeyCode::BTN_MODE));
        assert_eq!(names["start"], PadInput::Button(KeyCode::BTN_START));
        assert_eq!(names["up"], PadInput::Hat(0, HatDirection::Up));
        // the other direction of a stick axis is derived
        assert_eq!(names["joystick2right"], PadInput::Axis(3, 1));
        assert_eq!(names["joystick2down"], PadInput::Axis(4, 1));
        assert_eq!(hat_code(0, (0, -1)), Some(names["up"].code()));
        assert_eq!(
            hat_code(0, (-1, -1)),
            Some(names["left"].code()),
            "horizontal first"
        );
        assert_eq!(axis_code(3, 0.9), Some(names["joystick2right"].code()));
        assert_eq!(axis_code(3, 0.2), None);
        assert_eq!(hat_of(0x11), Some((0, true)));
        assert_eq!(hat_of(0x03), None);
    }

    #[test]
    fn a_profile_binds_to_a_controller() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"actions_player1": [
                {"trigger": ["hotkey", "start"], "type": "exec", "target": "retrobox-emukill"},
                {"trigger": "joystick2", "type": "mouse", "condition": "hotkey"},
                {"trigger": ["hotkey", "r3"], "type": "key", "target": "BTN_LEFT"},
                {"trigger": "l2", "type": "key", "target": "KEY_Q"}
            ]}"#,
        )
        .unwrap();
        let profile = read(value.as_object().unwrap(), Section::Player(1));
        let mappings = bind_pad(&profile, &names(), |_| false);
        // r3 is not in this controller's es_input.cfg
        assert_eq!(mappings.combos.len(), 2);
        assert_eq!(
            mappings.combos[0].codes,
            [0x13b, 0x13c].into_iter().collect::<BTreeSet<_>>()
        );
        assert_eq!(
            mappings.combos[1].codes,
            [AXIS_BASE + 2 * 2 + 1].into_iter().collect()
        );
        let stick = &mappings.sticks[0];
        assert_eq!(
            stick.x,
            Some(Axis {
                code: 3,
                scale: 1.0
            })
        );
        assert_eq!(
            stick.y,
            Axis {
                code: 4,
                scale: 1.0
            }
        );
        assert_eq!(stick.condition, [0x13c].into_iter().collect());
    }

    #[test]
    fn the_hotkey_cannot_tap() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"mappings": [
                {"trigger": "hotkey", "type": "exec", "target": "x", "tap": true},
                {"trigger": "select", "type": "exec", "target": "y", "tap": true}
            ]}"#,
        )
        .unwrap();
        let profile = read(value.as_object().unwrap(), Section::Player(1));
        assert_eq!(bind_pad(&profile, &names(), |_| false).combos.len(), 1);
    }

    #[test]
    fn a_shared_button_cannot_tap() {
        let mut names = names();
        names.insert("hotkey".into(), names["select"]);
        let two_names = |name: &str| matches!(name, "hotkey" | "select");
        let value: serde_json::Value = serde_json::from_str(
            r#"{"mappings": [{"trigger": "hotkey", "type": "exec", "target": "x", "tap": true}]}"#,
        )
        .unwrap();
        let profile = read(value.as_object().unwrap(), Section::Player(1));
        assert_eq!(bind_pad(&profile, &names, two_names).combos.len(), 0);
    }

    #[test]
    fn keyboard_modifiers_count_on_either_side() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"actions_keyboard": [
                {"trigger": ["KEY_LEFTSHIFT", "KEY_ESC"], "type": "exec", "target": "x"},
                {"trigger": ["KEY_NOPE"], "type": "exec", "target": "y"}
            ]}"#,
        )
        .unwrap();
        let mappings = keyboard(&read(value.as_object().unwrap(), Section::Keyboard));
        assert_eq!(mappings.combos.len(), 1);
        let shift = Code::from(same_key(KeyCode::KEY_RIGHTSHIFT).code());
        assert!(mappings.combos[0].codes.contains(&shift));
    }
}
