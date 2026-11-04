//! retrobox-overlay: Retrobox's in-game menu.
//!
//! Started by the hotkey (Home pressed alone, Hotkey+A) or Shift+F7 through
//! gamepadly, it shows a side menu styled like EmulationStation's over the
//! dimmed game, with a clock. Started again while open, it closes the open
//! one. Without a running game it does nothing (unless `--demo`).

mod app;
mod config;
mod i18n;
mod input;
mod instance;
mod paths;
mod session;
mod theme;
mod ui;

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::Result;

use crate::app::{Content, Outcome};
use crate::config::Config;
use crate::i18n::Translator;
use crate::input::Pad;
use crate::instance::Instance;
use crate::session::{GameInfo, Session};
use crate::theme::MenuTheme;
use crate::ui::panel::{MenuAction, MenuItem};
use crate::ui::text::Fonts;

const HELP: &str = "\
retrobox-overlay: Retrobox's in-game menu

Usage: retrobox-overlay [--demo] [--version] [--help]

Shows the overlay over the running game (see session.json in Retrobox's
runtime directory), or closes it when it is already open.

  --demo     show it even without a running game (to try a theme)
  --version  print the version
  --help     print this help

Settings: overlay.ini in /etc/retrobox and ~/.config/retrobox.
Log: RUST_LOG=debug for details.";

fn main() -> ExitCode {
    let launched = std::time::Instant::now();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("retrobox-overlay {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let demo = args.iter().any(|arg| arg == "--demo");

    match run(demo, launched) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(demo: bool, launched: std::time::Instant) -> Result<()> {
    let runtime_dir = paths::runtime_dir();
    let _lock = match instance::acquire(&runtime_dir)? {
        Instance::First(lock) => lock,
        Instance::Running(pid) => {
            instance::close_running(pid);
            return Ok(());
        }
    };

    let session = Session::load(&runtime_dir);
    if session.is_none() && !demo {
        log::info!("no game running: nothing to show");
        return Ok(());
    }

    let theme_path = session
        .as_ref()
        .and_then(Session::menu_theme_path)
        .unwrap_or_else(|| runtime_dir.join("es-menu-theme.json"));
    let theme = MenuTheme::load(Some(&theme_path));
    let config = Config::load(&paths::config_dirs());
    let language = i18n::preferred_language(&theme.language);
    let translator = Translator::new(&language);

    let info = session.as_ref().map_or_else(
        || GameInfo {
            name: "Retrobox".into(),
            ..GameInfo::default()
        },
        |session| GameInfo::load(Path::new(&session.game_info), &session.rom),
    );
    let subtitle = [info.developer.as_str(), info.release_year.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");

    let items = vec![
        MenuItem {
            label: translator.text("menu-continue"),
            action: MenuAction::Continue,
        },
        MenuItem {
            label: translator.text("menu-quit"),
            action: MenuAction::Quit,
        },
    ];

    let time_key = if theme.clock12h {
        "clock-time-format-12h"
    } else {
        "clock-time-format"
    };
    let clock_formats = (
        config
            .clock
            .time_format
            .clone()
            .unwrap_or_else(|| translator.text(time_key)),
        config
            .clock
            .date_format
            .clone()
            .unwrap_or_else(|| translator.text("clock-date-format")),
    );

    let theme_fonts: Vec<PathBuf> = [&theme.title, &theme.text, &theme.text_small]
        .iter()
        .map(|style| style.font.path.clone())
        .collect();
    let fonts = Fonts::new(theme_fonts.iter().map(PathBuf::as_path), &language);

    let pads: Vec<Pad> = session
        .iter()
        .flat_map(|session| &session.controllers)
        .filter(|controller| !controller.device.is_empty())
        .filter_map(|controller| Pad::open(Path::new(&controller.device), theme.invert_buttons))
        .collect();

    let outcome = app::run(Content {
        config,
        theme,
        fonts,
        title: info.name,
        subtitle,
        items,
        clock_formats,
        locale: translator.locale(),
        pads,
        launched,
    })?;
    // the controllers were released with the overlay: the game gets them back

    if outcome == Outcome::Quit && !demo {
        quit_game();
    }
    Ok(())
}

/// Leave the game through retrobox-emukill, detached from the overlay.
fn quit_game() {
    let emukill = paths::retrobox_command("retrobox-emukill");
    log::info!("quitting the game: {}", emukill.display());
    if let Err(err) = Command::new(&emukill)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
    {
        log::error!("cannot run {}: {err}", emukill.display());
    }
}
