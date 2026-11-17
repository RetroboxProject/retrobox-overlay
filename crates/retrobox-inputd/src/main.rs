//! retrobox-inputd: Retrobox's input daemon.
//!
//! Runs for the whole Retrobox session, reading every controller and
//! keyboard without taking them from EmulationStation or the games, and
//! opens the overlay (`retrobox-overlay`, which has the screen) when it is
//! asked for (see `daemon.rs`); while a game runs, it maps the game's
//! controllers to keys (see `mapper`). It has no screen of its own.

mod daemon;
mod mapper;
mod parent;
mod shield;

use std::process::ExitCode;

use retrobox_input::{hold, logging, paths};

const HELP: &str = "\
retrobox-inputd: Retrobox's input daemon

Usage: retrobox-inputd [--parent PID] [--version] [--help]

Opens the overlay (retrobox-overlay) when a controller's Home/Guide button is
held, on Hotkey+B, or on Ctrl+Shift+O or Shift+F7, until stopped; started
again while it runs, it does nothing. Retrobox runs it with EmulationStation.
SIGUSR1 opens or closes the overlay. While a game runs with a pad profile
(pad_profile in session.json), it maps the game's controllers and the
keyboards to keys, mouse and commands, as gamepadly did.

  --parent PID   stop when the process PID (Retrobox's) ends; without it,
                 RETROBOX_PARENT_PID in the environment (how systemd's
                 retrobox-inputd.service passes it)

Under systemd, Retrobox starts the user unit retrobox-inputd.service, which
reads $XDG_RUNTIME_DIR/retrobox/inputd.env (the display and the parent).

Settings: hold_ms in the [overlay] section of overlay.ini, in /etc/retrobox
and ~/.config/retrobox.
Log: stderr from a terminal or systemd's journal, else
~/.local/state/retrobox/logs/inputd.log;
RUST_LOG=debug for details.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("retrobox-inputd {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let parent = match args.as_slice() {
        [] => std::env::var(parent::PARENT_VARIABLE)
            .ok()
            .and_then(|pid| parent::parse(&pid)),
        [flag, pid] if flag == "--parent" => {
            let Some(pid) = parent::parse(pid) else {
                eprintln!("retrobox-inputd: --parent needs a PID");
                return ExitCode::from(2);
            };
            Some(pid)
        }
        _ => {
            eprintln!("retrobox-inputd: unknown arguments {args:?} (see --help)");
            return ExitCode::from(2);
        }
    };
    let _log = logging::init("inputd.log");
    match daemon::run(
        &paths::runtime_dir(),
        hold::hold_time(&paths::config_dirs()),
        parent,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
