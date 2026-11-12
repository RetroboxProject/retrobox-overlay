//! retrobox-overlay: Retrobox's in-game menu.
//!
//! `--daemon` runs for the whole Retrobox session and opens it when the
//! hotkey (or Select+A) is held, or on Shift+F7 (see `daemon.rs`); `--show`
//! is the overlay itself: a side menu styled like EmulationStation's over
//! the dimmed game, with a clock. Without a mode, it asks the daemon to open
//! or close the overlay, or shows it itself when no daemon runs; started
//! again while open, it closes the open one. Without a running game (in
//! EmulationStation) it shows the system's menu: sleep, the brightness and
//! the volume, like the Switch's quick settings.

mod app;
mod config;
mod daemon;
mod emulator;
mod i18n;
mod input;
mod instance;
mod manual;
mod paths;
mod savestates;
mod session;
mod system;
mod theme;
mod ui;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use nix::sys::signal::{SigSet, Signal};

use crate::app::{Content, Outcome};
use crate::config::Config;
use crate::emulator::{EmuAction, Emulator};
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

Usage: retrobox-overlay [--daemon | --show [--pad DEVICE]... | --demo]
                        [--version] [--help]

Without a mode, asks the daemon to open or close the overlay, or shows the
overlay over the running game itself (see session.json in Retrobox's runtime
directory) when no daemon runs; started again while open, it closes it.

  --daemon       open the overlay when a controller's hotkey (Select+A
                 without one) is held, or on Shift+F7, until stopped
                 (Retrobox runs it)
  --show         show the overlay (the daemon runs this)
  --pad DEVICE   a controller (evdev device) the overlay reads, besides the
                 game's own; repeatable
  --from-pad     opened from a controller (see take_keyboard in overlay.ini)
  --demo         show it even without a running game (to try a theme)
  --version      print the version
  --help         print this help

The game's manual (a PDF scraped by EmulationStation) is shown with
poppler's pdftoppm and pdfinfo.

Settings: overlay.ini in /etc/retrobox and ~/.config/retrobox.
Log: stderr from a terminal, else ~/.local/state/retrobox/logs/overlay.log; RUST_LOG=debug for details.";

fn main() -> ExitCode {
    let launched = std::time::Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let daemon = args.iter().any(|arg| arg == "--daemon");
    let log = init_log(if daemon {
        "overlay-daemon.log"
    } else {
        "overlay.log"
    });

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("retrobox-overlay {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    // A second start may signal this one at any moment: SIGUSR1 waits
    // (blocked) until the event loop reads it, instead of killing the process.
    block_usr1();
    let demo = args.iter().any(|arg| arg == "--demo");
    let from_pad = args.iter().any(|arg| arg == "--from-pad");
    let pads: Vec<PathBuf> = args
        .windows(2)
        .filter(|pair| pair[0] == "--pad")
        .map(|pair| PathBuf::from(&pair[1]))
        .collect();
    let runtime_dir = paths::runtime_dir();

    let result = if daemon {
        let config = Config::load(&paths::config_dirs());
        daemon::run(&runtime_dir, config.hold)
    } else if !demo
        && !args.iter().any(|arg| arg == "--show")
        && let Some(pid) = instance::daemon_pid(&runtime_dir)
    {
        log::info!("asking the daemon ({pid}) to open or close the overlay");
        instance::close_running(Some(pid));
        Ok(())
    } else {
        run(demo, from_pad, &pads, launched, log.as_ref())
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Keep SIGUSR1 pending until an event loop reads it (see `main`).
fn block_usr1() {
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGUSR1);
    if let Err(err) = mask.thread_set_mask() {
        log::warn!("cannot block SIGUSR1: {err}");
    }
}

fn run(
    demo: bool,
    from_pad: bool,
    extra_pads: &[PathBuf],
    launched: std::time::Instant,
    log: Option<&std::fs::File>,
) -> Result<()> {
    let runtime_dir = paths::runtime_dir();
    let lock = match instance::acquire(&runtime_dir)? {
        Instance::First(lock) => {
            // the log is this run's from here: a second start (which only
            // closes this one) leaves it alone
            if let Some(log) = log {
                log.set_len(0).ok();
            }
            lock
        }
        Instance::Running(pid) => {
            instance::close_running(pid);
            return Ok(());
        }
    };

    // what takes a program to read is read from now on, while the rest loads
    let (sender, controls) = calloop::channel::channel();
    let screens = system::brightness::Screens::scan();
    if screens.ambiguous() && system::brightness::primary_known() {
        system::read_primary(sender.clone());
    }
    system::read_volume(sender);

    let session = Session::load(&runtime_dir);
    if session.as_ref().is_some_and(|session| !session.overlay) && !demo {
        log::info!("the overlay is turned off for this game");
        return Ok(());
    }

    let theme_path = session
        .as_ref()
        .and_then(Session::menu_theme_path)
        .unwrap_or_else(|| runtime_dir.join("es-menu-theme.json"));
    let theme = MenuTheme::load_or_kept(&theme_path, &paths::kept_menu_theme());
    let config = Config::load(&paths::config_dirs());
    let language = i18n::preferred_language(&theme.language);
    let translator = Translator::new(&language);

    let in_game = session.is_some() || demo;
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

    let (emulator, states) = emulator_for(session.as_ref(), demo, &translator);
    let manual = manual_path(
        &info.manual,
        session.as_ref().map(|session| session.rom.as_str()),
    )
    .and_then(|path| Manual::open(&path));
    let (items, buttons) = menus(
        &translator,
        &config,
        in_game.then_some((&emulator, manual.is_some())),
    );

    let (clock_formats, small_clock) = clock_formats(&config, &translator, theme.clock12h);
    let (battery_percent, take_keyboard) = (
        config.system.battery_percent,
        config.take_keyboard.take(from_pad),
    );

    let fonts = theme_fonts(&theme, &language);

    let pads = open_pads(session.as_ref(), extra_pads, theme.invert_buttons);

    let (outcome, emulator, setters) = app::run(Content {
        config,
        theme,
        fonts,
        title: info.name,
        subtitle,
        items,
        buttons,
        clock_formats,
        locale: translator.locale(),
        pads,
        emulator,
        states,
        manual,
        translator,
        launched,
        small_clock,
        battery_percent,
        take_keyboard,
        controls: Some(controls),
        screens,
    })?;
    // The overlay is gone and the controllers are the game's again; the
    // keyboard focus is on its way back to the game. Another overlay may
    // open from now on: what is left only talks to the emulator.
    block_usr1();
    drop(lock);
    match outcome {
        Outcome::Quit if !demo => quit_game(),
        Outcome::Quit | Outcome::Back => {}
        Outcome::Request(request) => emulator.back_to_game(request),
        Outcome::Standby => {
            // the last slider change first
            drop(setters);
            system::standby();
            return Ok(());
        }
    }
    // the last slider change is applied before leaving
    drop(setters);
    Ok(())
}

/// The theme's menu fonts, then the fallbacks for the language.
fn theme_fonts(theme: &MenuTheme, language: &str) -> Fonts {
    let paths: Vec<PathBuf> = [&theme.title, &theme.text, &theme.text_small]
        .iter()
        .map(|style| style.font.path.clone())
        .collect();
    Fonts::new(paths.iter().map(PathBuf::as_path), language)
}

/// The big clock's formats (time, date) and the small one's, when the clock
/// is the small one: overlay.ini's, else the language's.
fn clock_formats(
    config: &Config,
    translator: &Translator,
    clock12h: bool,
) -> ((String, String), Option<String>) {
    let clock = &config.clock;
    let (time_key, small_key) = if clock12h {
        ("clock-time-format-12h", "clock-small-format-12h")
    } else {
        ("clock-time-format", "clock-small-format")
    };
    let big = (
        clock
            .time_format
            .clone()
            .unwrap_or_else(|| translator.text(time_key)),
        clock
            .date_format
            .clone()
            .unwrap_or_else(|| translator.text("clock-date-format")),
    );
    let small = (!clock.big).then(|| {
        clock
            .small_format
            .clone()
            .unwrap_or_else(|| translator.text(small_key))
    });
    (big, small)
}

/// Outside games: sleep (when allowed); the sliders come as they are found.
/// The menu's entries and round buttons: a game's (with its emulator and
/// whether it has a manual), or the system's.
fn menus(
    translator: &Translator,
    config: &Config,
    game: Option<(&Emulator, bool)>,
) -> (Vec<MenuItem>, Vec<MenuItem>) {
    if let Some((emulator, manual)) = game {
        (
            menu_items(translator, emulator, manual),
            menu_buttons(translator, emulator),
        )
    } else {
        log::info!("no game running: the system's menu");
        (system_items(translator, config), Vec::new())
    }
}

fn system_items(translator: &Translator, config: &Config) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if config.system.standby {
        items.push(MenuItem::new(
            translator.text("menu-standby"),
            MenuAction::Standby,
        ));
    }
    items
}

/// The game's controllers and the ones the daemon passed (outside a game,
/// every controller), each once.
fn open_pads(session: Option<&Session>, extra: &[PathBuf], swap_confirm: bool) -> Vec<Pad> {
    let mut seen = std::collections::HashSet::new();
    session
        .iter()
        .flat_map(|session| &session.controllers)
        .filter(|controller| !controller.device.is_empty())
        .map(|controller| PathBuf::from(&controller.device))
        .chain(extra.iter().cloned())
        .filter(|path| seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone())))
        .filter_map(|path| Pad::open(&path, swap_confirm))
        .collect()
}

/// Log to stderr from a terminal, else to the log file `name` (the
/// overlay's is emptied once this run is the overlay, see `run`); the file,
/// if it is the log.
fn init_log(name: &str) -> Option<std::fs::File> {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if std::io::stderr().is_terminal() {
        builder.format_timestamp(None);
    } else {
        let path = paths::log_file(name);
        let file = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
            });
        if let Ok(file) = file
            && let Ok(target) = file.try_clone()
        {
            builder
                .format_timestamp_millis()
                .target(env_logger::Target::Pipe(Box::new(target)));
            builder.init();
            return Some(file);
        }
    }
    builder.init();
    None
}

/// The running game's emulator and, when its states go by slot, the pages.
fn emulator_for(
    session: Option<&Session>,
    demo: bool,
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
    let emulator = Emulator::new(session);
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
    let item = |key: &str, action| MenuItem::new(translator.text(key), action);
    let mut items = Vec::new();
    if emulator.has_slots() {
        items.push(item("menu-load-state", MenuAction::States(StateMode::Load)));
        items.push(item("menu-save-state", MenuAction::States(StateMode::Save)));
    }
    if manual {
        items.push(item("menu-manual", MenuAction::Manual));
    }
    items.extend(
        emulator
            .actions()
            .iter()
            .copied()
            .filter(|action| *action != EmuAction::Screenshot)
            .map(|action| item(action.label_key(), MenuAction::Emulator(action))),
    );
    items
}

/// The round buttons under a game's entries: continue, a screenshot (when
/// the emulator takes them), quit.
fn menu_buttons(translator: &Translator, emulator: &Emulator) -> Vec<MenuItem> {
    let item = |key: &str, action| MenuItem::new(translator.text(key), action);
    let mut buttons = vec![item("menu-continue", MenuAction::Continue)];
    if emulator.actions().contains(&EmuAction::Screenshot) {
        let action = EmuAction::Screenshot;
        buttons.push(item(action.label_key(), MenuAction::Emulator(action)));
    }
    buttons.push(item("menu-quit", MenuAction::Quit));
    buttons
}

/// Leave the game through retrobox-emukill, detached from the overlay.
fn quit_game() {
    let emukill = paths::retrobox_command("retrobox-emukill");
    log::info!("quitting the game: {}", emukill.display());
    emulator::spawn(&[emukill.to_string_lossy().into_owned()]);
}
