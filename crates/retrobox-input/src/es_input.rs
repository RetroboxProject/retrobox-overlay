//! What EmulationStation says each controller's buttons are.
//!
//! `es_input.cfg` is the source of truth for the pads, as it is for the
//! emulators: it names each button the way EmulationStation does (`b` is the
//! south one, `a` the east one, `x` the top one, `y` the left one, in the SNES
//! layout) and numbers it as SDL does. EmulationStation is started with SDL's
//! evdev joystick driver, whose numbering is the order of the device's button
//! codes: from `BTN_JOYSTICK` up, then the ones below it.
//!
//! A controller's `Layout` is read once, when the device is opened, and
//! everything else asks it: the input daemon's triggers (the hotkey, B) and
//! its mapper (the profiles' names), and the overlay, which thinks in the
//! standard evdev layout (`BTN_SOUTH` is `b`...) and only needs the codes a
//! pad really emits translated into it.

use std::collections::HashMap;
use std::path::PathBuf;

use evdev::{Device, KeyCode};

use crate::paths;

/// First code of SDL's button numbering (`BTN_JOYSTICK`); the codes below
/// it come after all the others.
const FIRST_CODE: u16 = 0x120;
/// Past the last key code.
const LAST_CODE: u16 = 0x300;

/// The standard evdev code of an EmulationStation input name.
fn standard_code(name: &str) -> Option<KeyCode> {
    Some(match name {
        "b" => KeyCode::BTN_SOUTH,
        "a" => KeyCode::BTN_EAST,
        "x" => KeyCode::BTN_NORTH,
        "y" => KeyCode::BTN_WEST,
        "start" => KeyCode::BTN_START,
        "select" => KeyCode::BTN_SELECT,
        "pageup" => KeyCode::BTN_TL,
        "pagedown" => KeyCode::BTN_TR,
        "l2" => KeyCode::BTN_TL2,
        "r2" => KeyCode::BTN_TR2,
        "l3" => KeyCode::BTN_THUMBL,
        "r3" => KeyCode::BTN_THUMBR,
        "hotkey" => KeyCode::BTN_MODE,
        "up" => KeyCode::BTN_DPAD_UP,
        "down" => KeyCode::BTN_DPAD_DOWN,
        "left" => KeyCode::BTN_DPAD_LEFT,
        "right" => KeyCode::BTN_DPAD_RIGHT,
        _ => return None,
    })
}

/// How a pad's own button codes translate to the standard ones.
pub type Translation = HashMap<KeyCode, KeyCode>;

/// SDL's number of each button code of `codes`.
fn sdl_order(codes: &[u16]) -> Vec<u16> {
    let mut sorted = codes.to_vec();
    sorted.sort_unstable();
    let (low, high): (Vec<u16>, Vec<u16>) = sorted.into_iter().partition(|&code| code < FIRST_CODE);
    high.into_iter().chain(low).collect()
}

/// The user's `es_input.cfg`, the one EmulationStation writes when a
/// controller is configured.
pub fn user_config_file() -> Option<PathBuf> {
    paths::config_dirs()
        .pop()
        .map(|dir| dir.join("frontend/.emulationstation/es_input.cfg"))
}

/// The `es_input.cfg` files, the user's first.
fn config_files() -> Vec<PathBuf> {
    let packaged = paths::root_dir().map(|dir| dir.join("frontend/es_input.cfg"));
    user_config_file().into_iter().chain(packaged).collect()
}

/// The part of SDL's GUID that tells the models apart: vendor, product and
/// version (not the bus and the name's checksum, which vary).
fn guid_model(vendor: u16, product: u16, version: u16) -> String {
    let le = |value: u16| format!("{:02x}{:02x}0000", value & 0xff, value >> 8);
    format!("{}{}{}", le(vendor), le(product), le(version))
}

/// What one input of an `inputConfig` is, as SDL numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsKind {
    Button,
    /// `value` is the direction: 1 up, 2 right, 4 down, 8 left.
    Hat,
    /// `value` is the direction the input is at: -1 or 1.
    Axis,
}

/// One `<input>` of a controller's `inputConfig`: `name` (`a`, `hotkey`,
/// `joystick1up`...), its kind, SDL's number and the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EsInput {
    pub name: String,
    pub kind: EsKind,
    pub id: u16,
    pub value: i32,
}

/// The inputs of the `inputConfig` of `text` that is `device`'s own: by
/// GUID, else by name.
fn parse_inputs(text: &str, device: &Device) -> Option<Vec<EsInput>> {
    let document = roxmltree::Document::parse(text).ok()?;
    let id = device.input_id();
    let model = guid_model(id.vendor(), id.product(), id.version());
    let name = device.name().unwrap_or_default().trim().to_lowercase();
    let configs: Vec<_> = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("inputConfig") && node.attribute("type") == Some("joystick")
        })
        .collect();
    let config = configs
        .iter()
        .find(|node| {
            node.attribute("deviceGUID")
                .is_some_and(|guid| guid.len() == 32 && guid[8..].eq_ignore_ascii_case(&model))
        })
        .or_else(|| {
            configs.iter().find(|node| {
                node.attribute("deviceName")
                    .is_some_and(|device_name| device_name.trim().to_lowercase() == name)
            })
        })?;
    Some(
        config
            .children()
            .filter(|node| node.has_tag_name("input"))
            .filter_map(|node| {
                let kind = match node.attribute("type")? {
                    "button" => EsKind::Button,
                    "hat" => EsKind::Hat,
                    "axis" => EsKind::Axis,
                    _ => return None,
                };
                Some(EsInput {
                    name: node.attribute("name")?.to_owned(),
                    kind,
                    id: node.attribute("id")?.trim().parse().ok()?,
                    value: node
                        .attribute("value")
                        .and_then(|value| value.trim().parse().ok())
                        .unwrap_or(1),
                })
            })
            .collect(),
    )
}

/// The button codes of `device` in SDL's numbering (what `es_input.cfg`'s
/// button ids count).
fn sdl_buttons(device: &Device) -> Vec<KeyCode> {
    let codes: Vec<u16> = device
        .supported_keys()
        .map(|keys| {
            keys.iter()
                .map(KeyCode::code)
                .filter(|code| (0..LAST_CODE).contains(code))
                .collect()
        })
        .unwrap_or_default();
    sdl_order(&codes).into_iter().map(KeyCode::new).collect()
}

/// What EmulationStation says one controller's inputs are: its
/// `inputConfig`, with the device's buttons in SDL's numbering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    inputs: Vec<EsInput>,
    buttons: Vec<KeyCode>,
}

impl Layout {
    /// The layout of `device`, if EmulationStation knows it.
    pub fn of(device: &Device) -> Option<Self> {
        config_files().iter().find_map(|file| {
            let text = std::fs::read_to_string(file).ok()?;
            let inputs = parse_inputs(&text, device)?;
            log::info!(
                "inputs of {} from {}",
                device.name().unwrap_or("?"),
                file.display()
            );
            Some(Self {
                inputs,
                buttons: sdl_buttons(device),
            })
        })
    }

    pub fn inputs(&self) -> &[EsInput] {
        &self.inputs
    }

    /// The button codes, in SDL's numbering.
    pub fn buttons(&self) -> &[KeyCode] {
        &self.buttons
    }

    /// The code of a button input (`b`, `hotkey`...), as the controller
    /// sends it.
    pub fn button(&self, name: &str) -> Option<KeyCode> {
        let input = self.input(name)?;
        (input.kind == EsKind::Button)
            .then(|| self.buttons.get(usize::from(input.id)).copied())
            .flatten()
    }

    fn input(&self, name: &str) -> Option<&EsInput> {
        self.inputs.iter().find(|input| input.name == name)
    }

    /// Whether the input of `name` has another name as well: the hotkey on
    /// Select, on a controller without a button of its own for it.
    pub fn has_two_names(&self, name: &str) -> bool {
        self.input(name).is_some_and(|input| {
            self.inputs.iter().any(|other| {
                other.name != input.name
                    && other.kind == input.kind
                    && other.id == input.id
                    && (other.kind == EsKind::Button || other.value == input.value)
            })
        })
    }

    /// The hotkey's code, and whether it is a button of its own
    /// (Home/Guide) and not one the game uses too. None when it is not a
    /// button.
    pub fn hotkey(&self) -> Option<(KeyCode, bool)> {
        Some((self.button("hotkey")?, !self.has_two_names("hotkey")))
    }

    /// How the controller's buttons translate to the standard codes (None
    /// when none does). A hotkey on a game's button stays that button.
    pub fn translation(&self) -> Option<Translation> {
        let shared_hotkey = self.has_two_names("hotkey");
        let translation: Translation = self
            .inputs
            .iter()
            .filter(|input| input.kind == EsKind::Button)
            .filter(|input| !(shared_hotkey && input.name == "hotkey"))
            .filter_map(|input| {
                let standard = standard_code(&input.name)?;
                Some((*self.buttons.get(usize::from(input.id))?, standard))
            })
            .collect();
        (!translation.is_empty()).then_some(translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdl_numbers_the_joystick_codes_first() {
        // xpad: A B X Y TL TR SELECT START MODE THUMBL THUMBR
        let codes = [
            0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e,
        ];
        let order = sdl_order(&codes);
        assert_eq!(order[0], 0x130);
        assert_eq!(order[2], 0x133);
        assert_eq!(order[8], 0x13c);
        // a code below BTN_JOYSTICK goes last
        assert_eq!(*sdl_order(&[0x110, 0x130]).last().unwrap(), 0x110);
    }

    #[test]
    fn the_model_of_a_guid_is_vendor_product_version() {
        assert_eq!(
            guid_model(0x045e, 0x02ea, 0x0301),
            "5e040000ea020000".to_owned() + "01030000"
        );
    }

    fn button(name: &str, id: u16) -> EsInput {
        EsInput {
            name: name.to_owned(),
            kind: EsKind::Button,
            id,
            value: 1,
        }
    }

    fn layout(inputs: &[EsInput]) -> Layout {
        Layout {
            inputs: inputs.to_vec(),
            buttons: vec![KeyCode::BTN_SOUTH, KeyCode::BTN_SELECT, KeyCode::BTN_MODE],
        }
    }

    #[test]
    fn a_hotkey_of_its_own_is_dedicated() {
        let guide = layout(&[button("b", 0), button("select", 1), button("hotkey", 2)]);
        assert_eq!(guide.hotkey(), Some((KeyCode::BTN_MODE, true)));
        assert_eq!(guide.button("b"), Some(KeyCode::BTN_SOUTH));
        let translation = guide.translation().unwrap();
        assert_eq!(translation[&KeyCode::BTN_MODE], KeyCode::BTN_MODE);
        assert_eq!(translation[&KeyCode::BTN_SELECT], KeyCode::BTN_SELECT);
    }

    #[test]
    fn a_hotkey_on_select_is_shared_and_select_stays_select() {
        // the hotkey last: it must not take Select's place
        let select = layout(&[button("b", 0), button("select", 1), button("hotkey", 1)]);
        assert_eq!(select.hotkey(), Some((KeyCode::BTN_SELECT, false)));
        assert!(select.has_two_names("select"));
        assert!(!select.has_two_names("b"));
        let translation = select.translation().unwrap();
        assert_eq!(translation[&KeyCode::BTN_SELECT], KeyCode::BTN_SELECT);
    }

    #[test]
    fn a_hotkey_that_is_no_button_is_none() {
        assert_eq!(layout(&[button("b", 0)]).hotkey(), None);
        let on_a_hat = EsInput {
            kind: EsKind::Hat,
            ..button("hotkey", 0)
        };
        assert_eq!(layout(&[on_a_hat]).hotkey(), None);
    }

    #[test]
    fn names_have_a_standard_code() {
        assert_eq!(standard_code("b"), Some(KeyCode::BTN_SOUTH));
        assert_eq!(standard_code("joystick1left"), None);
    }
}
