//! What a controller's (or the keyboards') inputs fire: gamepadly's engine.
//!
//! The engine knows inputs only as numbers (see `bind`): a button, a hat
//! direction or an axis direction of a controller, or a key. It is told
//! which go down and up, and what the sticks read, and drives a `Sink` with
//! the actions of its mappings:
//!
//! * the longest combination that a press completes fires (Hotkey+Start
//!   wins over Start alone); its action is held until one of its inputs is
//!   let go;
//! * a combination with a hold time waits for it, and anything else pressed
//!   meanwhile cancels it;
//! * a tap fires when its input is let go, if nothing else was pressed and
//!   it was not held longer than `TAP_MAX`;
//! * a sequence fires when its inputs are pressed one after the other;
//! * a cooldown keeps a mapping from firing again too soon;
//! * the sticks move the pointer or the wheel on every tick, past their
//!   dead zone and along an acceleration curve.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use evdev::RelativeAxisCode;

use retrobox_input::pad_profile::{Action, Motion};

/// A tap held longer than this is not a tap any more.
pub const TAP_MAX: Duration = Duration::from_millis(500);

/// An input, as the engine numbers it (see `bind`).
pub type Code = u32;

/// What the actions go to.
pub trait Sink {
    /// The trigger fired: keys and buttons go down, commands run.
    fn down(&mut self, action: &Action);
    /// The trigger was let go: keys and buttons go up.
    fn up(&mut self, action: &Action);
    /// The pointer or the wheel moved.
    fn moved(&mut self, motion: &[(RelativeAxisCode, i32)]);
}

/// A combination of inputs and what it does.
#[derive(Debug, Clone)]
pub struct Combo {
    pub codes: BTreeSet<Code>,
    pub action: Action,
    pub cooldown: Duration,
    pub hold: Duration,
    pub tap: bool,
}

/// Inputs to press one after the other.
#[derive(Debug, Clone)]
pub struct Sequence {
    pub codes: Vec<Code>,
    pub action: Action,
    pub cooldown: Duration,
}

/// One axis of a stick: its number and how its value turns into motion
/// (-1 or 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    pub code: u16,
    pub scale: f32,
}

/// A stick that moves the pointer or the wheel.
#[derive(Debug, Clone)]
pub struct Stick {
    pub motion: Motion,
    pub x: Option<Axis>,
    pub y: Axis,
    pub speed: f32,
    pub deadzone: f32,
    pub acceleration: f32,
    pub condition: BTreeSet<Code>,
}

/// Everything one engine maps.
#[derive(Debug, Clone, Default)]
pub struct Mappings {
    pub combos: Vec<Combo>,
    pub sequences: Vec<Sequence>,
    pub sticks: Vec<Stick>,
}

impl Mappings {
    pub fn is_empty(&self) -> bool {
        self.combos.is_empty() && self.sequences.is_empty() && self.sticks.is_empty()
    }
}

/// What may not fire again too soon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Fired {
    Combo(BTreeSet<Code>),
    Sequence(usize),
}

/// Map a stick value to [-1, 1] past the dead zone, along the curve.
pub fn curve(value: f32, deadzone: f32, acceleration: f32) -> f32 {
    if value.abs() <= deadzone || deadzone >= 1.0 {
        return 0.0;
    }
    let normalized = ((value.abs() - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0);
    normalized.powf(acceleration).copysign(value)
}

/// Turn the inputs of a controller (or of the keyboards) into actions.
#[derive(Debug)]
pub struct Engine {
    /// Without taps, the longest first.
    combos: Vec<Combo>,
    taps: HashMap<Code, Combo>,
    sequences: Vec<Sequence>,
    sticks: Vec<Stick>,
    pressed: BTreeSet<Code>,
    /// Combinations fired, whose action is held until one of their inputs
    /// goes up.
    active: Vec<Combo>,
    /// Combinations waiting for their hold time, and since when.
    pending: Vec<(Combo, Instant)>,
    /// Taps pressed alone so far, and when.
    armed: HashMap<Code, Instant>,
    last_fired: HashMap<Fired, Instant>,
    progress: Vec<usize>,
    /// What the sticks read (-1 to 1), by axis number.
    axes: HashMap<u16, f32>,
    /// The motion below one unit carried to the next tick: pointer, wheel.
    carry: [[f32; 2]; 2],
}

impl Engine {
    pub fn new(mappings: Mappings) -> Self {
        let (taps, mut combos): (Vec<Combo>, Vec<Combo>) =
            mappings.combos.into_iter().partition(|combo| combo.tap);
        // stable: of two as long, the one the profile has first
        combos.sort_by_key(|combo| std::cmp::Reverse(combo.codes.len()));
        let taps = taps
            .into_iter()
            .filter_map(|combo| Some((*combo.codes.first()?, combo)))
            .collect();
        Self {
            combos,
            taps,
            progress: vec![0; mappings.sequences.len()],
            sequences: mappings.sequences,
            sticks: mappings.sticks,
            pressed: BTreeSet::new(),
            active: Vec::new(),
            pending: Vec::new(),
            armed: HashMap::new(),
            last_fired: HashMap::new(),
            axes: HashMap::new(),
            carry: [[0.0; 2]; 2],
        }
    }

    /// An input went down.
    pub fn press(&mut self, code: Code, now: Instant, sink: &mut dyn Sink) {
        self.pressed.insert(code);
        // a combination waiting for its hold time is held as it is: anything
        // else pressed meanwhile cancels it
        self.pending
            .retain(|(combo, _)| combo.codes.contains(&code));
        // a tap is a tap only while nothing else is pressed
        self.armed.retain(|armed, _| *armed == code);
        if self.taps.contains_key(&code) {
            self.armed.insert(code, now);
        }
        self.advance_sequences(code, now, sink);
        let best = self.combos.iter().find(|combo| {
            combo.codes.contains(&code)
                && combo.codes.is_subset(&self.pressed)
                && !self.active.iter().any(|active| active.codes == combo.codes)
                && !self
                    .pending
                    .iter()
                    .any(|(pending, _)| pending.codes == combo.codes)
        });
        let Some(best) = best.cloned() else {
            return;
        };
        if best.hold.is_zero() {
            self.fire(best, now, sink);
        } else {
            self.pending.push((best, now));
        }
    }

    /// An input went up.
    pub fn release(&mut self, code: Code, now: Instant, sink: &mut dyn Sink) {
        self.pressed.remove(&code);
        if let Some(since) = self.armed.remove(&code)
            && now.saturating_duration_since(since) <= TAP_MAX
            && let Some(tap) = self.taps.get(&code)
            && Self::cooled_down(
                &mut self.last_fired,
                Fired::Combo(tap.codes.clone()),
                tap.cooldown,
                now,
            )
        {
            log::debug!("tap {code:#x}");
            sink.down(&tap.action);
            sink.up(&tap.action);
        }
        let (ended, kept): (Vec<Combo>, Vec<Combo>) = std::mem::take(&mut self.active)
            .into_iter()
            .partition(|combo| combo.codes.contains(&code));
        self.active = kept;
        for combo in ended {
            sink.up(&combo.action);
        }
        self.pending
            .retain(|(combo, _)| !combo.codes.contains(&code));
    }

    /// Let go of everything (the controller is gone, or taken by the
    /// overlay): every action held goes up.
    pub fn release_all(&mut self, now: Instant, sink: &mut dyn Sink) {
        for code in std::mem::take(&mut self.pressed) {
            self.release(code, now, sink);
        }
        self.armed.clear();
        self.axes.clear();
        self.carry = [[0.0; 2]; 2];
    }

    /// What a stick's axis reads now (-1 to 1).
    pub fn set_axis(&mut self, code: u16, value: f32) {
        self.axes.insert(code, value);
    }

    /// Whether `tick` has anything to do: a combination waits for its hold
    /// time, or a stick is out of its dead zone.
    pub fn needs_tick(&self) -> bool {
        !self.pending.is_empty()
            || self.sticks.iter().any(|stick| {
                stick.condition.is_subset(&self.pressed)
                    && [Some(stick.y), stick.x].iter().flatten().any(|axis| {
                        self.axes
                            .get(&axis.code)
                            .is_some_and(|value| value.abs() > stick.deadzone)
                    })
            })
    }

    /// Fire what has been held long enough, and move the pointer and the
    /// wheel for the time since the last tick (`elapsed`).
    pub fn tick(&mut self, now: Instant, elapsed: Duration, sink: &mut dyn Sink) {
        let pending = std::mem::take(&mut self.pending);
        let mut waiting = Vec::new();
        for (combo, since) in pending {
            if !combo.codes.is_subset(&self.pressed) {
                continue;
            }
            if now.saturating_duration_since(since) >= combo.hold {
                self.fire(combo, now, sink);
            } else {
                waiting.push((combo, since));
            }
        }
        // what fired meanwhile may have taken some of them
        waiting.retain(|(combo, _)| !self.active.iter().any(|active| active.codes == combo.codes));
        self.pending.extend(waiting);
        self.move_sticks(elapsed.as_secs_f32(), sink);
    }

    fn move_sticks(&mut self, seconds: f32, sink: &mut dyn Sink) {
        if self.sticks.is_empty() {
            return;
        }
        let read = |axis: Option<Axis>| {
            axis.map_or(0.0, |axis| {
                self.axes.get(&axis.code).copied().unwrap_or(0.0) * axis.scale
            })
        };
        for stick in &self.sticks {
            if !stick.condition.is_subset(&self.pressed) {
                continue;
            }
            let carry = &mut self.carry[usize::from(stick.motion == Motion::Wheel)];
            for (index, value) in [read(stick.x), read(Some(stick.y))].into_iter().enumerate() {
                carry[index] +=
                    curve(value, stick.deadzone, stick.acceleration) * stick.speed * seconds;
            }
        }
        for (motion, axes) in [
            (
                Motion::Pointer,
                [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y],
            ),
            (
                Motion::Wheel,
                [RelativeAxisCode::REL_HWHEEL, RelativeAxisCode::REL_WHEEL],
            ),
        ] {
            let carry = &mut self.carry[usize::from(motion == Motion::Wheel)];
            let mut moved = Vec::new();
            for (index, axis) in axes.into_iter().enumerate() {
                let whole = carry[index].trunc();
                if whole != 0.0 {
                    carry[index] -= whole;
                    moved.push((axis, whole as i32));
                }
            }
            if !moved.is_empty() {
                sink.moved(&moved);
            }
        }
    }

    /// Whether a mapping may fire now (and remember that it does).
    fn cooled_down(
        last_fired: &mut HashMap<Fired, Instant>,
        key: Fired,
        cooldown: Duration,
        now: Instant,
    ) -> bool {
        if last_fired
            .get(&key)
            .is_some_and(|last| now.saturating_duration_since(*last) < cooldown)
        {
            return false;
        }
        last_fired.insert(key, now);
        true
    }

    fn fire(&mut self, combo: Combo, now: Instant, sink: &mut dyn Sink) {
        if !Self::cooled_down(
            &mut self.last_fired,
            Fired::Combo(combo.codes.clone()),
            combo.cooldown,
            now,
        ) {
            return;
        }
        // its inputs are not taps any more
        self.armed.retain(|code, _| !combo.codes.contains(code));
        // the shorter combinations waiting inside this one are taken by it
        self.pending.retain(|(pending, _)| {
            !(pending.codes.is_subset(&combo.codes) && pending.codes != combo.codes)
        });
        log::debug!("combination {:x?}", combo.codes);
        sink.down(&combo.action);
        self.active.push(combo);
    }

    fn advance_sequences(&mut self, code: Code, now: Instant, sink: &mut dyn Sink) {
        for (index, sequence) in self.sequences.iter().enumerate() {
            let progress = &mut self.progress[index];
            if sequence.codes.get(*progress) == Some(&code) {
                *progress += 1;
            } else {
                *progress = usize::from(sequence.codes.first() == Some(&code));
            }
            if *progress == sequence.codes.len() {
                *progress = 0;
                if Self::cooled_down(
                    &mut self.last_fired,
                    Fired::Sequence(index),
                    sequence.cooldown,
                    now,
                ) {
                    log::debug!("sequence {index}");
                    sink.down(&sequence.action);
                    sink.up(&sequence.action);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::KeyCode;

    /// What the engine did, in order.
    #[derive(Default)]
    struct Record(Vec<String>);

    impl Sink for Record {
        fn down(&mut self, action: &Action) {
            self.0.push(format!("down {}", name(action)));
        }
        fn up(&mut self, action: &Action) {
            self.0.push(format!("up {}", name(action)));
        }
        fn moved(&mut self, motion: &[(RelativeAxisCode, i32)]) {
            for (axis, amount) in motion {
                self.0.push(format!("{axis:?} {amount}"));
            }
        }
    }

    fn name(action: &Action) -> String {
        match action {
            Action::Keys(keys) => format!("{keys:?}"),
            Action::Click(button) => format!("{button:?}"),
            Action::Exec(commands) => commands.join(";"),
        }
    }

    fn combo(codes: &[Code], key: KeyCode) -> Combo {
        Combo {
            codes: codes.iter().copied().collect(),
            action: Action::Keys(vec![key]),
            cooldown: Duration::ZERO,
            hold: Duration::ZERO,
            tap: false,
        }
    }

    const HOTKEY: Code = 1;
    const START: Code = 2;
    const UP: Code = 3;

    fn engine(combos: Vec<Combo>) -> Engine {
        Engine::new(Mappings {
            combos,
            ..Mappings::default()
        })
    }

    #[test]
    fn the_longest_combination_wins_and_is_held() {
        let mut engine = engine(vec![
            combo(&[START], KeyCode::KEY_ENTER),
            combo(&[HOTKEY, START], KeyCode::KEY_ESC),
        ]);
        let (mut sink, now) = (Record::default(), Instant::now());
        engine.press(HOTKEY, now, &mut sink);
        engine.press(START, now, &mut sink);
        engine.release(START, now, &mut sink);
        engine.press(START, now, &mut sink);
        engine.release(START, now, &mut sink);
        engine.release(HOTKEY, now, &mut sink);
        assert_eq!(
            sink.0,
            [
                "down [KEY_ESC]",
                "up [KEY_ESC]",
                "down [KEY_ESC]",
                "up [KEY_ESC]"
            ]
        );
        engine.press(START, now, &mut sink);
        assert_eq!(sink.0.last().unwrap(), "down [KEY_ENTER]");
    }

    #[test]
    fn cooldowns_keep_combinations_from_firing_too_soon() {
        let mut slow = combo(&[HOTKEY, START], KeyCode::KEY_ESC);
        slow.cooldown = Duration::from_millis(500);
        let mut engine = engine(vec![slow]);
        let (mut sink, now) = (Record::default(), Instant::now());
        for offset in [0, 100, 600] {
            let at = now + Duration::from_millis(offset);
            engine.press(HOTKEY, at, &mut sink);
            engine.press(START, at, &mut sink);
            engine.release(START, at, &mut sink);
            engine.release(HOTKEY, at, &mut sink);
        }
        assert_eq!(
            sink.0
                .iter()
                .filter(|line| line.starts_with("down"))
                .count(),
            2
        );
    }

    #[test]
    fn a_held_combination_waits_and_is_cancelled_by_another_press() {
        let mut held = combo(&[HOTKEY], KeyCode::KEY_F1);
        held.hold = Duration::from_secs(1);
        let mut engine = engine(vec![held, combo(&[HOTKEY, UP], KeyCode::KEY_VOLUMEUP)]);
        let (mut sink, now) = (Record::default(), Instant::now());
        engine.press(HOTKEY, now, &mut sink);
        assert!(engine.needs_tick());
        engine.tick(now + Duration::from_millis(500), Duration::ZERO, &mut sink);
        assert_eq!(sink.0, Vec::<String>::new());
        engine.tick(now + Duration::from_secs(1), Duration::ZERO, &mut sink);
        assert_eq!(sink.0, ["down [KEY_F1]"]);
        engine.release(HOTKEY, now, &mut sink);
        sink.0.clear();
        // Hotkey+Up meanwhile: the hold never fires
        engine.press(HOTKEY, now, &mut sink);
        engine.press(UP, now, &mut sink);
        engine.tick(now + Duration::from_secs(2), Duration::ZERO, &mut sink);
        assert_eq!(sink.0, ["down [KEY_VOLUMEUP]"]);
        assert!(!engine.needs_tick());
    }

    #[test]
    fn a_tap_fires_on_release_only_alone_and_quick() {
        let mut tap = combo(&[HOTKEY], KeyCode::KEY_F2);
        tap.tap = true;
        let mut engine = engine(vec![tap, combo(&[HOTKEY, START], KeyCode::KEY_ESC)]);
        let (mut sink, now) = (Record::default(), Instant::now());
        engine.press(HOTKEY, now, &mut sink);
        engine.release(HOTKEY, now + Duration::from_millis(200), &mut sink);
        assert_eq!(sink.0, ["down [KEY_F2]", "up [KEY_F2]"]);
        sink.0.clear();
        // too long
        engine.press(HOTKEY, now, &mut sink);
        engine.release(HOTKEY, now + Duration::from_secs(1), &mut sink);
        // part of a combination
        engine.press(HOTKEY, now, &mut sink);
        engine.press(START, now, &mut sink);
        engine.release(START, now, &mut sink);
        engine.release(HOTKEY, now, &mut sink);
        assert_eq!(sink.0, ["down [KEY_ESC]", "up [KEY_ESC]"]);
    }

    #[test]
    fn sequences_follow_the_presses() {
        let mut engine = Engine::new(Mappings {
            sequences: vec![Sequence {
                codes: vec![UP, UP, START],
                action: Action::Exec(vec!["konami".into()]),
                cooldown: Duration::ZERO,
            }],
            ..Mappings::default()
        });
        let (mut sink, now) = (Record::default(), Instant::now());
        // evmapy's (and gamepadly's) way: a wrong input starts over, from the
        // first one if it is that one
        for code in [UP, START, UP, UP, START, UP, UP, UP, START] {
            engine.press(code, now, &mut sink);
            engine.release(code, now, &mut sink);
        }
        assert_eq!(sink.0, ["down konami", "up konami"]);
    }

    #[test]
    fn a_stick_moves_the_pointer_while_its_condition_is_held() {
        let mut engine = Engine::new(Mappings {
            sticks: vec![Stick {
                motion: Motion::Pointer,
                x: Some(Axis {
                    code: 3,
                    scale: 1.0,
                }),
                y: Axis {
                    code: 4,
                    scale: 1.0,
                },
                speed: 800.0,
                deadzone: 0.15,
                acceleration: 1.0,
                condition: [HOTKEY].into_iter().collect(),
            }],
            ..Mappings::default()
        });
        let (mut sink, now) = (Record::default(), Instant::now());
        engine.set_axis(3, 1.0);
        assert!(!engine.needs_tick(), "the condition is not held");
        engine.press(HOTKEY, now, &mut sink);
        assert!(engine.needs_tick());
        engine.tick(now, Duration::from_millis(10), &mut sink);
        assert_eq!(sink.0, ["REL_X 8"]);
        engine.release_all(now, &mut sink);
        assert!(!engine.needs_tick());
    }

    #[test]
    fn the_curve_starts_past_the_dead_zone() {
        assert!(curve(0.1, 0.15, 1.5).abs() < f32::EPSILON);
        assert!((curve(-1.0, 0.15, 1.5) + 1.0).abs() < 1e-6);
        assert!(curve(0.5, 0.15, 2.0) < 0.5);
    }
}
