//! The audio page: where the system's sound goes (the outputs) and how the
//! card behind it is used (its profiles), as the sound settings of a
//! desktop have them. A page of choices of the side menu.

use super::panel::{ListPage, ListRow, RowKind};
use crate::i18n::Translator;
use crate::system::audio::devices::{Action, Card, Snapshot};

/// The page's texts, translated.
pub struct AudioMenu {
    title: String,
    outputs: String,
    profile: String,
    none: String,
    loading: String,
}

impl AudioMenu {
    pub fn new(translator: &Translator) -> Self {
        Self {
            title: translator.text("menu-audio").to_uppercase(),
            outputs: translator.text("audio-output"),
            profile: translator.text("audio-profile"),
            none: translator.text("audio-no-outputs"),
            loading: translator.text("audio-loading"),
        }
    }

    /// The page for the outputs and profiles known (None: not yet), and
    /// what each row asks for when chosen (nothing, for a header).
    ///
    /// The profiles are those of the default output's card. Its `Off` profile
    /// is left out: the card's outputs would go with it, and with them the
    /// way back to the profiles from this page.
    pub fn page(&self, snapshot: Option<&Snapshot>) -> (ListPage, Vec<Option<Action>>) {
        let mut rows = Vec::new();
        let mut actions = Vec::new();
        let mut push = |label: &str, kind: RowKind, action: Option<Action>| {
            rows.push(ListRow {
                label: label.to_owned(),
                kind,
            });
            actions.push(action);
        };
        match snapshot {
            None => push(&self.loading, RowKind::Header, None),
            Some(snapshot) if snapshot.outputs.is_empty() => {
                push(&self.none, RowKind::Header, None);
            }
            Some(snapshot) => {
                push(&self.outputs, RowKind::Header, None);
                for output in &snapshot.outputs {
                    push(
                        &output.label,
                        RowKind::Choice {
                            checked: output.default,
                            enabled: true,
                        },
                        Some(Action::SetDefault(output.id.clone())),
                    );
                }
                if let Some(card) = snapshot.default_card() {
                    self.push_profiles(card, &mut push);
                }
            }
        }
        let page = ListPage {
            title: self.title.clone(),
            rows,
        };
        (page, actions)
    }

    fn push_profiles(&self, card: &Card, push: &mut impl FnMut(&str, RowKind, Option<Action>)) {
        let mut profiles = card
            .profiles
            .iter()
            .filter(|profile| !profile.off)
            .peekable();
        if profiles.peek().is_none() {
            return;
        }
        push(
            &format!("{} · {}", self.profile, card.label),
            RowKind::Header,
            None,
        );
        for profile in profiles {
            push(
                &profile.label,
                RowKind::Choice {
                    checked: profile.active,
                    enabled: profile.available,
                },
                Some(Action::SetProfile {
                    card: card.id.clone(),
                    profile: profile.id.clone(),
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::audio::devices::{Output, Profile};

    fn menu() -> AudioMenu {
        AudioMenu::new(&Translator::new("en"))
    }

    fn profile(id: &str, active: bool, available: bool, off: bool) -> Profile {
        Profile {
            id: id.into(),
            label: format!("profile {id}"),
            active,
            available,
            off,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            outputs: vec![
                Output {
                    id: "1".into(),
                    label: "Speakers".into(),
                    default: true,
                    card: Some("c".into()),
                },
                Output {
                    id: "2".into(),
                    label: "Headphones".into(),
                    default: false,
                    card: None,
                },
            ],
            cards: vec![Card {
                id: "c".into(),
                label: "Built-in".into(),
                profiles: vec![
                    profile("a", true, true, false),
                    profile("b", false, false, false),
                    profile("off", false, true, true),
                ],
            }],
        }
    }

    #[test]
    fn lists_outputs_then_the_default_cards_profiles() {
        let (page, actions) = menu().page(Some(&snapshot()));
        assert_eq!(page.title, "AUDIO");
        let rows: Vec<_> = page
            .rows
            .iter()
            .map(|row| (row.label.as_str(), row.kind))
            .collect();
        assert_eq!(
            rows,
            [
                ("Output device", RowKind::Header),
                (
                    "Speakers",
                    RowKind::Choice {
                        checked: true,
                        enabled: true
                    }
                ),
                (
                    "Headphones",
                    RowKind::Choice {
                        checked: false,
                        enabled: true
                    }
                ),
                ("Profile · Built-in", RowKind::Header),
                (
                    "profile a",
                    RowKind::Choice {
                        checked: true,
                        enabled: true
                    }
                ),
                // not available: shown, but cannot be chosen; Off is not shown
                (
                    "profile b",
                    RowKind::Choice {
                        checked: false,
                        enabled: false
                    }
                ),
            ]
        );
        assert_eq!(actions.len(), rows.len());
        assert_eq!(actions[0], None);
        assert_eq!(actions[2], Some(Action::SetDefault("2".into())));
        assert_eq!(
            actions[4],
            Some(Action::SetProfile {
                card: "c".into(),
                profile: "a".into()
            })
        );
    }

    #[test]
    fn an_output_without_a_card_has_no_profiles() {
        let mut snapshot = snapshot();
        snapshot.apply(&Action::SetDefault("2".into()));
        let (page, _) = menu().page(Some(&snapshot));
        assert_eq!(page.rows.len(), 3);
    }

    #[test]
    fn says_when_it_is_loading_or_empty() {
        let (page, actions) = menu().page(None);
        assert_eq!(page.rows[0].label, "Loading…");
        assert_eq!(actions, [None]);
        let (page, _) = menu().page(Some(&Snapshot::default()));
        assert_eq!(page.rows[0].label, "No audio outputs found");
        assert_eq!(page.rows[0].kind, RowKind::Header);
    }
}
