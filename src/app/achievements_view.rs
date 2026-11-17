//! The achievements page's side of the menu: fetching them (on a thread of
//! its own, the first time the page opens) and showing them as they come,
//! the list first, then the badges. They are fetched once per opening of the
//! overlay: closing the page and opening it again shows them at once.

use smithay_client_toolkit::reexports::calloop::channel::{self, Event};
use tiny_skia::Pixmap;

use super::State;
use crate::achievements::{self, Achievement, Earned};
use crate::i18n::Translator;
use crate::ui::picture;
use retrobox_input::paths;

/// Where the achievements are.
#[derive(Default)]
pub(super) enum Cheevos {
    #[default]
    NotAsked,
    Coming,
    Failed,
    Came {
        list: Vec<Achievement>,
        /// Each one's badge, once fetched.
        badges: Option<Vec<Option<Pixmap>>>,
    },
}

/// What the fetching thread tells.
enum News {
    List(anyhow::Result<Vec<Achievement>>),
    Badges(Vec<Option<Pixmap>>),
}

impl State {
    /// Show the achievements page (fetching them the first time).
    pub(super) fn open_achievements(&mut self) {
        let Some((panel, _)) = self.panel.as_mut() else {
            return;
        };
        if panel.page_open() {
            return;
        }
        let translator = &self.content.translator;
        panel.open_achievements(
            translator.text("menu-achievements"),
            translator.text("achievements-loading"),
        );
        if matches!(self.cheevos, Cheevos::NotAsked) {
            let size = panel.badge_size();
            self.fetch_achievements(size);
        }
        self.show_achievements();
        self.draw_menu_help();
    }

    /// Fetch them, and their badges `badge_size` pixels wide.
    fn fetch_achievements(&mut self, badge_size: u32) {
        let Some((account, game)) = self.content.cheevos.clone() else {
            return;
        };
        let (sender, news) = channel::channel();
        let watched = self.handle.insert_source(news, |event, &mut (), state| {
            if let Event::Msg(news) = event {
                state.achievements_news(news);
            }
        });
        if watched.is_err() {
            return;
        }
        self.cheevos = Cheevos::Coming;
        std::thread::spawn(move || {
            let list = achievements::fetch(&account, &game);
            let shown = list.as_ref().ok().cloned();
            if sender.send(News::List(list)).is_err() {
                return;
            }
            let Some(list) = shown else {
                return;
            };
            let dir = paths::cache_dir().join("achievements");
            if let Err(err) = achievements::fetch_badges(&list, &dir) {
                log::warn!("the achievements' badges: {err:#}");
            }
            let side = badge_size as f32;
            let badges = list
                .iter()
                .map(|achievement| {
                    let badge = achievement.badge_path(&dir);
                    badge
                        .is_file()
                        .then(|| picture::load(&badge, side, side))
                        .flatten()
                })
                .collect();
            sender.send(News::Badges(badges)).ok();
        });
    }

    fn achievements_news(&mut self, news: News) {
        match news {
            News::List(Ok(list)) => {
                self.cheevos = Cheevos::Came { list, badges: None };
            }
            News::List(Err(err)) => {
                log::warn!("the achievements: {err:#}");
                self.cheevos = Cheevos::Failed;
            }
            News::Badges(fetched) => {
                if let Cheevos::Came { badges, .. } = &mut self.cheevos {
                    *badges = Some(fetched);
                }
            }
        }
        self.show_achievements();
    }

    /// Tell the page what there is (it ignores it when it is not open).
    fn show_achievements(&mut self) {
        let Some((panel, _)) = self.panel.as_mut() else {
            return;
        };
        let translator = &self.content.translator;
        match &self.cheevos {
            Cheevos::NotAsked | Cheevos::Coming => return,
            Cheevos::Failed => panel.achievements_failed(translator.text("achievements-failed")),
            Cheevos::Came { list, badges } => {
                panel.show_achievements(
                    list.clone(),
                    summary(list, translator),
                    translator.text("achievements-none"),
                );
                if let Some(badges) = badges {
                    panel.show_badges(badges.clone());
                }
            }
        }
        self.draw_panel();
    }
}

/// "12 of 40 · 120 of 400 points" (empty without achievements).
fn summary(list: &[Achievement], translator: &Translator) -> String {
    if list.is_empty() {
        return String::new();
    }
    let earned: Vec<&Achievement> = list
        .iter()
        .filter(|item| item.earned != Earned::No)
        .collect();
    let count = |items: usize| i64::try_from(items).unwrap_or(i64::MAX);
    let points = |items: &mut dyn Iterator<Item = &Achievement>| {
        i64::try_from(items.map(|item| item.points).sum::<u64>()).unwrap_or(i64::MAX)
    };
    translator.text_args(
        "achievements-summary",
        &[
            ("earned", count(earned.len())),
            ("total", count(list.len())),
            ("points", points(&mut earned.iter().copied())),
            ("max", points(&mut list.iter())),
        ],
    )
}
