//! What EmulationStation says each controller's buttons are.
//!
//! `es_input.cfg` is the source of truth for the pads, as it is for the
//! emulators: it names each button the way EmulationStation does (`b` is the
//! south one, `a` the east one, `x` the top one, `y` the left one, in the SNES
//! layout) and numbers it as SDL does. EmulationStation is started with SDL's
//! evdev joystick driver, whose numbering is the order of the device's button
//! codes: from `BTN_JOYSTICK` up, then the ones below it.
//!
//! The rest of the overlay thinks in the standard evdev layout (`BTN_SOUTH`
//! is `b`...), so this module only translates the codes a pad really emits
//! into it.

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

/// The `es_input.cfg` files, the user's first.
fn config_files() -> Vec<PathBuf> {
    let user = paths::config_dirs()
        .pop()
        .map(|dir| dir.join("frontend/.emulationstation/es_input.cfg"));
    let packaged = paths::root_dir().map(|dir| dir.join("frontend/es_input.cfg"));
    user.into_iter().chain(packaged).collect()
}

/// The part of SDL's GUID that tells the models apart: vendor, product and
/// version (not the bus and the name's checksum, which vary).
fn guid_model(vendor: u16, product: u16, version: u16) -> String {
    let le = |value: u16| format!("{:02x}{:02x}0000", value & 0xff, value >> 8);
    format!("{}{}{}", le(vendor), le(product), le(version))
}

/// The translation for `device` from the `inputConfig` of `text` that is
/// its own: by GUID, else by name. None when there is none, or it maps no
/// button.
fn parse(text: &str, device: &Device) -> Option<Translation> {
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
    let codes: Vec<u16> = device
        .supported_keys()?
        .iter()
        .map(KeyCode::code)
        .filter(|code| (0..LAST_CODE).contains(code))
        .collect();
    let order = sdl_order(&codes);
    let translation: Translation = config
        .children()
        .filter(|node| node.has_tag_name("input") && node.attribute("type") == Some("button"))
        .filter_map(|node| {
            let standard = standard_code(node.attribute("name")?)?;
            let number: usize = node.attribute("id")?.parse().ok()?;
            Some((KeyCode::new(*order.get(number)?), standard))
        })
        .collect();
    (!translation.is_empty()).then_some(translation)
}

/// What EmulationStation says the buttons of `device` are, if it knows it.
pub fn translation(device: &Device) -> Option<Translation> {
    config_files().iter().find_map(|file| {
        let text = std::fs::read_to_string(file).ok()?;
        let found = parse(&text, device);
        if found.is_some() {
            log::info!(
                "buttons of {} from {}",
                device.name().unwrap_or("?"),
                file.display()
            );
        }
        found
    })
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

    #[test]
    fn names_have_a_standard_code() {
        assert_eq!(standard_code("b"), Some(KeyCode::BTN_SOUTH));
        assert_eq!(standard_code("joystick1left"), None);
    }
}
