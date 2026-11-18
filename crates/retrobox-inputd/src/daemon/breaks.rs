//! The break reminder (see `retrobox_input::breaks`): while a game runs,
//! a timer waits for its next reminder, and then the toast shows
//! (`retrobox-overlay --toast break`). With the menu open, or the game
//! starting or ending, it shows a little later. Opening the menu takes a
//! toast still on screen away.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use calloop::RegistrationToken;
use calloop::timer::{TimeoutAction, Timer};
use retrobox_input::breaks::{self, Reminder};
use retrobox_input::{launch, paths};
use serde_json::Value;

use super::State;

/// When a reminder could not show: how long until it tries again.
const RETRY: Duration = Duration::from_mins(1);
/// A timer may fire a little early: the reminder it was set for counts as
/// shown.
const SLACK: Duration = Duration::from_secs(5);

/// The reminders of the running game.
#[derive(Debug, Default)]
pub(super) struct Breaks {
    /// The game's start, in clock ticks since boot (0: no game).
    start_time: u64,
    reminder: Option<Reminder>,
    timer: Option<RegistrationToken>,
    /// The toast, while it shows.
    toast: Option<Child>,
}

/// The game's start as the session says it (0: no game, or unknown).
fn session_start(runtime_dir: &Path) -> u64 {
    std::fs::read_to_string(runtime_dir.join(super::SESSION_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|session| session.get("start_time")?.as_u64())
        .unwrap_or(0)
}

impl State {
    /// The session changed: wait for its game's next reminder, or stop
    /// (no game, or reminders off). The same game goes on as it was.
    pub(super) fn follow_breaks(&mut self) {
        let start_time = session_start(&self.runtime_dir);
        if start_time == self.breaks.start_time {
            return;
        }
        if let Some(timer) = self.breaks.timer.take() {
            self.handle.remove(timer);
        }
        self.breaks.start_time = start_time;
        self.breaks.reminder = (start_time > 0)
            .then(|| Reminder::load(&paths::config_dirs()))
            .flatten();
        let Some(wait) = self.breaks.wait(Duration::ZERO) else {
            return;
        };
        log::info!("a break is suggested in {} min", wait.as_secs() / 60);
        let timer = self
            .handle
            .insert_source(Timer::from_duration(wait), |_, &mut (), state| {
                state.break_due()
            });
        match timer {
            Ok(timer) => self.breaks.timer = Some(timer),
            Err(err) => log::warn!("no break reminder: {}", err.error),
        }
    }

    /// The reminder's time came: the toast, unless something is on screen.
    fn break_due(&mut self) -> TimeoutAction {
        self.reap();
        if self.overlay.is_some() || launch::busy() {
            return TimeoutAction::ToDuration(RETRY);
        }
        self.breaks.show_toast();
        if let Some(wait) = self.breaks.wait(SLACK) {
            TimeoutAction::ToDuration(wait)
        } else {
            self.breaks.timer = None;
            TimeoutAction::Drop
        }
    }
}

impl Breaks {
    /// How long until the next reminder, counting `shown` more as played
    /// (None: no game, or no more reminders).
    fn wait(&self, shown: Duration) -> Option<Duration> {
        let reminder = self.reminder?;
        let played = breaks::game_age(self.start_time)?;
        let next = reminder.next(played + shown)?;
        Some(next.saturating_sub(played))
    }

    fn show_toast(&mut self) {
        self.hide_toast();
        let shown = Command::new(paths::sibling_command("retrobox-overlay"))
            .args(["--toast", "break"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match shown {
            Ok(child) => {
                log::info!("break suggested ({})", child.id());
                self.toast = Some(child);
            }
            Err(err) => log::error!("cannot suggest a break: {err}"),
        }
    }

    /// The menu opens: the toast goes.
    pub(super) fn hide_toast(&mut self) {
        if let Some(mut toast) = self.toast.take() {
            toast.kill().ok();
            toast.wait().ok();
        }
    }

    /// Forget the toast once it is gone.
    pub(super) fn reap(&mut self) {
        if self
            .toast
            .as_mut()
            .is_some_and(|toast| !matches!(toast.try_wait(), Ok(None)))
        {
            self.toast = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_says_when_the_game_started() {
        let dir =
            std::env::temp_dir().join(format!("retrobox-break-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(session_start(&dir), 0);
        std::fs::write(dir.join("session.json"), r#"{"start_time": 1234, "x": 1}"#).unwrap();
        assert_eq!(session_start(&dir), 1234);
        std::fs::write(dir.join("session.json"), r#"{"emulator": "pcsx2"}"#).unwrap();
        assert_eq!(session_start(&dir), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_game_no_wait() {
        let breaks = Breaks {
            reminder: Some(Reminder::default()),
            ..Breaks::default()
        };
        assert_eq!(breaks.wait(Duration::ZERO), None);
        // a game started at boot has played as long as the system is up
        let running = Breaks {
            start_time: 1,
            reminder: Some(Reminder {
                after: Duration::from_secs(u64::from(u32::MAX)),
                every: None,
            }),
            ..Breaks::default()
        };
        assert!(running.wait(Duration::ZERO).is_some());
    }
}
