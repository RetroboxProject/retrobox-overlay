//! retrobox-overlay: Retrobox's in-game menu.
//!
//! Started by the hotkey (Home pressed alone, Hotkey+A) or Shift+F7 through
//! gamepadly, it shows a side menu styled like EmulationStation's over the
//! dimmed game, with a clock. Started again while open, it closes the open
//! one. Without a running game it does nothing (unless `--demo`).

mod app;
mod config;
mod emulator;
mod i18n;
mod input;
mod instance;
mod manual;
mod paths;
mod savestates;
mod session;
mod theme;
mod ui;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::app::{Content, Outcome};
use crate::config::Config;
use crate::emulator::Emulator;
use crate::i18n::Translator;
use crate::input::Pad;
use crate::instance::Instance;
use crate::manual::Manual;
use crate::savestates::{SaveStates, StateMode};
use crate::session::{GameInfo, Session};
use crate::theme::MenuTheme;
use crate::ui::panel::{MenuAction, MenuItem};
use crate::ui::states::StatesMenu;
use crate::ui::text::Fonts;

const HELP: &str = "\
retrobox-overlay: Retrobox's in-game menu

Usage: retrobox-overlay [--demo] [--version] [--help]

Shows the overlay over the running game (see session.json in Retrobox's
runtime directory), or closes it when it is already open.

  --demo     show it even without a running game (to try a theme)
  --version  print the version
  --help     print this help

The game's manual (a PDF scraped by EmulationStation) is shown with
poppler's pdftoppm and pdfinfo.

Settings: overlay.ini in /etc/retrobox and ~/.config/retrobox.
Log: stderr from a terminal, else ~/.local/state/retrobox/logs/overlay.log; RUST_LOG=debug for details.";

fn main() -> ExitCode {
    let launched = std::time::Instant::now();
    init_log();

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

    let (emulator, states) = emulator_for(session.as_ref(), demo, &config, &translator);
    let manual = manual_path(
        &info.manual,
        session.as_ref().map(|session| session.rom.as_str()),
    )
    .and_then(|path| Manual::open(&path));
    let items = menu_items(&translator, &emulator, manual.is_some());

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

    // before the overlay shows itself: the game still has the keyboard focus
    emulator.pause();
    let (outcome, emulator) = app::run(Content {
        config,
        theme,
        fonts,
        title: info.name,
        subtitle,
        items,
        clock_formats,
        locale: translator.locale(),
        pads,
        emulator,
        states,
        manual,
        translator,
        launched,
    })?;
    // The overlay is gone and the controllers are the game's again; the
    // keyboard focus is on its way back to the game.
    match outcome {
        Outcome::Quit if !demo => quit_game(),
        Outcome::Quit => {}
        Outcome::Dismiss => log::info!("closed without a choice: the game stays paused"),
        Outcome::Continue => emulator.back_to_game(None),
        Outcome::Request(request) => emulator.back_to_game(Some(request)),
    }
    Ok(())
}

/// Log to stderr from a terminal, else to the log file (replaced on every run).
fn init_log() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if std::io::stderr().is_terminal() {
        builder.format_timestamp(None);
    } else {
        let path = paths::log_file();
        let file = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::File::create(&path));
        if let Ok(file) = file {
            builder
                .format_timestamp_millis()
                .target(env_logger::Target::Pipe(Box::new(file)));
        }
    }
    builder.init();
}

/// The running game's emulator and, when its states go by slot, the pages.
fn emulator_for(
    session: Option<&Session>,
    demo: bool,
    config: &Config,
    translator: &Translator,
) -> (Emulator, Option<StatesMenu>) {
    let Some(session) = session.filter(|_| !demo) else {
        return (Emulator::none(), None);
    };
    log::info!(
        "game: {} on {} ({})",
        session.rom,
        session.system,
        session.emulator
    );
    let emulator = Emulator::new(session, config.pause_game);
    let states = session
        .savestates
        .as_ref()
        .filter(|_| emulator.has_slots())
        .map(|info| StatesMenu::new(SaveStates::new(info), translator));
    (emulator, states)
}

/// Where the game's manual is: ES writes full paths, but a gamelist may
/// still hold one relative to the ROM's folder (`./manuals/...`).
fn manual_path(manual: &str, rom: Option<&str>) -> Option<PathBuf> {
    if manual.is_empty() {
        return None;
    }
    let path = Path::new(manual);
    if path.is_absolute() {
        return Some(path.to_owned());
    }
    let folder = Path::new(rom?).parent()?;
    Some(folder.join(path.strip_prefix("./").unwrap_or(path)))
}

/// Continue, the save states, the manual, what the emulator can do, and
/// leaving the game.
fn menu_items(translator: &Translator, emulator: &Emulator, manual: bool) -> Vec<MenuItem> {
    let item = |key: &str, action| MenuItem {
        label: translator.text(key),
        action,
        enabled: true,
    };
    let mut items = vec![item("menu-continue", MenuAction::Continue)];
    if emulator.has_slots() {
        items.push(item("menu-save-state", MenuAction::States(StateMode::Save)));
        items.push(item("menu-load-state", MenuAction::States(StateMode::Load)));
    }
    if manual {
        items.push(item("menu-manual", MenuAction::Manual));
    }
    items.extend(
        emulator
            .actions()
            .iter()
            .copied()
            .map(|action| item(action.label_key(), MenuAction::Emulator(action))),
    );
    items.push(item("menu-quit", MenuAction::Quit));
    items
}

/// Leave the game through retrobox-emukill, detached from the overlay.
fn quit_game() {
    let emukill = paths::retrobox_command("retrobox-emukill");
    log::info!("quitting the game: {}", emukill.display());
    emulator::spawn(&[emukill.to_string_lossy().into_owned()]);
}
