//! Sleeping, restarting and turning the computer off, from the power
//! button's page: through systemd (`systemctl`) where it is the init, else
//! through elogind's `loginctl` (the other inits' logind); with neither,
//! the page offers nothing. Both let the active local session do it without
//! a password (polkit's default). EmulationStation, the games and the
//! session get SIGTERM from the shutdown and quit cleanly (SDL turns it into
//! a quit).

use std::path::Path;

use super::{installed, run};

/// What the power button's page offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Standby,
    Reboot,
    PowerOff,
}

impl PowerAction {
    /// The text of its row.
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Standby => "menu-standby",
            Self::Reboot => "menu-reboot",
            Self::PowerOff => "menu-poweroff",
        }
    }

    /// The verb of `systemctl` and `loginctl`.
    const fn verb(self) -> &'static str {
        match self {
            Self::Standby => "suspend",
            Self::Reboot => "reboot",
            Self::PowerOff => "poweroff",
        }
    }
}

/// Who does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manager {
    Systemd,
    Elogind,
}

impl Manager {
    fn find() -> Option<Self> {
        // systemd's own test of whether it is the init (sd_booted)
        if Path::new("/run/systemd/system").is_dir() {
            Some(Self::Systemd)
        } else if installed("loginctl") {
            Some(Self::Elogind)
        } else {
            None
        }
    }

    const fn program(self) -> &'static str {
        match self {
            Self::Systemd => "systemctl",
            Self::Elogind => "loginctl",
        }
    }
}

/// What the power button's page offers: sleep (when `standby`), and
/// outside games restarting and turning off. Empty when nothing can do it.
pub fn actions(in_game: bool, standby: bool) -> Vec<PowerAction> {
    if Manager::find().is_none() {
        log::info!("neither systemd nor elogind: no power button");
        return Vec::new();
    }
    offered(in_game, standby)
}

fn offered(in_game: bool, standby: bool) -> Vec<PowerAction> {
    let mut actions = Vec::new();
    if standby {
        actions.push(PowerAction::Standby);
    }
    if !in_game {
        actions.extend([PowerAction::Reboot, PowerAction::PowerOff]);
    }
    actions
}

/// Do it (once the overlay is gone).
pub fn perform(action: PowerAction) {
    let Some(manager) = Manager::find() else {
        return;
    };
    log::info!("{} {}", manager.program(), action.verb());
    run(manager.program(), &[action.verb()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_only_offer_sleep() {
        assert_eq!(offered(true, true), [PowerAction::Standby]);
        assert_eq!(offered(true, false), []);
        assert_eq!(
            offered(false, true),
            [
                PowerAction::Standby,
                PowerAction::Reboot,
                PowerAction::PowerOff
            ]
        );
        assert_eq!(
            offered(false, false),
            [PowerAction::Reboot, PowerAction::PowerOff]
        );
    }
}
