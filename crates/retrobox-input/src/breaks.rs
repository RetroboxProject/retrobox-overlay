//! The break reminder: after a while playing (two hours by default), and
//! then every so often (half an hour), the overlay suggests a break, in a
//! toast at the top of the screen (`retrobox-overlay --toast break`).
//!
//! The input daemon, which follows the game's session, reads `[break]` in
//! `overlay.ini` when a game starts and shows the toasts:
//!
//! ```ini
//! [break]
//! after_min = 120   ; minutes of play before the first one (0: never)
//! every_min = 30    ; then again every so many minutes (0: only once)
//! ```
//!
//! The time counted is the game's session: since its emulator started.

use std::path::Path;
use std::time::Duration;

use ini::Ini;

/// The overlay's settings file, in each configuration directory.
const CONFIG_FILE: &str = "overlay.ini";
const SECTION: &str = "break";
const MINUTE: Duration = Duration::from_mins(1);

/// When the reminders come, in time played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder {
    /// The first one.
    pub after: Duration,
    /// The ones after it, this far apart (None: only the first).
    pub every: Option<Duration>,
}

impl Default for Reminder {
    fn default() -> Self {
        Self {
            after: 120 * MINUTE,
            every: Some(30 * MINUTE),
        }
    }
}

impl Reminder {
    /// The reminder `overlay.ini` sets in `dirs` (lowest priority first);
    /// None when it is off.
    pub fn load<P: AsRef<Path>>(dirs: &[P]) -> Option<Self> {
        let mut minutes = (120, 30);
        for dir in dirs {
            let path = dir.as_ref().join(CONFIG_FILE);
            if !path.exists() {
                continue;
            }
            match Ini::load_from_file(&path) {
                Ok(ini) => minutes = read(&ini, minutes),
                Err(err) => log::warn!("cannot read {}: {err}", path.display()),
            }
        }
        Self::from_minutes(minutes)
    }

    /// `after` and `every` in minutes (0: off, only once).
    fn from_minutes((after, every): (u64, u64)) -> Option<Self> {
        (after > 0).then(|| Self {
            after: MINUTE * u32::try_from(after).unwrap_or(u32::MAX),
            every: (every > 0).then(|| MINUTE * u32::try_from(every).unwrap_or(u32::MAX)),
        })
    }

    /// The time played at which the first reminder after `played` comes
    /// (None: there are no more).
    pub fn next(&self, played: Duration) -> Option<Duration> {
        let Some(since) = played.checked_sub(self.after) else {
            return Some(self.after);
        };
        let every = self.every.filter(|every| !every.is_zero())?;
        let past = since.as_millis() / every.as_millis();
        let count = u32::try_from(past + 1).ok()?;
        self.after.checked_add(every.checked_mul(count)?)
    }
}

/// `[break]`'s minutes over `minutes` (a value that is no number is
/// ignored).
fn read(ini: &Ini, mut minutes: (u64, u64)) -> (u64, u64) {
    let Some(section) = ini.section(Some(SECTION)) else {
        return minutes;
    };
    for (key, slot) in [("after_min", &mut minutes.0), ("every_min", &mut minutes.1)] {
        if let Some(value) = section.get(key) {
            match value.trim().parse() {
                Ok(value) => *slot = value,
                Err(_) => log::warn!("overlay.ini: [{SECTION}] {key} is no number: {value:?}"),
            }
        }
    }
    minutes
}

/// How long a game has run: the age of its emulator, `start_time` being
/// its start in clock ticks since boot (`/proc/<pid>/stat`, as the session
/// says it). None when it is not known.
pub fn game_age(start_time: u64) -> Option<Duration> {
    if start_time == 0 {
        return None;
    }
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let ticks = rustix::param::clock_ticks_per_second();
    let started = start_time as f64 / ticks as f64;
    Some(Duration::from_secs_f64((uptime - started).max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minutes(count: u64) -> Duration {
        MINUTE * u32::try_from(count).unwrap()
    }

    #[test]
    fn first_after_two_hours_then_every_half_hour() {
        let reminder = Reminder::default();
        assert_eq!(reminder.next(Duration::ZERO), Some(minutes(120)));
        assert_eq!(reminder.next(minutes(119)), Some(minutes(120)));
        // at a reminder, the next one
        assert_eq!(reminder.next(minutes(120)), Some(minutes(150)));
        assert_eq!(reminder.next(minutes(140)), Some(minutes(150)));
        assert_eq!(reminder.next(minutes(181)), Some(minutes(210)));
    }

    #[test]
    fn once_or_never() {
        let once = Reminder::from_minutes((90, 0)).unwrap();
        assert_eq!(once.next(minutes(10)), Some(minutes(90)));
        assert_eq!(once.next(minutes(90)), None);
        assert_eq!(Reminder::from_minutes((0, 30)), None);
    }

    #[test]
    fn the_user_s_file_wins_and_bad_values_are_ignored() {
        let system = Ini::load_from_str("[break]\nafter_min = 60\nevery_min = 20\n").unwrap();
        let user = Ini::load_from_str("[break]\nevery_min = soon\n").unwrap();
        assert_eq!(read(&user, read(&system, (120, 30))), (60, 20));
        let off = Ini::load_from_str("[break]\nafter_min = 0\n").unwrap();
        assert_eq!(Reminder::from_minutes(read(&off, (120, 30))), None);
    }

    #[test]
    fn loads_from_the_directories() {
        let dir = std::env::temp_dir().join(format!("retrobox-breaks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CONFIG_FILE), "[break]\nafter_min = 45\n").unwrap();
        let reminder = Reminder::load(&[dir.as_path(), Path::new("/nonexistent")]);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(
            reminder,
            Some(Reminder {
                after: minutes(45),
                every: Some(minutes(30)),
            })
        );
    }

    #[test]
    fn an_unknown_start_has_no_age() {
        assert_eq!(game_age(0), None);
        // started at boot: as old as the system
        assert!(game_age(1).is_some());
    }
}
