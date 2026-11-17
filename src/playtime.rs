//! How long the game has been played: this time (since its emulator
//! started) and in all (EmulationStation's `gametime`, which counts the
//! times before, plus this one), shown in the menu's status line and on the
//! game's page.

use std::time::{Duration, Instant};

use crate::i18n::Translator;
use crate::session::Session;

#[derive(Debug, Clone, Copy)]
pub struct PlayClock {
    /// When the game started.
    started: Instant,
    /// The time played before.
    before: Duration,
}

impl PlayClock {
    /// The running game's clock, `before` played before it started.
    pub fn new(session: &Session, before: Duration) -> Option<Self> {
        let started = Instant::now().checked_sub(session.running_for()?)?;
        Some(Self { started, before })
    }

    pub fn session(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn total(&self) -> Duration {
        self.before + self.session()
    }

    /// "25 min", or "25 min · 3 h 10 min in total" when it was played before.
    pub fn text(&self, translator: &Translator) -> String {
        let session = duration_text(self.session(), translator);
        if self.before.is_zero() {
            return session;
        }
        translator.text_strings(
            "play-time",
            &[
                ("session", &session),
                ("total", &duration_text(self.total(), translator)),
            ],
        )
    }
}

/// "25 min", "3 h", "3 h 10 min" (in whole minutes).
pub fn duration_text(duration: Duration, translator: &Translator) -> String {
    let minutes = i64::try_from(duration.as_secs() / 60).unwrap_or(i64::MAX);
    let (hours, minutes) = (minutes / 60, minutes % 60);
    match (hours, minutes) {
        (0, minutes) => translator.text_with("duration-minutes", "minutes", minutes),
        (hours, 0) => translator.text_with("duration-hours", "hours", hours),
        (hours, minutes) => translator.text_args(
            "duration-hours-minutes",
            &[("hours", hours), ("minutes", minutes)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_in_hours_and_minutes() {
        let translator = Translator::new("en_US");
        let text =
            |minutes: u64| duration_text(Duration::from_secs(minutes * 60 + 59), &translator);
        assert_eq!(text(25), "25 min");
        assert_eq!(text(120), "2 h");
        assert_eq!(text(190), "3 h 10 min");
    }

    #[test]
    fn this_time_and_in_all() {
        let translator = Translator::new("en_US");
        let clock = |before: u64| PlayClock {
            started: Instant::now().checked_sub(Duration::from_mins(25)).unwrap(),
            before: Duration::from_mins(before),
        };
        assert_eq!(clock(0).text(&translator), "25 min");
        assert_eq!(clock(165).text(&translator), "25 min · 3 h 10 min in total");
    }
}
