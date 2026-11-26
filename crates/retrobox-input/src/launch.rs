//! Where a game's launch is: `launch.json` in the runtime directory, written
//! by `retrobox-launch` for as long as it runs.
//!
//! ```json
//! {"version": 1, "pid": 4242, "start_time": 123456, "phase": "starting"}
//! ```
//!
//! The phase is `starting` from the moment the launcher starts (the game is
//! being prepared; an emulator may be installed first, on the task screen),
//! `running` once the emulator runs, and `ending` once the game is being
//! closed (`retrobox-emukill`, or the emulator quitting) until the launcher
//! is gone and the file with it. The overlay has nothing to offer while a
//! game starts or ends: it does not open then, and it closes when either
//! begins. `pid` and `start_time` (from `/proc/<pid>/stat`) tell a file left
//! by a launcher that died: it counts as no launch at all.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::paths;

/// The file's name, in the runtime directory.
pub const LAUNCH_FILE: &str = "launch.json";

/// Where the launch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Running,
    Ending,
}

impl Phase {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "starting" => Some(Self::Starting),
            "running" => Some(Self::Running),
            "ending" => Some(Self::Ending),
            _ => None,
        }
    }

    /// Whether the overlay stays away: a game starting or ending.
    pub const fn keeps_overlay_away(self) -> bool {
        matches!(self, Self::Starting | Self::Ending)
    }
}

/// The launch file in the runtime directory.
pub fn path() -> PathBuf {
    paths::runtime_dir().join(LAUNCH_FILE)
}

/// The phase of the launch under way, if any.
pub fn phase() -> Option<Phase> {
    phase_in(&path())
}

/// Whether a game is starting or ending (the overlay stays away).
pub fn busy() -> bool {
    phase().is_some_and(Phase::keeps_overlay_away)
}

/// The phase `path` tells, if its launcher still runs.
pub fn phase_in(path: &Path) -> Option<Phase> {
    let text = fs::read_to_string(path).ok()?;
    let (pid, start_time, phase) = parse(&text)?;
    alive(pid, start_time).then_some(phase)
}

/// The launcher's pid and start time, and the phase.
fn parse(text: &str) -> Option<(u32, u64, Phase)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let pid = u32::try_from(value.get("pid")?.as_u64()?).ok()?;
    let start_time = value.get("start_time").and_then(Value::as_u64).unwrap_or(0);
    let phase = Phase::parse(value.get("phase")?.as_str()?)?;
    Some((pid, start_time, phase))
}

/// Whether the process `pid` runs and started at `start_time` (0: any).
fn alive(pid: u32, start_time: u64) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    start_time == 0 || stat_start_time(&stat) == Some(start_time)
}

/// The start time in a `/proc/<pid>/stat` line (its 22nd field): after the
/// command name, which may hold spaces and parentheses.
fn stat_start_time(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_launchers_file() {
        assert_eq!(
            parse(r#"{"version": 1, "pid": 42, "start_time": 7, "phase": "ending"}"#),
            Some((42, 7, Phase::Ending))
        );
        assert_eq!(parse(r#"{"pid": 42, "phase": "later"}"#), None);
        assert_eq!(parse("{"), None);
    }

    #[test]
    fn reads_the_start_time_after_an_odd_name() {
        let stat = "123 (a (b) c) S 1 123 123 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 98765 1000";
        assert_eq!(stat_start_time(stat), Some(98765));
    }

    #[test]
    fn a_running_launcher_counts_and_a_dead_one_does_not() {
        let dir = std::env::temp_dir().join(format!("launch-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(LAUNCH_FILE);
        let me = std::process::id();
        let born =
            stat_start_time(&fs::read_to_string(format!("/proc/{me}/stat")).unwrap()).unwrap();

        fs::write(
            &file,
            format!(r#"{{"pid": {me}, "start_time": {born}, "phase": "starting"}}"#),
        )
        .unwrap();
        assert_eq!(phase_in(&file), Some(Phase::Starting));
        // the pid reused by another process
        fs::write(
            &file,
            format!(
                r#"{{"pid": {me}, "start_time": {}, "phase": "starting"}}"#,
                born + 1
            ),
        )
        .unwrap();
        assert_eq!(phase_in(&file), None);
        fs::remove_file(&file).unwrap();
        assert_eq!(phase_in(&file), None);
        fs::remove_dir(&dir).ok();
    }
}
