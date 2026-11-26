//! retrobox-overlay: Retrobox's in-game menu.
//!
//! `retrobox-inputd` (the input daemon, its own binary) runs for the whole
//! Retrobox session and opens it when the dedicated hotkey (Home/Guide) is
//! tapped, on Hotkey+A where the hotkey is Select, or on Ctrl+Shift+O or
//! Shift+F7; `--show` is the
//! overlay itself: a side menu styled like EmulationStation's over
//! the dimmed game, with a clock. Without a mode, it asks the daemon to open
//! or close the overlay, or shows it itself when no daemon runs; started
//! again while open, it closes the open one. Without a running game (in
//! EmulationStation) it shows the system's menu: the power button (sleep,
//! restart, turn off), the brightness and the volume, like the Switch's
//! quick settings. It neither opens nor stays open while a game is starting
//! or ending (see `retrobox_input::launch`). `--power` shows the power
//! options instead (sleep, restart, turn off): a popup in the middle of the
//! screen, the System page's *Power off…*, or a power button's binding.

mod achievements;
mod app;
mod config;
mod controls;
mod emulator;
mod game_pages;
mod i18n;
mod input;
mod manual;
mod playtime;
mod savestates;
mod session;
mod sound;
mod system;
mod task;
mod theme;
mod toast;
mod trace;
mod ui;
mod volume;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use nix::sys::signal::{SigSet, Signal};

use crate::app::{Content, Outcome, PowerOptions};
use crate::config::Config;
use crate::emulator::{EmuAction, Emulator, Request};
use crate::game_pages::GamePages;
use crate::i18n::Translator;
use crate::input::Pad;
use crate::savestates::{SaveStates, StateMode};
use crate::session::{GameInfo, Session};
use crate::system::performance::Performance;
use crate::system::power;
use crate::theme::MenuTheme;
use crate::ui::panel::{MenuAction, MenuItem};
use crate::ui::states::StatesMenu;
use crate::ui::text::Fonts;
use retrobox_input::instance::{self, Instance};
use retrobox_input::{held, launch, logging, paths};

/// How much lower the overlay's CPU priority is than the game's.
const NICENESS: i32 = 10;

const HELP: &str = "\
retrobox-overlay: Retrobox's in-game menu

Usage: retrobox-overlay [--show [--pad DEVICE]... | --osk | --demo] [--power]
                        [--version] [--help]
       retrobox-overlay --toast --title TEXT [--text TEXT] [--icon NAME]
                        [--sound select|back|scroll|none] [--seconds N]
       retrobox-overlay --toast break
       retrobox-overlay --volume up|down|mute
       retrobox-overlay --task --label NAME --log FILE [--check FILE]
                        [--background PNG] [--pad DEVICE]... [--lang CODE]
                        [--download | --reinstall | --update] -- COMMAND [ARGUMENT]...
       retrobox-overlay --held

Without a mode, asks the daemon to open or close the overlay, or shows the
overlay over the running game itself (see session.json in Retrobox's runtime
directory) when no daemon runs; started again while open, it closes it.

  --daemon       run retrobox-inputd, the input daemon that opens the
                 overlay (for launchers that still start it this way)
  --show         show the overlay (the daemon runs this)
  --pad DEVICE   a controller (evdev device) the overlay reads, besides the
                 game's own; repeatable
  --osk          show the on-screen keyboard over the running game, if its
                 emulator allows it (Retrobox's hotkeys run this); started
                 again while open, it closes
  --from-pad     opened from a controller (see take_keyboard in overlay.ini)
  --power        the power options (sleep, restart, turn off) instead of
                 the menu: a popup in the middle of the dimmed screen;
                 without --show, asks the daemon for it (as a power button's
                 binding would), or shows it itself when no daemon runs
  --demo         show it even without a running game (to try a theme)
  --toast        a notice at the top of the screen, on the menu's side, for
                 a few seconds (6, or --seconds), over whatever runs and
                 without taking its input; one waits for the one before it.
                 --icon: the name of one of the overlay's icons (trophy, coffee,
                 information...); the texts are shown as given. --toast break:
                 the break reminder (retrobox-inputd shows it after a while
                 playing: [break] in overlay.ini)
  --volume       a volume key: step the default output's volume by 5 %
                 (up to 100 %), or mute and unmute it; with [volume] toast
                 in overlay.ini, show it at the top of the screen, on the
                 menu's side, for 3 seconds after the last press (the
                 session's key bindings run this)
  --version      print the version
  --help         print this help

  --task         run an installer or a download full screen, over Retrobox's
                 background, showing its output (and passing the keyboard to
                 it): Retrobox runs it for the emulators and the content it
                 needs. It asks whether to install NAME first, unless
                 --download runs it at once (and closes when it works);
                 --reinstall offers to install what is installed again;
                 --update asks whether to update the system (the System
                 page runs it, with the system settings program's update).
                 Exit status: 0 done, 10 declined or cancelled, 11 failed,
                 12 no screen. FILE is the file the installation leaves
                 (checked after the command works), PNG the background
                 (default: Retrobox's emusetup/background.png).
  --held         exit with status 0 if a controller has L1 and R1 held down,
                 or a keyboard Ctrl and Shift, else 1 (how a game's start
                 asks for a reinstall)

The game's manual (a PDF scraped by EmulationStation) is shown with
poppler's pdftoppm and pdfinfo.

Settings: overlay.ini in /etc/retrobox and ~/.config/retrobox.
Log: stderr from a terminal, else ~/.local/state/retrobox/logs/overlay.log; RUST_LOG=debug for details.";

fn main() -> ExitCode {
    let launched = std::time::Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // (not the input daemon this mode becomes: the game's input goes through it)
    if !args.iter().any(|arg| arg == "--daemon") {
        lower_priority();
    }
    // before anything else looks at the arguments: those of a task's command
    // come after its own and are none of the overlay's
    match args.first().map(String::as_str) {
        Some("--held") => {
            return if held::reinstall_held() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        Some("--task") => return run_task(&args),
        Some("--toast") => return run_toast(&args[1..]),
        Some("--volume") => return run_volume(&args[1..]),
        _ => {}
    }
    if args.iter().any(|arg| arg == "--daemon") {
        return run_daemon();
    }
    let log = logging::init("overlay.log");

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
    let osk = args.iter().any(|arg| arg == "--osk");
    let from_pad = args.iter().any(|arg| arg == "--from-pad");
    let power = args.iter().any(|arg| arg == "--power");
    let pads: Vec<PathBuf> = args
        .windows(2)
        .filter(|pair| pair[0] == "--pad")
        .map(|pair| PathBuf::from(&pair[1]))
        .collect();
    let runtime_dir = paths::runtime_dir();

    let result = if !demo
        && !osk
        && !args.iter().any(|arg| arg == "--show")
        && let Some(pid) = instance::daemon_pid(&runtime_dir)
    {
        if power {
            log::info!("asking the daemon ({pid}) for the power options");
            instance::ask_power(pid);
        } else {
            log::info!("asking the daemon ({pid}) to open or close the overlay");
            instance::close_running(Some(pid));
        }
        Ok(())
    } else {
        trace::trace!(
            "start",
            "pid={} version={}",
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        );
        let shown = if osk {
            Shown::Keyboard
        } else if power {
            Shown::Power
        } else {
            Shown::Menu
        };
        let asked = Asked {
            shown,
            demo,
            from_pad,
        };
        let result = run(asked, &pads, launched, log.as_ref());
        trace::trace!("exit");
        trace::flush();
        result
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

/// The game first: the overlay, and what it starts (the sound player, the
/// tools it reads the system with), runs at a lower CPU priority. Opening
/// does a lot at once (fonts, the theme, pictures, the controllers, a few
/// programs), which on a small machine (a Nintendo Switch) would otherwise
/// take the cores from the game for a moment: its audio stutters. With cores
/// to spare, as when only the menu runs, nothing changes.
fn lower_priority() {
    if let Err(err) = rustix::process::nice(NICENESS) {
        log::warn!("cannot lower the overlay's priority: {err}");
    }
}

/// `--daemon`: become `retrobox-inputd`, the daemon that used to be this
/// mode (the launchers of before the split start it this way).
fn run_daemon() -> ExitCode {
    use std::os::unix::process::CommandExt as _;
    let daemon = paths::sibling_command("retrobox-inputd");
    let err = std::process::Command::new(&daemon).exec();
    eprintln!("retrobox-overlay: cannot run {}: {err}", daemon.display());
    ExitCode::FAILURE
}

/// `--task`: the task screen, in a process of its own (see `task`).
fn run_task(args: &[String]) -> ExitCode {
    let options = match task::Options::parse(args) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("retrobox-overlay --task: {err}");
            return ExitCode::from(2);
        }
    };
    let log = logging::init("overlay-task.log");
    block_usr1();
    ExitCode::from(match task_screen(options, log.as_ref()) {
        Ok(status) => status,
        Err(err) => {
            log::error!("{err:#}");
            task::EXIT_UNAVAILABLE
        }
    })
}

/// Show the task screen until it is done: the exit status.
fn task_screen(options: task::Options, log: Option<&std::fs::File>) -> Result<u8> {
    let runtime_dir = paths::runtime_dir();
    let _lock = match instance::acquire_named(&runtime_dir, task::LOCK_FILE)? {
        Instance::First(lock) => lock,
        Instance::Running(_) => anyhow::bail!("another task screen is running"),
    };
    if let Some(log) = log {
        log.set_len(0).ok();
    }
    let theme = MenuTheme::load_or_kept(
        &runtime_dir.join("es-menu-theme.json"),
        &paths::kept_menu_theme(),
    );
    let config = Config::load(&paths::config_dirs());
    let mut options = options;
    if options.background.is_none() {
        options.background = paths::layered_file(task::BACKGROUND);
    }
    let language = options
        .language
        .clone()
        .filter(|language| !language.is_empty())
        .unwrap_or_else(|| i18n::preferred_language(&theme.language));
    let translator = Translator::new(&language);
    let fonts = theme_fonts(&theme, &language);
    let new_pads = open_task_pads_later(&options.pads, theme.invert_buttons);
    // the status line's time, short, as the menu's small clock
    let small_clock = Some(small_clock_format(&config, &translator, theme.clock12h));
    let battery_percent = config.system.battery_percent;
    let (outcome, _, _) = app::run(Content {
        config,
        theme,
        fonts,
        title: String::new(),
        subtitle: String::new(),
        items: Vec::new(),
        buttons: Vec::new(),
        power: Vec::new(),
        power_options: PowerOptions::None,
        performance: None,
        clock_formats: (String::new(), String::new()),
        locale: translator.locale(),
        pads: Vec::new(),
        new_pads: Some(new_pads),
        emulator: Emulator::none(),
        states: None,
        manual: None,
        info: None,
        mappings: Vec::new(),
        cheevos: None,
        play: None,
        translator,
        launched: std::time::Instant::now(),
        small_clock,
        battery_percent,
        take_keyboard: true,
        osk: false,
        controls: None,
        screens: system::brightness::Screens::scan(),
        task: Some(options),
        toast: None,
        toast_feed: None,
    })?;
    Ok(match outcome {
        Outcome::Task(status) => status,
        _ => task::EXIT_DECLINED,
    })
}

/// The controllers of the task screen: those it was told, else every one.
fn open_task_pads_later(
    paths: &[PathBuf],
    swap_confirm: bool,
) -> calloop::channel::Channel<Vec<Pad>> {
    let (sender, channel) = calloop::channel::channel();
    let paths = paths.to_vec();
    std::thread::spawn(move || {
        let pads = input::open_pads(&paths, true, swap_confirm, false);
        sender.send(pads).ok();
    });
    channel
}

/// `--toast ...`: a toast over whatever runs, for a few seconds (see
/// `toast`), once the ones before it are gone.
fn run_toast(args: &[String]) -> ExitCode {
    let request = match toast::Request::parse(args) {
        Ok(request) => request,
        Err(err) => {
            eprintln!("retrobox-overlay --toast: {err:#}");
            return ExitCode::from(2);
        }
    };
    let _log = logging::init("overlay-toast.log");
    block_usr1();
    match toast_screen(request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

fn toast_screen(request: toast::Request) -> Result<()> {
    let runtime_dir = paths::runtime_dir();
    let _turn = toast::wait_turn(&runtime_dir)?;
    if instance::overlay_open(&runtime_dir) {
        log::info!("the overlay is open: no toast");
        return Ok(());
    }
    let session = Session::load(&runtime_dir);
    let config = Config::load(&paths::config_dirs());
    show_toast(&runtime_dir, session.as_ref(), config, None, |translator| {
        Ok(match request {
            toast::Request::Given(toast) => toast,
            toast::Request::Break => {
                let played = session
                    .as_ref()
                    .and_then(Session::running_for)
                    .context("no game is running: no break to suggest")?;
                toast::Toast::break_reminder(
                    translator.text("break-title"),
                    translator.text_strings(
                        "break-text",
                        &[("time", &playtime::duration_text(played, translator))],
                    ),
                )
            }
        })
    })
}

/// `--volume up|down|mute`: a volume key pressed (see `volume`).
fn run_volume(args: &[String]) -> ExitCode {
    let Some(key) = args.first().and_then(|name| volume::Key::parse(name)) else {
        eprintln!("retrobox-overlay --volume: up, down or mute");
        return ExitCode::from(2);
    };
    let log = logging::init("overlay-volume.log");
    block_usr1();
    match volume_key(key, log.as_ref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Change the volume and show it: on the toast on screen, or on a new one
/// (that this process then is, until it goes).
fn volume_key(key: volume::Key, log: Option<&std::fs::File>) -> Result<()> {
    let runtime_dir = paths::runtime_dir();
    std::fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("cannot create {}", runtime_dir.display()))?;
    let (level, turn) = volume::press(key, &runtime_dir)?;
    let config = Config::load(&paths::config_dirs());
    // the menu's slider shows it already
    if !config.volume_toast || instance::overlay_open(&runtime_dir) {
        return Ok(());
    }
    let volume::Turn::Toast(shown) = volume::hand_over(level, &runtime_dir)? else {
        return Ok(());
    };
    // the next presses may go on: they reach this toast
    drop(turn);
    if let Some(log) = log {
        log.set_len(0).ok();
    }
    let feed = shown
        .socket
        .try_clone()
        .context("cannot share the volume toast's socket")?;
    let session = Session::load(&runtime_dir);
    let result = show_toast(&runtime_dir, session.as_ref(), config, Some(feed), |_| {
        Ok(toast::Toast::volume(level))
    });
    shown.end();
    result
}

/// Show a toast, in the menu's theme and language (`toast_for` writes it).
fn show_toast(
    runtime_dir: &Path,
    session: Option<&Session>,
    config: Config,
    feed: Option<std::os::unix::net::UnixDatagram>,
    toast_for: impl FnOnce(&Translator) -> Result<toast::Toast>,
) -> Result<()> {
    let theme_path = session
        .and_then(Session::menu_theme_path)
        .unwrap_or_else(|| runtime_dir.join("es-menu-theme.json"));
    let theme = MenuTheme::load_or_kept(&theme_path, &paths::kept_menu_theme());
    let language = i18n::preferred_language(&theme.language);
    let translator = Translator::new(&language);
    let toast = toast_for(&translator)?;
    let fonts = theme_fonts(&theme, &language);
    app::run(Content {
        config,
        theme,
        fonts,
        title: String::new(),
        subtitle: String::new(),
        items: Vec::new(),
        buttons: Vec::new(),
        power: Vec::new(),
        power_options: PowerOptions::None,
        performance: None,
        clock_formats: (String::new(), String::new()),
        locale: translator.locale(),
        pads: Vec::new(),
        new_pads: None,
        emulator: Emulator::none(),
        states: None,
        manual: None,
        info: None,
        mappings: Vec::new(),
        cheevos: None,
        play: None,
        translator,
        launched: std::time::Instant::now(),
        small_clock: None,
        battery_percent: false,
        take_keyboard: false,
        osk: false,
        controls: None,
        screens: system::brightness::Screens::default(),
        task: None,
        toast: Some(toast),
        toast_feed: feed,
    })?;
    Ok(())
}

/// Keep SIGUSR1 pending until an event loop reads it (see `main`).
fn block_usr1() {
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGUSR1);
    if let Err(err) = mask.thread_set_mask() {
        log::warn!("cannot block SIGUSR1: {err}");
    }
}

/// What the overlay was started to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Menu,
    /// The on-screen keyboard (`--osk`).
    Keyboard,
    /// The power options' popup (`--power`).
    Power,
}

/// How the overlay was started.
#[derive(Debug, Clone, Copy)]
struct Asked {
    shown: Shown,
    /// Even without a running game (to try a theme).
    demo: bool,
    /// Opened from a controller (see `take_keyboard`).
    from_pad: bool,
}

fn run(
    asked: Asked,
    extra_pads: &[PathBuf],
    launched: std::time::Instant,
    log: Option<&std::fs::File>,
) -> Result<()> {
    let demo = asked.demo;
    let (osk, popup) = (asked.shown == Shown::Keyboard, asked.shown == Shown::Power);
    let runtime_dir = paths::runtime_dir();
    let Some(lock) = first_overlay(&runtime_dir, log, demo)? else {
        return Ok(());
    };

    // what takes a program to read is read from now on, while the rest loads
    let (sender, controls) = calloop::channel::channel();
    let screens = system::brightness::Screens::scan();
    // (the keyboard and the power options have no sliders: none of it)
    if !osk && !popup {
        read_controls(&screens, sender.clone());
    }

    let session = Session::load(&runtime_dir);
    // the power options are there even where a game turns the menu off
    if !popup && !overlay_allowed(demo, osk, session.as_ref()) {
        return Ok(());
    }
    // the overlay comes up alone: no toast over it, or taking its focus away
    toast::close_shown(&runtime_dir);

    let (theme, config, language, translator) = look(&runtime_dir, session.as_ref());

    let in_game = session.is_some() || demo;
    let (info, subtitle) = heading(session.as_ref(), (osk, popup), &translator);

    let (emulator, saves) = emulator_for(session.as_ref().filter(|_| !popup), demo);
    let states = saves
        .clone()
        .map(|saves| StatesMenu::new(saves, &translator));
    let pages = session
        .as_ref()
        .filter(|_| !osk && !popup)
        .map(|session| GamePages::new(session, &info, &translator))
        .unwrap_or_default();
    let autosave = saves.as_ref().is_some_and(SaveStates::autoload);
    let Some(Entries {
        items,
        buttons,
        power,
        power_options,
        performance,
    }) = entries(
        &translator,
        &config,
        (osk, popup),
        in_game.then_some((&emulator, &pages)),
        autosave,
        sender,
    )
    else {
        return Ok(());
    };

    let (clock_formats, small_clock) = clock_formats(&config, &translator, theme.clock12h);
    // the keyboard types on the game's window: it must keep the focus
    let (battery_percent, take_keyboard) = (
        config.system.battery_percent,
        !osk && config.take_keyboard.take(asked.from_pad),
    );

    let new_pads = open_pads_later(
        session.as_ref(),
        extra_pads,
        theme.invert_buttons,
        osk,
        launched,
    );
    let fonts = theme_fonts(&theme, &language);

    let (outcome, emulator, setters) = app::run(Content {
        config,
        theme,
        fonts,
        title: info.name,
        subtitle,
        items,
        buttons,
        power,
        power_options,
        performance,
        clock_formats,
        locale: translator.locale(),
        pads: Vec::new(),
        new_pads: Some(new_pads),
        emulator,
        states,
        manual: pages.manual,
        info: pages.info,
        mappings: pages.controls,
        cheevos: pages.cheevos,
        play: pages.play,
        translator,
        launched,
        small_clock,
        battery_percent,
        take_keyboard,
        osk,
        controls: Some(controls),
        screens,
        task: None,
        toast: None,
        toast_feed: None,
    })?;
    // The overlay is gone and the controllers are the game's again; the
    // keyboard focus is on its way back to the game. Another overlay may
    // open from now on: what is left only talks to the emulator.
    block_usr1();
    drop(lock);
    // the last slider change first
    drop(setters);
    after(outcome, &emulator, saves.as_ref().filter(|_| !demo), demo);
    Ok(())
}

/// What the menu asked for, once the overlay is gone (the demo quits
/// nothing).
fn after(outcome: Outcome, emulator: &Emulator, saves: Option<&SaveStates>, demo: bool) {
    match outcome {
        Outcome::Power(action) => power::perform(action),
        Outcome::PowerOptions => show_power_options(&paths::runtime_dir()),
        Outcome::Quit | Outcome::SaveAndQuit | Outcome::QuitUnsaved if demo => {}
        Outcome::Quit => quit_game(),
        // the automatic state it would load next time is gone
        Outcome::QuitUnsaved => {
            if let Some(saves) = saves {
                saves.forget_auto();
            }
            quit_game();
        }
        Outcome::SaveAndQuit => {
            if let Some(saves) = saves {
                let before = saves.auto_saved();
                emulator.back_to_game(Request::SaveAuto);
                if !saves.wait_for_auto_save(before) {
                    log::warn!("the state was not saved in time: quitting anyway");
                }
            }
            quit_game();
        }
        Outcome::Overlay => open_overlay(),
        Outcome::Request(request) => emulator.back_to_game(request),
        Outcome::Back | Outcome::Task(_) => {}
    }
}

/// This overlay's lock, when it is the only one and it may open: another
/// one running is asked to close instead, and nothing opens while a game is
/// starting or ending (but `demo`).
fn first_overlay(
    runtime_dir: &Path,
    log: Option<&std::fs::File>,
    demo: bool,
) -> Result<Option<nix::fcntl::Flock<std::fs::File>>> {
    let lock = match instance::acquire(runtime_dir)? {
        Instance::First(lock) => lock,
        Instance::Running(pid) => {
            instance::close_running(pid);
            return Ok(None);
        }
    };
    if !demo && launch::busy() {
        log::info!("a game is starting or ending: not now");
        return Ok(None);
    }
    // the log is this run's from here: a second start (which only closes
    // this one) leaves it alone
    if let Some(log) = log {
        log.set_len(0).ok();
    }
    Ok(Some(lock))
}

/// Start reading the sliders' values (the volume, the primary screen).
fn read_controls(
    screens: &system::brightness::Screens,
    sender: calloop::channel::Sender<system::Msg>,
) {
    if screens.ambiguous() && system::brightness::primary_known() {
        system::read_primary(sender.clone());
    }
    system::read_volume(sender);
}

/// Whether the game lets this overlay open (see `osk_allowed`).
fn overlay_allowed(demo: bool, osk: bool, session: Option<&Session>) -> bool {
    if session.is_some_and(|session| !session.overlay) && !demo {
        log::info!("the overlay is turned off for this game");
        return false;
    }
    osk_allowed(osk, demo, session)
}

/// What the game's info file says, else just "Retrobox".
fn game_info(session: Option<&Session>) -> GameInfo {
    session.map_or_else(
        || GameInfo {
            name: "Retrobox".into(),
            ..GameInfo::default()
        },
        |session| GameInfo::load(Path::new(&session.game_info), &session.rom),
    )
}

/// Find and grab the controllers (and the keyboards, for the on-screen
/// keyboard) on a thread of their own, which takes longer than showing the
/// overlay: they are sent when ready.
fn open_pads_later(
    session: Option<&Session>,
    extra_pads: &[PathBuf],
    swap_confirm: bool,
    osk: bool,
    launched: std::time::Instant,
) -> calloop::channel::Channel<Vec<Pad>> {
    let (sender, channel) = calloop::channel::channel();
    let session = session.cloned();
    let extra_pads = extra_pads.to_vec();
    std::thread::spawn(move || {
        let pads = open_pads(session.as_ref(), &extra_pads, swap_confirm, osk);
        log::info!(
            "controllers opened {} ms after start",
            launched.elapsed().as_millis()
        );
        sender.send(pads).ok();
    });
    channel
}

/// Start the overlay again, as the pad's hotkey does (the on-screen keyboard
/// has just left, and its lock with it).
fn open_overlay() {
    if let Ok(exe) = std::env::current_exe() {
        emulator::spawn(&[exe.to_string_lossy().into_owned(), "--from-pad".into()]);
    }
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
    let time_key = if clock12h {
        "clock-time-format-12h"
    } else {
        "clock-time-format"
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
    let small = (!clock.big).then(|| small_clock_format(config, translator, clock12h));
    (big, small)
}

/// The small clock's format (the status line's time).
fn small_clock_format(config: &Config, translator: &Translator, clock12h: bool) -> String {
    let key = if clock12h {
        "clock-small-format-12h"
    } else {
        "clock-small-format"
    };
    config
        .clock
        .small_format
        .clone()
        .unwrap_or_else(|| translator.text(key))
}

/// What the System page offers besides the power options (nothing over
/// the on-screen keyboard or in the power options' popup): updating the
/// system (outside games, where the machine's system settings program can),
/// and a Nintendo Switch's performance profiles (they tell the menu through
/// `sender` when they change).
fn power_page(
    config: &Config,
    in_game: bool,
    none: bool,
    sender: calloop::channel::Sender<system::Msg>,
) -> (Vec<power::PowerAction>, Option<Performance>) {
    if none {
        return (Vec::new(), None);
    }
    let mut actions = Vec::new();
    if !in_game && system::update::available() {
        actions.push(power::PowerAction::Update);
    }
    (
        actions,
        Performance::find(config.system.performance, sender),
    )
}

/// The menu's entries and round buttons: a game's (with its emulator and
/// whether it has a manual), or the system's (only the System button; the
/// sliders come as they are found). `autosave`: whether quitting saves
/// the automatic state; `power`: whether the System page offers anything.
fn menus(
    translator: &Translator,
    game: Option<(&Emulator, &GamePages)>,
    autosave: bool,
    power: bool,
) -> (Vec<MenuItem>, Vec<MenuItem>) {
    let mut buttons = Vec::new();
    let items = if let Some((emulator, pages)) = game {
        buttons = menu_buttons(translator, emulator, autosave);
        menu_items(translator, emulator, pages)
    } else {
        log::info!("no game running: the system's menu");
        Vec::new()
    };
    if power {
        buttons.push(MenuItem::new(
            translator.text("menu-system"),
            MenuAction::Power,
        ));
    }
    (items, buttons)
}

/// The theme, the settings, the language and its translations: those of
/// the running game's EmulationStation, else the last ones kept.
fn look(runtime_dir: &Path, session: Option<&Session>) -> (MenuTheme, Config, String, Translator) {
    let theme_path = session
        .and_then(Session::menu_theme_path)
        .unwrap_or_else(|| runtime_dir.join("es-menu-theme.json"));
    let theme = MenuTheme::load_or_kept(&theme_path, &paths::kept_menu_theme());
    let config = Config::load(&paths::config_dirs());
    let language = i18n::preferred_language(&theme.language);
    let translator = Translator::new(&language);
    (theme, config, language, translator)
}

/// What the overlay shows besides the sliders (they come as they are
/// found).
struct Entries {
    items: Vec<MenuItem>,
    buttons: Vec<MenuItem>,
    /// The System page's own actions (the update).
    power: Vec<power::PowerAction>,
    power_options: PowerOptions,
    performance: Option<Performance>,
}

/// The entries: the power options in their popup (`shown.1`; none when
/// nothing can do them), else the menu's (none of the System page over the
/// on-screen keyboard, `shown.0`), a game's when `game`.
fn entries(
    translator: &Translator,
    config: &Config,
    shown: (bool, bool),
    game: Option<(&Emulator, &GamePages)>,
    autosave: bool,
    sender: calloop::channel::Sender<system::Msg>,
) -> Option<Entries> {
    let (osk, popup) = shown;
    let options = power::actions(config.system.standby);
    if popup {
        return (!options.is_empty()).then(|| Entries {
            items: power_items(translator, &options),
            buttons: Vec::new(),
            power: Vec::new(),
            power_options: PowerOptions::Popup,
            performance: None,
        });
    }
    let (power, performance) = power_page(config, game.is_some(), osk, sender);
    let power_options = if options.is_empty() || osk {
        PowerOptions::None
    } else {
        PowerOptions::Row
    };
    let system = power_options != PowerOptions::None || !power.is_empty() || performance.is_some();
    let (items, buttons) = menus(translator, game, autosave, system);
    Some(Entries {
        items,
        buttons,
        power,
        power_options,
        performance,
    })
}

/// The power options' entries.
fn power_items(translator: &Translator, options: &[power::PowerAction]) -> Vec<MenuItem> {
    options
        .iter()
        .map(|&action| {
            MenuItem::new(
                translator.text(action.label_key()),
                MenuAction::PowerOption(action),
            )
        })
        .collect()
}

/// The overlay is gone and the power options were asked for (the System
/// page's *Power off…*): the daemon shows them (`--power`), or, without
/// one, a new process of this program.
fn show_power_options(runtime_dir: &Path) {
    if let Some(pid) = instance::daemon_pid(runtime_dir) {
        instance::ask_power(pid);
        return;
    }
    let exe = paths::sibling_command("retrobox-overlay");
    emulator::spawn(&[
        exe.to_string_lossy().into_owned(),
        "--show".into(),
        "--power".into(),
    ]);
}

/// The header: the game's name and the line under it, nothing over the
/// on-screen keyboard (`shown.0`), the power options' title in their popup
/// (`shown.1`).
fn heading(
    session: Option<&Session>,
    shown: (bool, bool),
    translator: &Translator,
) -> (GameInfo, String) {
    let (osk, popup) = shown;
    let mut info = game_info(session.filter(|_| !osk && !popup));
    if popup {
        info.name = translator.text("power-title");
    }
    let subtitle = subtitle_of(&info);
    (info, subtitle)
}

/// The line under the game's name: its developer and year.
fn subtitle_of(info: &GameInfo) -> String {
    [info.developer.as_str(), info.release_year()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Whether the on-screen keyboard may open (when it is what was asked for):
/// over a running game whose emulator allows it.
fn osk_allowed(osk: bool, demo: bool, session: Option<&Session>) -> bool {
    let allowed = !osk || demo || session.is_some_and(|session| session.osk);
    if !allowed {
        log::info!("the on-screen keyboard is not available for this game");
    }
    allowed
}

/// The controllers of the game and the ones the daemon passed (see
/// `input::open_pads`), for the on-screen keyboard when `keyboard`.
fn open_pads(
    session: Option<&Session>,
    extra: &[PathBuf],
    swap_confirm: bool,
    keyboard: bool,
) -> Vec<Pad> {
    let paths: Vec<PathBuf> = session
        .iter()
        .flat_map(|session| &session.controllers)
        .filter(|controller| !controller.device.is_empty())
        .map(|controller| PathBuf::from(&controller.device))
        .chain(extra.iter().cloned())
        .collect();
    input::open_pads(&paths, false, swap_confirm, keyboard)
}

/// The running game's emulator and, when its states go by slot, where they
/// are.
fn emulator_for(session: Option<&Session>, demo: bool) -> (Emulator, Option<SaveStates>) {
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
        .map(SaveStates::new);
    (emulator, states)
}

/// The save states, the game's pages (manual, information, controls) and
/// what the emulator can do.
fn menu_items(translator: &Translator, emulator: &Emulator, pages: &GamePages) -> Vec<MenuItem> {
    let item = |key: &str, action| MenuItem::new(translator.text(key), action);
    let mut items = Vec::new();
    if emulator.has_slots() {
        items.push(item("menu-load-state", MenuAction::States(StateMode::Load)));
        items.push(item("menu-save-state", MenuAction::States(StateMode::Save)));
    }
    items.extend(pages.items(translator));
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

/// The round buttons under a game's entries: continue, quit (the System
/// button comes last). Where the game loads the automatic state on start
/// (`autosave`), quitting saves it first, and Quit held quits without
/// saving.
fn menu_buttons(translator: &Translator, emulator: &Emulator, autosave: bool) -> Vec<MenuItem> {
    let item = |key: &str, action| MenuItem::new(translator.text(key), action);
    let mut buttons = vec![item("menu-continue", MenuAction::Continue)];
    let autosave = autosave && emulator.has_slots();
    let label = if autosave {
        "menu-save-quit"
    } else {
        "menu-quit"
    };
    buttons.push(item(label, MenuAction::Quit { autosave }));
    buttons
}

/// Leave the game through retrobox-emukill, detached from the overlay.
fn quit_game() {
    let emukill = paths::retrobox_command("retrobox-emukill");
    log::info!("quitting the game: {}", emukill.display());
    emulator::spawn(&[emukill.to_string_lossy().into_owned()]);
}
