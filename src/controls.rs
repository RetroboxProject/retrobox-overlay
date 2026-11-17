//! What the game's pad-to-keyboard profile does, for the controls page:
//! player 1's mappings (what `retrobox-inputd` maps), each as the inputs
//! that trigger it and what it presses, in the profile's order.

use std::path::Path;

use retrobox_input::pad_profile::{self, Action, Motion, Section};

use crate::i18n::Translator;

/// One mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    /// EmulationStation's input names (`a`, `l2`, `joystick1up`...),
    /// pressed together or, for a `sequence`, one after the other.
    pub inputs: Vec<String>,
    pub sequence: bool,
    /// What it does: the profile's description, else what it presses.
    pub label: String,
    /// What it presses, when the label is the description.
    pub detail: String,
    /// The keyboard's keys and the mouse's buttons it presses (`KEY_*`,
    /// `BTN_*`), lit on the keyboard's picture.
    pub keys: Vec<String>,
}

/// The mappings of the profile at `path` (none when it cannot be read).
pub fn read(path: &Path, translator: &Translator) -> Vec<Mapping> {
    let Some(data) = pad_profile::load(path) else {
        return Vec::new();
    };
    let profile = pad_profile::read(&data, Section::Player(1));
    let hotkeys = profile.hotkeys.iter().map(|hotkey| {
        let mut mapping = mapping(
            &hotkey.trigger,
            false,
            &hotkey.action,
            &hotkey.description,
            translator,
        );
        if !hotkey.hold.is_zero() {
            let held = translator.text("controls-hold");
            let shown = if mapping.detail.is_empty() {
                &mut mapping.label
            } else {
                &mut mapping.detail
            };
            *shown = format!("{shown} ({held})");
        }
        mapping
    });
    let sequences = profile.sequences.iter().map(|sequence| {
        mapping(
            &sequence.trigger,
            true,
            &sequence.action,
            &sequence.description,
            translator,
        )
    });
    let sticks = profile.sticks.iter().map(|stick| {
        // the stick itself (`joystick1up` is one of joystick1's axes)
        let whole = stick
            .axis_y
            .get(.."joystickN".len())
            .unwrap_or(&stick.axis_y);
        let mut inputs = stick.condition.clone();
        inputs.push(whole.to_owned());
        let what = translator.text(match stick.motion {
            Motion::Pointer => "controls-pointer",
            Motion::Wheel => "controls-wheel",
        });
        described(inputs, false, &stick.description, what, Vec::new())
    });
    hotkeys.chain(sequences).chain(sticks).collect()
}

fn mapping(
    inputs: &[String],
    sequence: bool,
    action: &Action,
    description: &str,
    translator: &Translator,
) -> Mapping {
    let (what, keys) = match action {
        Action::Keys(codes) => {
            let keys: Vec<String> = codes.iter().map(|code| format!("{code:?}")).collect();
            let names: Vec<String> = keys.iter().map(|key| key_name(key)).collect();
            (names.join(" + "), keys)
        }
        Action::Click(button) => {
            let key = format!("{button:?}");
            (translator.text(click_name(&key)), vec![key])
        }
        Action::Exec(commands) => (commands.join("; "), Vec::new()),
    };
    // a command without a description says little: "Command"
    let what = if matches!(action, Action::Exec(_)) && description.is_empty() {
        translator.text("controls-command")
    } else {
        what
    };
    described(inputs.to_vec(), sequence, description, what, keys)
}

fn described(
    inputs: Vec<String>,
    sequence: bool,
    description: &str,
    what: String,
    keys: Vec<String>,
) -> Mapping {
    let (label, detail) = if description.is_empty() {
        (what, String::new())
    } else {
        (description.to_owned(), what)
    };
    Mapping {
        inputs,
        sequence,
        label,
        detail,
        keys,
    }
}

/// The translation key of a mouse button's name.
fn click_name(button: &str) -> &'static str {
    match button {
        "BTN_RIGHT" => "controls-click-right",
        "BTN_MIDDLE" => "controls-click-middle",
        _ => "controls-click-left",
    }
}

/// A key as its keycap says (`KEY_LEFTALT`: Alt).
fn key_name(key: &str) -> String {
    let name = key.strip_prefix("KEY_").unwrap_or(key);
    let known = match name {
        "LEFTCTRL" | "RIGHTCTRL" => "Ctrl",
        "LEFTSHIFT" | "RIGHTSHIFT" => "Shift",
        "LEFTALT" => "Alt",
        "RIGHTALT" => "AltGr",
        "LEFTMETA" | "RIGHTMETA" => "Super",
        "ESC" => "Esc",
        "ENTER" => "Enter",
        "SPACE" => "Space",
        "BACKSPACE" => "Backspace",
        "TAB" => "Tab",
        "CAPSLOCK" => "Caps Lock",
        "PAGEUP" => "Page Up",
        "PAGEDOWN" => "Page Down",
        "DELETE" => "Del",
        "INSERT" => "Ins",
        "HOME" => "Home",
        "END" => "End",
        "UP" => "Up",
        "DOWN" => "Down",
        "LEFT" => "Left",
        "RIGHT" => "Right",
        "GRAVE" => "`",
        "MINUS" => "-",
        "EQUAL" => "=",
        "LEFTBRACE" => "[",
        "RIGHTBRACE" => "]",
        "SEMICOLON" => ";",
        "APOSTROPHE" => "'",
        "BACKSLASH" => "\\",
        "COMMA" => ",",
        "DOT" => ".",
        "SLASH" => "/",
        _ => {
            return name.strip_prefix("KP").map_or_else(
                || name.to_owned(),
                |keypad| format!("Num {}", key_name(&format!("KEY_{keypad}"))),
            );
        }
    };
    known.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_as_their_keycaps() {
        assert_eq!(key_name("KEY_LEFTALT"), "Alt");
        assert_eq!(key_name("KEY_F4"), "F4");
        assert_eq!(key_name("KEY_KP5"), "Num 5");
        assert_eq!(key_name("KEY_KPENTER"), "Num Enter");
    }

    #[test]
    fn reads_player_ones_mappings() {
        let dir = std::env::temp_dir().join(format!("controls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.keys");
        std::fs::write(
            &path,
            r#"{"actions_player1": [
                {"trigger": ["hotkey", "start"], "type": "key", "target": ["KEY_LEFTALT", "KEY_F4"]},
                {"trigger": "a", "type": "key", "target": "KEY_SPACE", "description": "Jump", "hold": 1},
                {"trigger": "r2", "type": "key", "target": "BTN_LEFT"},
                {"trigger": "joystick2", "type": "mouse"}
            ]}"#,
        )
        .unwrap();
        let translator = Translator::new("en_US");
        let controls = read(&path, &translator);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(controls.len(), 4);
        assert_eq!(controls[0].label, "Alt + F4");
        assert_eq!(controls[0].keys, ["KEY_LEFTALT", "KEY_F4"]);
        assert_eq!(controls[1].label, "Jump");
        assert_eq!(controls[1].detail, "Space (hold)");
        assert_eq!(controls[2].keys, ["BTN_LEFT"]);
        assert_eq!(controls[3].inputs, ["joystick2"]);
    }
}
