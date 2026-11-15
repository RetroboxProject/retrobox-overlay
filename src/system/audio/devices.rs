//! What the audio menu shows and asks for: the outputs, the profiles of the
//! cards they belong to, and the two changes it can make. The sound server
//! behind them is `PipeWire` or `PulseAudio` (see `pipewire.rs`, `pulse.rs`);
//! this is what both are turned into.

/// An audio output (a sink), as the sound server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// What the server calls it: `PipeWire`'s node id, `PulseAudio`'s sink name.
    pub id: String,
    pub label: String,
    /// Whether it is the system's default output.
    pub default: bool,
    /// The card it belongs to (a [`Card::id`]), when it has one.
    pub card: Option<String>,
}

/// One of a card's profiles: how its connectors are used (analog stereo,
/// HDMI, a headset's A2DP or hands-free...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// `PipeWire`'s profile index, `PulseAudio`'s profile name.
    pub id: String,
    pub label: String,
    pub active: bool,
    /// Whether it can be used now (an HDMI profile with no screen plugged in
    /// cannot).
    pub available: bool,
    /// The profile that turns the card off.
    pub off: bool,
}

/// A sound card (or Bluetooth device) and its profiles, best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// `PipeWire`'s device id, `PulseAudio`'s card name.
    pub id: String,
    pub label: String,
    pub profiles: Vec<Profile>,
}

/// The outputs and cards at a moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub outputs: Vec<Output>,
    pub cards: Vec<Card>,
}

/// What the menu asks the sound server for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Make an output (an [`Output::id`]) the system's default.
    SetDefault(String),
    /// Switch a card to one of its profiles.
    SetProfile { card: String, profile: String },
}

impl Snapshot {
    /// The system's default output.
    pub fn default_output(&self) -> Option<&Output> {
        self.outputs.iter().find(|output| output.default)
    }

    /// The card of the default output, whose profiles the menu offers.
    pub fn default_card(&self) -> Option<&Card> {
        let id = self.default_output()?.card.as_deref()?;
        self.cards.iter().find(|card| card.id == id)
    }

    /// Whether an action would change nothing: its output is the default, its
    /// profile the active one.
    pub fn is_current(&self, action: &Action) -> bool {
        match action {
            Action::SetDefault(id) => self.default_output().is_some_and(|output| output.id == *id),
            Action::SetProfile { card, profile } => self
                .cards
                .iter()
                .filter(|known| known.id == *card)
                .flat_map(|known| &known.profiles)
                .any(|known| known.id == *profile && known.active),
        }
    }

    /// Show an action as if it had already been done, until the sound server
    /// says so.
    pub fn apply(&mut self, action: &Action) {
        match action {
            Action::SetDefault(id) => {
                for output in &mut self.outputs {
                    output.default = output.id == *id;
                }
            }
            Action::SetProfile { card, profile } => {
                for card in self.cards.iter_mut().filter(|known| known.id == *card) {
                    for known in &mut card.profiles {
                        known.active = known.id == *profile;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, active: bool) -> Profile {
        Profile {
            id: id.into(),
            label: id.into(),
            active,
            available: true,
            off: false,
        }
    }

    fn snapshot() -> Snapshot {
        let output = |id: &str, default, card: Option<&str>| Output {
            id: id.into(),
            label: id.into(),
            default,
            card: card.map(Into::into),
        };
        Snapshot {
            outputs: vec![
                output("10", true, Some("1")),
                output("11", false, Some("2")),
                output("12", false, None),
            ],
            cards: vec![
                Card {
                    id: "1".into(),
                    label: "Built-in".into(),
                    profiles: vec![profile("a", true), profile("b", false)],
                },
                Card {
                    id: "2".into(),
                    label: "HDMI".into(),
                    profiles: vec![profile("c", true)],
                },
            ],
        }
    }

    #[test]
    fn default_card_is_the_default_outputs() {
        let mut snapshot = snapshot();
        assert_eq!(snapshot.default_card().unwrap().label, "Built-in");
        snapshot.apply(&Action::SetDefault("11".into()));
        assert_eq!(snapshot.default_card().unwrap().label, "HDMI");
        // an output without a card has no profiles
        snapshot.apply(&Action::SetDefault("12".into()));
        assert!(snapshot.default_card().is_none());
        snapshot.apply(&Action::SetDefault("nope".into()));
        assert!(snapshot.default_output().is_none());
    }

    #[test]
    fn knows_what_is_current() {
        let snapshot = snapshot();
        assert!(snapshot.is_current(&Action::SetDefault("10".into())));
        assert!(!snapshot.is_current(&Action::SetDefault("11".into())));
        let profile = |card: &str, profile: &str| Action::SetProfile {
            card: card.into(),
            profile: profile.into(),
        };
        assert!(snapshot.is_current(&profile("1", "a")));
        assert!(!snapshot.is_current(&profile("1", "b")));
        assert!(!snapshot.is_current(&profile("nope", "a")));
    }

    #[test]
    fn a_profile_change_moves_only_its_cards_mark() {
        let mut snapshot = snapshot();
        snapshot.apply(&Action::SetProfile {
            card: "1".into(),
            profile: "b".into(),
        });
        let active = |card: usize| {
            snapshot.cards[card]
                .profiles
                .iter()
                .filter(|profile| profile.active)
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(active(0), ["b"]);
        assert_eq!(active(1), ["c"]);
    }
}
