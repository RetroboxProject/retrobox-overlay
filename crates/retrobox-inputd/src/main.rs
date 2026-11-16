//! retrobox-inputd: Retrobox's input daemon.
//!
//! Runs for the whole Retrobox session, reading every controller and
//! keyboard without taking them from EmulationStation or the games, and
//! opens the overlay (`retrobox-overlay`, which has the screen) when it is
//! asked for (see `daemon.rs`). It has no screen of its own.

mod daemon;

use std::process::ExitCode;

use retrobox_input::{hold, logging, paths};

const HELP: &str = "\
retrobox-inputd: Retrobox's input daemon

Usage: retrobox-inputd [--version] [--help]

Opens the overlay (retrobox-overlay) when a controller's Home/Guide button is
held, on Hotkey+B, or on Ctrl+Shift+O or Shift+F7, until stopped; started
again while it runs, it does nothing. Retrobox runs it with EmulationStation.
SIGUSR1 opens or closes the overlay.

Settings: hold_ms in the [overlay] section of overlay.ini, in /etc/retrobox
and ~/.config/retrobox.
Log: stderr from a terminal, else ~/.local/state/retrobox/logs/inputd.log;
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
    if let Some(unknown) = args.first() {
        eprintln!("retrobox-inputd: unknown argument {unknown:?} (see --help)");
        return ExitCode::from(2);
    }
    let _log = logging::init("inputd.log");
    match daemon::run(
        &paths::runtime_dir(),
        hold::hold_time(&paths::config_dirs()),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
