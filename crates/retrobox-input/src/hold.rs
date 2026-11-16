//! How long a controller's dedicated hotkey (Home/Guide) is held to open the
//! overlay: `hold_ms` in the `[overlay]` section of `overlay.ini`.

use std::path::Path;
use std::time::Duration;

use ini::Ini;

/// The overlay's settings file, in each configuration directory.
pub const CONFIG_FILE: &str = "overlay.ini";

const DEFAULT_MS: u64 = 750;
const MIN_MS: u64 = 300;
const MAX_MS: u64 = 3000;

/// The hold time the configuration directories set (lowest priority first;
/// the last one that sets it wins), kept in a sane range.
pub fn hold_time<P: AsRef<Path>>(dirs: &[P]) -> Duration {
    let ms = dirs
        .iter()
        .filter_map(|dir| {
            let path = dir.as_ref().join(CONFIG_FILE);
            if !path.exists() {
                return None;
            }
            match Ini::load_from_file(&path) {
                Ok(ini) => ini
                    .section(Some("overlay"))
                    .and_then(|section| section.get("hold_ms"))
                    .and_then(|value| value.trim().parse::<u64>().ok()),
                Err(err) => {
                    log::warn!("cannot read {}: {err}", path.display());
                    None
                }
            }
        })
        .last()
        .unwrap_or(DEFAULT_MS);
    Duration::from_millis(ms.clamp(MIN_MS, MAX_MS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_wins_and_the_value_is_kept_in_range() {
        let dir = std::env::temp_dir().join(format!("input-hold-{}", std::process::id()));
        let (system, user) = (dir.join("etc"), dir.join("user"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        assert_eq!(
            hold_time(&[&system, &user]),
            Duration::from_millis(DEFAULT_MS)
        );
        std::fs::write(system.join(CONFIG_FILE), "[overlay]\nhold_ms=1200\n").unwrap();
        assert_eq!(hold_time(&[&system, &user]), Duration::from_millis(1200));
        std::fs::write(user.join(CONFIG_FILE), "[overlay]\nhold_ms=50\n").unwrap();
        assert_eq!(hold_time(&[&system, &user]), Duration::from_millis(MIN_MS));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
