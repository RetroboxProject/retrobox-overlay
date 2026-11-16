//! Where the mappings' actions go: a virtual keyboard and a virtual mouse
//! (uinput), and the commands of `exec` mappings.
//!
//! The keyboard and the mouse are two devices: a mouse button on a keyboard
//! (or a key on a mouse) is not always taken for what it is. Each declares
//! only what the profile uses; their names have "retrobox" in them, so the
//! daemon never reads them (`probe`).

use std::collections::BTreeSet;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode};

use super::engine::Sink;
use super::profile::{Action, Motion, Profile};

/// The mouse buttons, among the key codes (the rest go to the keyboard).
const BUTTONS: std::ops::RangeInclusive<u16> = 0x100..=0x2e7;

fn is_button(key: KeyCode) -> bool {
    BUTTONS.contains(&key.code())
}

/// The virtual devices, and the commands running.
#[derive(Default)]
pub struct Output {
    keyboard: Option<VirtualDevice>,
    mouse: Option<VirtualDevice>,
    commands: Vec<Child>,
}

impl Output {
    /// The devices for what these profiles use.
    pub fn for_profiles<'a>(profiles: impl IntoIterator<Item = &'a Profile>) -> Self {
        let mut keys = BTreeSet::new();
        let (mut pointer, mut wheel) = (false, false);
        for profile in profiles {
            let actions = profile
                .hotkeys
                .iter()
                .map(|hotkey| &hotkey.action)
                .chain(profile.sequences.iter().map(|sequence| &sequence.action));
            for action in actions {
                match action {
                    Action::Keys(codes) => keys.extend(codes.iter().copied()),
                    Action::Click(button) => {
                        keys.insert(*button);
                    }
                    Action::Exec(_) => {}
                }
            }
            for stick in &profile.sticks {
                match stick.motion {
                    Motion::Pointer => pointer = true,
                    Motion::Wheel => wheel = true,
                }
            }
        }
        let (mut buttons, keys): (BTreeSet<KeyCode>, BTreeSet<KeyCode>) =
            keys.into_iter().partition(|key| is_button(*key));
        if pointer {
            // a pointer without a button is not taken for a mouse
            buttons.insert(KeyCode::BTN_LEFT);
        }
        let keyboard = (!keys.is_empty())
            .then(|| build("retrobox-inputd keyboard", &keys, &[]))
            .flatten();
        let mut motion = Vec::new();
        if pointer || !buttons.is_empty() {
            motion.extend([RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]);
        }
        if wheel {
            motion.extend([RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_HWHEEL]);
        }
        let mouse = (!motion.is_empty())
            .then(|| build("retrobox-inputd mouse", &buttons, &motion))
            .flatten();
        Self {
            keyboard,
            mouse,
            commands: Vec::new(),
        }
    }

    /// Forget the commands that have ended (none is left a zombie).
    pub fn reap(&mut self) {
        self.commands
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
    }

    fn keys(&mut self, keys: &[KeyCode], value: i32) {
        let (buttons, keys): (Vec<KeyCode>, Vec<KeyCode>) =
            keys.iter().partition(|key| is_button(**key));
        for (device, keys) in [(&mut self.keyboard, keys), (&mut self.mouse, buttons)] {
            if keys.is_empty() {
                continue;
            }
            let events: Vec<InputEvent> = keys
                .iter()
                .map(|key| InputEvent::new(EventType::KEY.0, key.code(), value))
                .collect();
            emit(device.as_mut(), &events);
        }
    }

    fn run(&mut self, command: &str) {
        log::debug!("exec: {command}");
        // in a process group of its own, in the background: it never stalls
        // the daemon, and a command that ends the game is not ended with it
        match Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
        {
            Ok(child) => self.commands.push(child),
            Err(err) => log::error!("cannot run {command:?}: {err}"),
        }
    }
}

impl Sink for Output {
    fn down(&mut self, action: &Action) {
        match action {
            Action::Keys(keys) => self.keys(keys, 1),
            Action::Click(button) => self.keys(&[*button], 1),
            Action::Exec(commands) => {
                for command in commands {
                    self.run(command);
                }
            }
        }
    }

    fn up(&mut self, action: &Action) {
        match action {
            Action::Keys(keys) => {
                let reversed: Vec<KeyCode> = keys.iter().rev().copied().collect();
                self.keys(&reversed, 0);
            }
            Action::Click(button) => self.keys(&[*button], 0),
            Action::Exec(_) => {}
        }
    }

    fn moved(&mut self, motion: &[(RelativeAxisCode, i32)]) {
        let events: Vec<InputEvent> = motion
            .iter()
            .map(|(axis, amount)| InputEvent::new(EventType::RELATIVE.0, axis.0, *amount))
            .collect();
        emit(self.mouse.as_mut(), &events);
    }
}

/// Write events, and the report that ends them.
fn emit(device: Option<&mut VirtualDevice>, events: &[InputEvent]) {
    if let Some(device) = device
        && let Err(err) = device.emit(events)
    {
        log::warn!("cannot write to the virtual device: {err}");
    }
}

/// A virtual device with these keys and relative axes.
fn build(
    name: &str,
    keys: &BTreeSet<KeyCode>,
    motion: &[RelativeAxisCode],
) -> Option<VirtualDevice> {
    let built = (|| {
        let mut builder = VirtualDevice::builder()?.name(name);
        if !keys.is_empty() {
            let set: AttributeSet<KeyCode> = keys.iter().copied().collect();
            builder = builder.with_keys(&set)?;
        }
        if !motion.is_empty() {
            let set: AttributeSet<RelativeAxisCode> = motion.iter().copied().collect();
            builder = builder.with_relative_axes(&set)?;
        }
        builder.build()
    })();
    match built {
        Ok(device) => {
            log::info!("{name}: {} key(s), {} axis", keys.len(), motion.len());
            Some(device)
        }
        Err(err) => {
            log::error!("cannot create {name} (/dev/uinput: the input group?): {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_go_to_the_mouse() {
        assert!(is_button(KeyCode::BTN_LEFT));
        assert!(!is_button(KeyCode::KEY_F4));
        assert!(!is_button(KeyCode::KEY_VOLUMEUP));
    }
}
