//! The power button's page: sleep (and, outside games, restart and turn
//! off), then, on a Nintendo Switch, its performance profiles, the one in
//! use ticked. A page of choices of the side menu.

use super::icons::Icon;
use super::panel::{ListPage, ListRow, RowKind};
use crate::i18n::Translator;
use crate::system::performance::Performance;
use crate::system::power::PowerAction;

/// What a row of the page does when chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerRow {
    /// Done once the overlay is gone.
    Action(PowerAction),
    /// Change to this performance profile; the page stays.
    Mode(u32),
}

/// The page, and what each row does (nothing, for a header).
pub fn page(
    translator: &Translator,
    actions: &[PowerAction],
    performance: Option<&Performance>,
) -> (ListPage, Vec<Option<PowerRow>>) {
    let mut rows = Vec::new();
    let mut does = Vec::new();
    for &action in actions {
        rows.push(ListRow {
            label: translator.text(action.label_key()),
            kind: RowKind::Choice {
                checked: false,
                enabled: true,
            },
            icon: Some(Icon::of_power(action)),
        });
        does.push(Some(PowerRow::Action(action)));
    }
    if let Some(performance) = performance {
        rows.push(ListRow {
            label: translator.text("menu-performance"),
            kind: RowKind::Header,
            icon: None,
        });
        does.push(None);
        for mode in performance.modes() {
            rows.push(ListRow {
                label: mode.name.clone(),
                kind: RowKind::Choice {
                    checked: performance.current() == Some(mode.id),
                    enabled: true,
                },
                icon: None,
            });
            does.push(Some(PowerRow::Mode(mode.id)));
        }
    }
    let page = ListPage {
        title: translator.text("menu-power"),
        rows,
    };
    (page, does)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::performance::Mode;

    #[test]
    fn the_actions_without_a_switch() {
        let translator = Translator::new("en");
        let (page, does) = page(
            &translator,
            &[PowerAction::Standby, PowerAction::PowerOff],
            None,
        );
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            does,
            [
                Some(PowerRow::Action(PowerAction::Standby)),
                Some(PowerRow::Action(PowerAction::PowerOff))
            ]
        );
    }

    #[test]
    fn the_switch_has_its_profiles_under_them() {
        let translator = Translator::new("en");
        let mode = |id, name: &str| Mode {
            id,
            name: name.into(),
        };
        let performance = Performance::of(vec![mode(0, "Console"), mode(2, "OC CPU")], Some(2));
        let (page, does) = page(&translator, &[PowerAction::Standby], Some(&performance));
        let kinds: Vec<RowKind> = page.rows.iter().map(|row| row.kind).collect();
        let choice = |checked| RowKind::Choice {
            checked,
            enabled: true,
        };
        assert_eq!(
            kinds,
            [choice(false), RowKind::Header, choice(false), choice(true)]
        );
        assert_eq!(page.rows[3].label, "OC CPU");
        assert_eq!(does[1], None);
        assert_eq!(does[3], Some(PowerRow::Mode(2)));
    }
}
