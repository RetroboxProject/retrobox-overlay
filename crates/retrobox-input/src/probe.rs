//! Which input devices are controllers and which are keyboards.
//!
//! The one place that looks for them, for every mode of the overlay (the
//! daemon, the menu, the on-screen keyboard, the task screen, `--held`). It
//! reads sysfs, never the devices: opening a device just to look at it is
//! slow (closing it waits for the kernel's RCU grace period, tens of
//! milliseconds each, for every device in the machine), and only what is
//! going to be used is opened afterwards.

use std::path::{Path, PathBuf};

use evdev::KeyCode;

const SYSFS: &str = "/sys/class/input";
const DEV: &str = "/dev/input";
/// `BUS_VIRTUAL`
const BUS_VIRTUAL: u64 = 0x06;

/// What a device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// It has the south button.
    Pad,
    /// It has letters, Enter and the arrows (and no south button).
    Keyboard,
}

/// A controller or keyboard of the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub kind: Kind,
    /// Not a virtual device (those are made by software, like the on-screen
    /// keyboard's own or a remote desktop's).
    pub physical: bool,
}

/// Every controller and keyboard, by path. Retrobox's own virtual devices
/// are left out: reading them would be listening to ourselves.
pub fn scan() -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(SYSFS) else {
        return Vec::new();
    };
    let mut found: Vec<Found> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("event"))
        .filter_map(|entry| {
            let device = entry.path().join("device");
            let read = |file: &str| std::fs::read_to_string(device.join(file)).unwrap_or_default();
            let bus = u64::from_str_radix(read("id/bustype").trim(), 16).unwrap_or(BUS_VIRTUAL);
            let (kind, physical) = classify(
                &key_bits(&read("capabilities/key")),
                bus,
                &read("name").to_lowercase(),
            )?;
            Some(Found {
                path: Path::new(DEV).join(entry.file_name()),
                kind,
                physical,
            })
        })
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

/// The paths of the devices of one kind (the physical ones only, for
/// keyboards: a virtual one is not to be grabbed).
pub fn paths(kind: Kind) -> Vec<PathBuf> {
    scan()
        .into_iter()
        .filter(|found| found.kind == kind && (kind == Kind::Pad || found.physical))
        .map(|found| found.path)
        .collect()
}

/// What a device with these keys (see `key_bits`), on this bus and with this
/// lower-case name is: its kind, and whether it is physical.
fn classify(keys: &[u64], bus: u64, name: &str) -> Option<(Kind, bool)> {
    if name.contains("retrobox") {
        return None;
    }
    let has = |code: KeyCode| bit(keys, code.code());
    let kind = if has(KeyCode::BTN_SOUTH) {
        Kind::Pad
    } else if has(KeyCode::KEY_A) && has(KeyCode::KEY_ENTER) && has(KeyCode::KEY_LEFT) {
        Kind::Keyboard
    } else {
        return None;
    };
    Some((kind, bus != BUS_VIRTUAL))
}

/// A sysfs capability bitmap (`capabilities/key`: hexadecimal words, the
/// highest first) as words, the lowest first.
fn key_bits(text: &str) -> Vec<u64> {
    text.split_whitespace()
        .rev()
        .map(|word| u64::from_str_radix(word, 16).unwrap_or(0))
        .collect()
}

/// Whether bit `index` of the words is set.
fn bit(words: &[u64], index: u16) -> bool {
    words
        .get(usize::from(index) / 64)
        .is_some_and(|word| word >> (index % 64) & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(codes: &[KeyCode]) -> Vec<u64> {
        let mut words = vec![0_u64; 8];
        for code in codes {
            words[usize::from(code.code()) / 64] |= 1 << (code.code() % 64);
        }
        words
    }

    #[test]
    fn reads_a_sysfs_key_bitmap() {
        // KEY_A (30) and KEY_LEFT (105), the highest word first
        let keys = key_bits("0 20000000000 40000000");
        assert!(bit(&keys, 30));
        assert!(bit(&keys, 105));
        assert!(!bit(&keys, 29));
        assert!(!bit(&keys, 1000));
    }

    #[test]
    fn tells_controllers_from_keyboards() {
        let pad = keys(&[KeyCode::BTN_SOUTH, KeyCode::BTN_EAST]);
        let keyboard = keys(&[KeyCode::KEY_A, KeyCode::KEY_ENTER, KeyCode::KEY_LEFT]);
        assert_eq!(classify(&pad, 3, "pad"), Some((Kind::Pad, true)));
        assert_eq!(classify(&keyboard, 3, "keys"), Some((Kind::Keyboard, true)));
        assert_eq!(
            classify(&keyboard, BUS_VIRTUAL, "remote"),
            Some((Kind::Keyboard, false))
        );
        // a keyboard that also has a south button is a controller
        let both = keys(&[
            KeyCode::BTN_SOUTH,
            KeyCode::KEY_A,
            KeyCode::KEY_ENTER,
            KeyCode::KEY_LEFT,
        ]);
        assert_eq!(classify(&both, 3, "x"), Some((Kind::Pad, true)));
    }

    #[test]
    fn leaves_out_mice_and_our_own_devices() {
        assert_eq!(classify(&keys(&[KeyCode::BTN_LEFT]), 3, "mouse"), None);
        let keyboard = keys(&[KeyCode::KEY_A, KeyCode::KEY_ENTER, KeyCode::KEY_LEFT]);
        assert_eq!(classify(&keyboard, 3, "retrobox virtual keyboard"), None);
    }
}
