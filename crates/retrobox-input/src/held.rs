//! What the user holds while a game loads: how the launcher is asked to
//! install the game's emulator again (`retrobox-overlay --held`, L1+R1 or
//! Ctrl+Shift).

use evdev::{Device, KeyCode};

use crate::es_input;
use crate::probe::{self, Kind};

/// Whether the user holds what asks for an emulator to be installed again
/// while a game loads: L1 and R1 on a controller, or Ctrl and Shift on a
/// keyboard.
pub fn reinstall_held() -> bool {
    shoulders_held() || modifiers_held()
}

/// Whether Ctrl and Shift (either of each) are down.
fn ctrl_and_shift(keys: &evdev::AttributeSet<KeyCode>) -> bool {
    (keys.contains(KeyCode::KEY_LEFTCTRL) || keys.contains(KeyCode::KEY_RIGHTCTRL))
        && (keys.contains(KeyCode::KEY_LEFTSHIFT) || keys.contains(KeyCode::KEY_RIGHTSHIFT))
}

/// Whether some keyboard has Ctrl and Shift down right now (the state comes
/// from the kernel, nothing is grabbed).
fn modifiers_held() -> bool {
    probe::paths(Kind::Keyboard).iter().any(|path| {
        Device::open(path)
            .ok()
            .and_then(|device| device.get_key_state().ok())
            .is_some_and(|keys| ctrl_and_shift(&keys))
    })
}

/// Whether some controller has both shoulder buttons (L1 and R1) down right
/// now, as EmulationStation numbers them. Reads the state from the kernel,
/// without grabbing anything.
fn shoulders_held() -> bool {
    probe::paths(Kind::Pad).iter().any(|path| {
        let Ok(device) = Device::open(path) else {
            return false;
        };
        let Ok(keys) = device.get_key_state() else {
            return false;
        };
        let translation = es_input::translation(&device);
        let standard: Vec<KeyCode> = keys
            .iter()
            .map(|key| {
                translation
                    .as_ref()
                    .and_then(|table| table.get(&key).copied())
                    .unwrap_or(key)
            })
            .collect();
        standard.contains(&KeyCode::BTN_TL) && standard.contains(&KeyCode::BTN_TR)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_and_shift_are_both_needed() {
        let keys = |codes: &[KeyCode]| codes.iter().copied().collect::<evdev::AttributeSet<_>>();
        assert!(ctrl_and_shift(&keys(&[
            KeyCode::KEY_LEFTCTRL,
            KeyCode::KEY_RIGHTSHIFT
        ])));
        assert!(!ctrl_and_shift(&keys(&[KeyCode::KEY_LEFTSHIFT])));
        assert!(!ctrl_and_shift(&keys(&[KeyCode::KEY_RIGHTCTRL])));
        assert!(!ctrl_and_shift(&keys(&[])));
    }
}
