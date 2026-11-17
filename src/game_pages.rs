//! The running game's own pages in the menu: its manual, its page (what
//! game.xml says, and the time played), its controls (its pad-to-keyboard
//! profile) and its achievements (RetroAchievements). Each shows only when
//! it has something to show.

use std::time::Duration;

use chrono::NaiveDate;

use crate::achievements::{Account, GameId};
use crate::controls::{self, Mapping};
use crate::i18n::Translator;
use crate::manual::Manual;
use crate::playtime::{self, PlayClock};
use crate::session::{GameInfo, Session};
use crate::ui::panel::{InfoSheet, MenuAction, MenuItem};

#[derive(Default)]
pub struct GamePages {
    pub manual: Option<Manual>,
    pub info: Option<InfoSheet>,
    pub controls: Vec<Mapping>,
    /// The time played, shown in the status line.
    pub play: Option<PlayClock>,
    /// The RetroAchievements account and the game, when both are known.
    pub cheevos: Option<(Account, GameId)>,
}

impl GamePages {
    pub fn new(session: &Session, info: &GameInfo, translator: &Translator) -> Self {
        let play = PlayClock::new(session, info.game_time);
        Self {
            manual: info.manual.as_deref().and_then(Manual::open),
            info: sheet(info, play.as_ref(), translator),
            controls: session
                .pad_profile
                .as_deref()
                .map(|profile| controls::read(profile, translator))
                .unwrap_or_default(),
            play,
            cheevos: session
                .achievements
                .clone()
                .filter(|account| {
                    !account.username.is_empty()
                        && (!account.token.is_empty() || !account.password.is_empty())
                })
                .zip(Some(info.cheevos.clone()).filter(GameId::known)),
        }
    }

    /// The menu's entries for the pages there are.
    pub fn items(&self, translator: &Translator) -> Vec<MenuItem> {
        [
            (self.manual.is_some(), "menu-manual", MenuAction::Manual),
            (self.info.is_some(), "menu-info", MenuAction::Info),
            (
                !self.controls.is_empty(),
                "menu-controls",
                MenuAction::Controls,
            ),
            (
                self.cheevos.is_some(),
                "menu-achievements",
                MenuAction::Achievements,
            ),
        ]
        .into_iter()
        .filter(|(shown, ..)| *shown)
        .map(|(_, key, action)| MenuItem::new(translator.text(key), action))
        .collect()
    }
}

/// The game's page, when game.xml says more than its name.
fn sheet(info: &GameInfo, play: Option<&PlayClock>, translator: &Translator) -> Option<InfoSheet> {
    let released = info
        .release_date
        .get(..8)
        .and_then(|date| NaiveDate::parse_from_str(date, "%Y%m%d").ok())
        .map_or_else(
            || info.release_year().to_owned(),
            |date| date.format_localized("%x", translator.locale()).to_string(),
        );
    let rating = info.rating.map(|rating| {
        // ES's stars: halves
        let stars = (rating * 10.0).round() / 2.0;
        format!("{stars} / 5")
    });
    let played = play.map_or(info.game_time, PlayClock::total);
    let play_count = if info.play_count > 0 {
        info.play_count.to_string()
    } else {
        String::new()
    };
    let play_time = if played >= Duration::from_mins(1) {
        playtime::duration_text(played, translator)
    } else {
        String::new()
    };
    let facts = [
        ("info-developer", info.developer.clone()),
        ("info-publisher", info.publisher.clone()),
        ("info-released", released),
        ("info-genre", info.genre.clone()),
        ("info-players", info.players.clone()),
        ("info-rating", rating.unwrap_or_default()),
        ("info-play-count", play_count),
        ("info-play-time", play_time),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| (translator.text(key), value))
    .collect::<Vec<_>>();
    let sheet = InfoSheet {
        title: translator.text("menu-info"),
        image: info.image.clone().filter(|image| image.is_file()),
        facts,
        description: info.description.clone(),
    };
    (sheet.image.is_some() || !sheet.facts.is_empty() || !sheet.description.is_empty())
        .then_some(sheet)
}
