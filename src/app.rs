//! The overlay on screen: pieces placed like layer-shell surfaces, on
//! Wayland or X11 (see `display.rs`).
//!
//! * the dim: one translucent black buffer over the whole output;
//! * the panel: the side menu, slid in from its edge by animating its margin
//!   (its buffer is drawn once, not on every frame); it takes the keyboard,
//!   once the game is paused (the pause key has to reach the game first, see
//!   emulator.rs);
//! * the clock, in a corner on the other side, redrawn once a second;
//! * the manual, when opened: a page over the whole screen (see
//!   `manual_view.rs`);
//! * the help prompts, at the bottom like EmulationStation's, inside the dim.
//!
//! Where the display can (Wayland's `wp_alpha_modifier`, KWin 6.1+ and labwc
//! 0.9+; X11's window opacity, with a compositing manager) they fade in as
//! well, by the compositor: nothing is drawn again for the fade. (A
//! single-pixel buffer stretched by `wp_viewporter` would make the dim free,
//! but software compositors filter it to nothing; a full buffer works
//! everywhere.)
//!
//! The dim comes first: its size is the output's, which the rest is sized
//! from.

use std::os::fd::AsFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use chrono::{Local, Timelike};
use smithay_client_toolkit::compositor::CompositorHandler;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::signals::{Signal, Signals};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1;
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_v1::WpAlphaModifierV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{self, NotifyDetail, NotifyMode};

use crate::config::{Config, Side, VerticalPosition};
use crate::emulator::{EmuAction, Emulator, Request};
use crate::i18n::Translator;
use crate::input::{self, Input, Nav, Pad};
use crate::manual::Manual;
use crate::savestates::StateMode;
use crate::theme::MenuTheme;
use crate::ui::clock::Clock;
use crate::ui::help::{Help, Prompt};
use crate::ui::panel::{MenuAction, MenuItem, Panel};
use crate::ui::states::StatesMenu;
use crate::ui::text::Fonts;

mod display;
mod manual_view;

use display::{Display, Piece, Spec, Sub, SurfaceId, X11};
use manual_view::ManualState;

const FRAME: Duration = Duration::from_millis(16);
/// How long a controller direction is held before it repeats, and how often
/// it does then.
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_INTERVAL: Duration = Duration::from_millis(70);
/// Space between the clock and the screen edge when it is at the top or bottom.
const CLOCK_MARGIN: f32 = crate::ui::clock::MARGIN;

/// How the overlay was left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// "Continue", or the shortcut that opened the overlay pressed again
    /// (Home, Shift+F7): back to the game, running.
    Continue,
    /// An emulator action, then back to the game, running.
    Request(Request),
    /// Closed without a choice (back, a click outside, another window taking
    /// the focus): the game stays as it is, paused.
    Dismiss,
    Quit,
}

/// Everything the overlay shows, prepared before it connects to the display.
pub struct Content {
    pub config: Config,
    pub theme: MenuTheme,
    pub fonts: Fonts,
    pub title: String,
    pub subtitle: String,
    pub items: Vec<MenuItem>,
    pub clock_formats: (String, String),
    pub locale: chrono::Locale,
    pub pads: Vec<Pad>,
    pub emulator: Emulator,
    /// The save and load pages, when the emulator's states can be managed.
    pub states: Option<StatesMenu>,
    /// The game's manual, when it has one that can be shown.
    pub manual: Option<Manual>,
    /// For the texts that change while the menu is open (the disc line).
    pub translator: Translator,
    /// When the process started, to log how long the menu took to show.
    pub launched: Instant,
}

struct State {
    display: Display,
    /// Whether the menu has the keyboard focus.
    focused: bool,
    handle: LoopHandle<'static, State>,

    content: Content,
    dim: Piece,
    panel: Option<(Panel, Piece)>,
    clock: Option<(Clock, Piece)>,
    /// The help style, at this screen's size.
    help: Option<Help>,
    /// The menu's help prompts, on the dim.
    menu_help: Option<Sub>,
    manual: ManualState,
    /// The controller direction held, repeating like a held key.
    repeat: Option<(Nav, RegistrationToken)>,
    started: Option<Instant>,
    outcome: Option<Outcome>,
}

/// A pointer event, whichever the display.
pub(crate) struct PointerInput {
    surface: SurfaceId,
    /// On the surface, in pixels.
    position: (f64, f64),
    kind: PointerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerKind {
    Enter,
    Leave,
    Motion,
    Press(u32),
    Release(u32),
    /// Wheel notches, down when positive.
    Wheel(i32),
}

/// Show the overlay until the user leaves it: on Wayland with layer-shell,
/// else on X11 (an X11 session, or `XWayland` where the compositor has no
/// layer-shell).
pub fn run(content: Content) -> Result<(Outcome, Emulator)> {
    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("cannot create the event loop")?;
    let handle = event_loop.handle();
    let mut display = match connect_wayland(&handle, content.config.compositor_scaling) {
        Ok(display) => display,
        Err(wayland) => {
            log::info!("{wayland:#}: trying X11");
            connect_x11(&handle)
                .with_context(|| format!("{wayland:#}"))
                .context("no display to show the overlay on")?
        }
    };

    // SIGUSR1: the overlay started again (the hotkey, Shift+F7) closes this
    // one, back to the game like Continue; being stopped leaves it paused
    let signals = Signals::new(&[Signal::SIGUSR1, Signal::SIGTERM, Signal::SIGINT])?;
    handle
        .insert_source(signals, |event, &mut (), state| {
            state.outcome = Some(if event.signal() == Signal::SIGUSR1 {
                Outcome::Continue
            } else {
                Outcome::Dismiss
            });
        })
        .map_err(|err| anyhow!("cannot watch signals: {}", err.error))?;

    watch_controllers(&handle, &content.pads)?;

    // Everything the overlay needs is there: pause the game now, before the
    // overlay shows itself (the game still has the keyboard focus). Failing
    // any sooner (no display it can show on) leaves the game running.
    content.emulator.pause();

    // the dim comes first: its size is the screen's, which the rest is sized from
    let dim = display.create_piece(Spec {
        anchor: Anchor::all(),
        size: (0, 0),
        margin: [0; 4],
        keyboard: false,
        fade: true,
    });
    if let Some(window) = dim.x11_window() {
        grab_keyboard(&handle, window)?;
    }
    let mut state = State {
        display,
        focused: false,
        handle,
        content,
        dim,
        panel: None,
        clock: None,
        help: None,
        menu_help: None,
        manual: ManualState::default(),
        repeat: None,
        started: None,
        outcome: None,
    };

    while state.outcome.is_none() {
        state.x11_configures();
        state.display.flush();
        if let Err(err) = event_loop.dispatch(None, &mut state) {
            // the overlay is gone: do not leave the game paused behind it
            state.content.emulator.back_to_game(None);
            return Err(err).context("event loop error");
        }
    }
    let outcome = state.outcome.unwrap_or(Outcome::Dismiss);
    // Hide the overlay first, then hand the controllers back once the button
    // that closed it is up (see input.rs).
    let mut pads = std::mem::take(&mut state.content.pads);
    let emulator = std::mem::replace(&mut state.content.emulator, Emulator::none());
    let State { display, .. } = state;
    if let Display::X11(x11) = &display {
        x11.restore_focus();
    }
    display.flush();
    drop(display);
    input::wait_released(&mut pads);
    drop(pads);
    Ok((outcome, emulator))
}

/// The Wayland compositor, if it has what the overlay needs.
fn connect_wayland(
    handle: &LoopHandle<'static, State>,
    compositor_scaling: bool,
) -> Result<Display> {
    let conn = Connection::connect_to_env().context("no Wayland compositor")?;
    let (globals, queue) = registry_queue_init(&conn).context("cannot list the Wayland globals")?;
    let wl = display::bind_wayland(conn.clone(), &globals, queue.handle(), compositor_scaling)?;
    WaylandSource::new(conn, queue)
        .insert(handle.clone())
        .map_err(|err| anyhow!("cannot watch the Wayland socket: {}", err.error))?;
    Ok(Display::Wayland(Box::new(wl)))
}

/// The X server, its events read in the event loop.
fn connect_x11(handle: &LoopHandle<'static, State>) -> Result<Display> {
    let x11 = X11::connect()?;
    let fd = x11.conn.stream().as_fd().try_clone_to_owned()?;
    handle
        .insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            |_, _, state| {
                state.x11_events();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| anyhow!("cannot watch the X11 connection: {}", err.error))?;
    Ok(Display::X11(x11))
}

/// Read the controllers' events, and grab them all once `GRAB_WAIT` has
/// passed (one whose buttons are not all up yet is grabbed by then anyway).
fn watch_controllers(handle: &LoopHandle<'static, State>, pads: &[Pad]) -> Result<()> {
    for (index, pad) in pads.iter().enumerate() {
        let fd = pad.device.as_fd().try_clone_to_owned()?;
        handle
            .insert_source(
                Generic::new(fd, Interest::READ, Mode::Level),
                move |_, _, state| {
                    for input in state.content.pads[index].read() {
                        state.pad_input(input);
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| anyhow!("cannot watch a controller: {}", err.error))?;
    }
    handle
        .insert_source(
            Timer::from_duration(input::GRAB_WAIT),
            |_, &mut (), state| {
                for pad in &mut state.content.pads {
                    pad.try_grab();
                }
                TimeoutAction::Drop
            },
        )
        .map_err(|err| anyhow!("cannot start a timer: {}", err.error))?;
    Ok(())
}

/// How often, and how many times, the X11 keyboard grab is tried again while
/// another client holds the keyboard (a window manager shortcut being held).
const GRAB_RETRY: Duration = Duration::from_millis(50);
const GRAB_TRIES: u32 = 40;

/// Grab the keyboard on X11, trying again for a while if it is taken.
fn grab_keyboard(handle: &LoopHandle<'static, State>, window: xproto::Window) -> Result<()> {
    let mut tries = 0;
    handle
        .insert_source(Timer::immediate(), move |_, &mut (), state| {
            let Some(x11) = state.display.x11() else {
                return TimeoutAction::Drop;
            };
            if x11.grab_keyboard(window) {
                log::debug!("keyboard grabbed");
                return TimeoutAction::Drop;
            }
            tries += 1;
            if tries >= GRAB_TRIES {
                log::warn!("cannot grab the keyboard: keys follow the focus");
                return TimeoutAction::Drop;
            }
            TimeoutAction::ToDuration(GRAB_RETRY)
        })
        .map_err(|err| anyhow!("cannot start a timer: {}", err.error))?;
    Ok(())
}

/// Smooth start and end of the animations.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

impl State {
    fn progress(&self) -> f32 {
        let total = self.content.config.animation_ms;
        match self.started {
            Some(start) if total > 0 => {
                (start.elapsed().as_millis() as f32 / total as f32).clamp(0.0, 1.0)
            }
            Some(_) => 1.0,
            None => 0.0, // the animation has not started yet
        }
    }

    /// The dim: translucent black over the whole output, drawn straight into
    /// the buffer (no pixmap: it can be a 4K screen).
    fn draw_dim(&mut self) {
        let alpha = (self.content.config.dim.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.dim
            .set_opacity(&self.display, ease_out(self.progress()));
        // premultiplied black: only the alpha byte (B, G, R, A in memory)
        self.dim.fill(&mut self.display, [0, 0, 0, alpha]);
    }

    /// Once the output size is known: create the panel and the clock.
    fn create_pieces(&mut self) {
        let (width, height) = self.dim.size;
        let config = &self.content.config;
        let mut panel = Panel::new(
            &self.content.theme,
            (width, height),
            &mut self.content.fonts,
            &self.content.title,
            &self.content.subtitle,
            std::mem::take(&mut self.content.items),
        );
        panel.set_icons(config.icons);
        let (edge, other) = match config.side {
            Side::Right => (Anchor::RIGHT, Anchor::LEFT),
            Side::Left => (Anchor::LEFT, Anchor::RIGHT),
        };

        // hidden past its edge; it slides in (see animate)
        let piece = self.display.create_piece(Spec {
            anchor: Anchor::TOP | Anchor::BOTTOM | edge,
            size: (panel.width, 0),
            margin: edge_margin(config.side, -(panel.width as i32)),
            keyboard: true,
            fade: true,
        });
        self.panel = Some((panel, piece));

        if config.clock.enabled {
            let free =
                width.saturating_sub(self.panel.as_ref().map_or(0, |(panel, _)| panel.width));
            let clock = Clock::new(
                &self.content.theme.title.font.path,
                height,
                self.content.clock_formats.clone(),
                self.content.locale,
                config.clock.color,
                // towards the edge of its corner: left when the menu is right
                match config.side {
                    Side::Right => cosmic_text::Align::Left,
                    Side::Left => cosmic_text::Align::Right,
                },
            );
            let margin = (height as f32 * CLOCK_MARGIN) as i32;
            let (vertical, top, bottom) = match config.clock.position {
                VerticalPosition::Top => (Anchor::TOP, margin, 0),
                VerticalPosition::Bottom => (Anchor::BOTTOM, 0, margin),
                VerticalPosition::Center => (Anchor::empty(), 0, 0),
            };
            let piece = self.display.create_piece(Spec {
                anchor: other | vertical,
                size: (free.max(1), clock.height()),
                margin: [top, 0, bottom, 0],
                keyboard: false,
                fade: true,
            });
            self.clock = Some((clock, piece));
            self.schedule_clock();
        }

        self.help = Some(Help::new(&self.content.theme, (width, height)));
        self.menu_help = Sub::new(&mut self.display, &self.dim, false, true);
        self.draw_menu_help();

        self.started = Some(Instant::now());
        self.handle
            .insert_source(Timer::from_duration(FRAME), |_, &mut (), state| {
                state.animate()
            })
            .ok();
    }

    /// One animation frame: opacities and the panel's position.
    fn animate(&mut self) -> TimeoutAction {
        let t = ease_out(self.progress());
        self.dim.set_opacity(&self.display, t);
        if let Some(help) = &self.menu_help {
            help.set_opacity(t);
            help.commit();
        }
        // a subsurface's state takes effect with its parent's
        self.dim.commit();
        if let Some((panel, piece)) = &mut self.panel {
            let margin = (-(panel.width as f32) * (1.0 - t)).round() as i32;
            piece.set_margin(&self.display, edge_margin(self.content.config.side, margin));
            piece.set_opacity(&self.display, t);
            piece.commit();
        }
        if let Some((_, piece)) = &self.clock {
            piece.set_opacity(&self.display, t);
            piece.commit();
        }
        if t >= 1.0 {
            TimeoutAction::Drop
        } else {
            TimeoutAction::ToDuration(FRAME)
        }
    }

    /// Redraw the clock at the start of every second.
    fn schedule_clock(&mut self) {
        let until_next = 1000 - u64::from(Local::now().nanosecond() / 1_000_000).min(999);
        self.handle
            .insert_source(
                Timer::from_duration(Duration::from_millis(until_next)),
                |_, &mut (), state| {
                    state.draw_clock();
                    let until_next =
                        1000 - u64::from(Local::now().nanosecond() / 1_000_000).min(999);
                    TimeoutAction::ToDuration(Duration::from_millis(until_next))
                },
            )
            .ok();
    }

    fn draw_panel(&mut self) {
        if let Some((panel, piece)) = &mut self.panel {
            if piece.size.1 == 0 {
                return;
            }
            if let Some(pixmap) = panel.render(&mut self.content.fonts, piece.size.1) {
                if !piece.has_content() {
                    log::debug!(
                        "menu shown {} ms after start",
                        self.content.launched.elapsed().as_millis()
                    );
                }
                piece.present(&mut self.display, &pixmap);
            }
        }
    }

    fn draw_clock(&mut self) {
        if let Some((clock, piece)) = &mut self.clock {
            if piece.size.0 == 0 {
                return;
            }
            if let Some(pixmap) = clock.render(&mut self.content.fonts, piece.size.0, &Local::now())
            {
                piece.present(&mut self.display, &pixmap);
            }
        }
    }

    /// The menu's help prompts, as ES's menus have them, in the part of the
    /// screen the panel leaves free.
    fn draw_menu_help(&mut self) {
        let (Some(help), Some(sub), Some((panel, _))) = (
            self.help.as_mut(),
            self.menu_help.as_mut(),
            self.panel.as_ref(),
        ) else {
            return;
        };
        let texts = &self.content.translator;
        let prompt = |icon: &str, key: &str| Prompt {
            icon: icon.to_owned(),
            label: texts.text(key),
        };
        let back = if panel.page_open() {
            "help-back"
        } else {
            "help-close"
        };
        let prompts = [
            prompt("up/down", "help-choose"),
            prompt(help.ok(), "help-select"),
            prompt(help.back(), back),
        ];
        let screen_width = self.dim.size.0 as f32;
        let free = screen_width - panel.width as f32 - help.left() * 2.0;
        match help.render(&mut self.content.fonts, &prompts, free, None) {
            Some(pixmap) => {
                let (mut x, y) = help.place(pixmap.width());
                if self.content.config.side == Side::Left {
                    x += panel.width as i32;
                }
                sub.move_to(x, y);
                sub.show_pixmap(&mut self.display, &pixmap);
            }
            None => sub.hide(),
        }
        sub.commit();
        self.dim.commit();
    }

    /// Show the disc in the drive and the tray, and fit the disc entries to
    /// them (open or close the tray; changing the disc needs it open).
    fn refresh_discs(&mut self) {
        let Some(discs) = self.content.emulator.discs() else {
            return;
        };
        let Some((panel, _)) = self.panel.as_mut() else {
            return;
        };
        let texts = &self.content.translator;
        // a single disc: only whether the tray is open
        let mut parts = Vec::new();
        if discs.count > 1 {
            parts.push(texts.text_args(
                "disc-status",
                &[
                    ("current", i64::from(discs.current)),
                    ("count", i64::from(discs.count)),
                ],
            ));
        }
        if discs.tray_open {
            parts.push(texts.text("disc-tray-open"));
        }
        // "Tray open" on its own starts the line
        let status = (!parts.is_empty()).then(|| {
            let line = parts.join(" · ");
            let mut chars = line.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        });
        panel.set_status(status);
        let tray = if discs.tray_open {
            "menu-close-tray"
        } else {
            "menu-open-tray"
        };
        panel.set_entry(
            MenuAction::Emulator(EmuAction::DiscTray),
            Some(&texts.text(tray)),
            true,
        );
        for action in [EmuAction::NextDisc, EmuAction::PrevDisc] {
            panel.set_entry(MenuAction::Emulator(action), None, discs.allows(action));
        }
    }

    /// Do what an entry asks.
    fn choose(&mut self, index: usize) {
        let action = self
            .panel
            .as_ref()
            .and_then(|(panel, _)| panel.action(index));
        self.activate(action);
    }

    fn activate(&mut self, action: Option<MenuAction>) {
        self.outcome = match action {
            None => return, // e.g. an empty slot to load
            Some(MenuAction::States(mode)) => {
                self.open_states(mode);
                return;
            }
            Some(MenuAction::Manual) => {
                self.open_manual();
                return;
            }
            Some(MenuAction::Continue) => Some(Outcome::Continue),
            Some(MenuAction::Quit) => Some(Outcome::Quit),
            // disc changes are done with the menu open, which shows the result
            Some(MenuAction::Emulator(action)) if self.content.emulator.in_place(action) => {
                if self.content.emulator.perform_now(action) {
                    self.refresh_discs();
                    self.draw_panel();
                }
                return;
            }
            Some(MenuAction::Emulator(action)) => Some(Outcome::Request(Request::Action(action))),
            Some(MenuAction::Slot(StateMode::Save, slot)) => {
                Some(Outcome::Request(Request::SaveState(slot)))
            }
            Some(MenuAction::Slot(StateMode::Load, slot)) => {
                Some(Outcome::Request(Request::LoadState(slot)))
            }
        };
    }

    /// Show the save or load page, with what each slot holds now.
    fn open_states(&mut self, mode: StateMode) {
        let Some(menu) = &self.content.states else {
            return;
        };
        let page = menu.page(mode);
        if let Some((panel, _)) = self.panel.as_mut() {
            panel.open_page(page);
            self.draw_panel();
            self.draw_menu_help();
        }
    }

    /// Back: from a save state page to the main entries, else close.
    fn back(&mut self) {
        let closed_page = self
            .panel
            .as_mut()
            .is_some_and(|(panel, _)| panel.close_page());
        if closed_page {
            self.draw_panel();
            self.draw_menu_help();
        } else {
            self.outcome = Some(Outcome::Dismiss);
        }
    }

    /// A press or release on a controller: a held direction repeats, as a
    /// held key does (the compositor repeats the keyboard's).
    fn pad_input(&mut self, input: Input) {
        let repeats = matches!(
            input.nav,
            Nav::Up | Nav::Down | Nav::Left | Nav::Right | Nav::PrevPage | Nav::NextPage
        );
        if repeats && (input.pressed || self.repeat.is_some_and(|(nav, _)| nav == input.nav)) {
            if let Some((_, token)) = self.repeat.take() {
                self.handle.remove(token);
            }
            if input.pressed {
                let nav = input.nav;
                let token = self.handle.insert_source(
                    Timer::from_duration(REPEAT_DELAY),
                    move |_, &mut (), state| {
                        state.input(Input::press(nav));
                        TimeoutAction::ToDuration(REPEAT_INTERVAL)
                    },
                );
                match token {
                    Ok(token) => self.repeat = Some((nav, token)),
                    Err(err) => log::debug!("no repeat timer: {}", err.error),
                }
            }
        }
        self.input(input);
    }

    /// A press or release, from a controller or the keyboard.
    fn input(&mut self, input: Input) {
        if self.manual_is_open() {
            self.manual_input(input);
        } else if input.pressed {
            self.navigate(input.nav);
        }
    }

    /// A press on the menu.
    fn navigate(&mut self, nav: Nav) {
        match nav {
            Nav::Up | Nav::Down => {
                let delta = if nav == Nav::Up { -1 } else { 1 };
                let moved = self
                    .panel
                    .as_mut()
                    .is_some_and(|(panel, _)| panel.move_selection(delta));
                if moved {
                    self.draw_panel();
                }
            }
            Nav::Confirm => {
                let action = self
                    .panel
                    .as_ref()
                    .and_then(|(panel, _)| panel.selected_action());
                self.activate(action);
            }
            Nav::Back => self.back(),
            // the shortcut that opened the overlay, again: back to the game
            Nav::Close => self.outcome = Some(Outcome::Continue),
            Nav::Left | Nav::Right | Nav::ZoomIn | Nav::ZoomOut | Nav::PrevPage | Nav::NextPage => {
            }
        }
    }
}

/// The panel's margins (top, right, bottom, left): `margin` on its edge.
const fn edge_margin(side: Side, margin: i32) -> [i32; 4] {
    match side {
        Side::Right => [0, margin, 0, 0],
        Side::Left => [0, 0, 0, margin],
    }
}

/// What the overlay does whatever the display: sizes, keys, the pointer,
/// the focus.
impl State {
    /// A surface has its size (Wayland's configure; at once on X11).
    fn configured(&mut self, surface: &SurfaceId, size: (u32, u32)) {
        if self.dim.is(surface) {
            let first = self.dim.size == (0, 0);
            self.dim.size = size;
            self.draw_dim();
            if first && size.0 > 0 && size.1 > 0 {
                self.create_pieces();
                self.refresh_discs();
            }
        } else if let Some((_, piece)) = self.panel.as_mut().filter(|(_, piece)| piece.is(surface))
        {
            piece.size = size;
            self.draw_panel();
        } else if let Some((_, piece)) = self.clock.as_mut().filter(|(_, piece)| piece.is(surface))
        {
            piece.size = size;
            self.draw_clock();
        } else {
            self.manual_configured(surface, size);
        }
    }

    fn focus_in(&mut self) {
        log::debug!("keyboard focus");
        self.focused = true;
    }

    /// Another window took the keyboard (Alt+Tab, a click elsewhere): get
    /// out of the way.
    fn focus_out(&mut self) {
        if self.focused && self.outcome.is_none() {
            log::info!("keyboard focus lost: closing");
            self.outcome = Some(Outcome::Dismiss);
        }
        self.focused = false;
    }

    fn key_press(&mut self, nav: Nav) {
        self.input(Input::press(nav));
    }

    /// Held keys repeat in the menu; the manual follows them being held.
    fn key_repeat(&mut self, nav: Nav) {
        match nav {
            Nav::Up | Nav::Down if !self.manual_is_open() => self.navigate(nav),
            // turning pages; a zoomed page follows the key being held instead
            Nav::Up | Nav::Down | Nav::Left | Nav::Right | Nav::PrevPage | Nav::NextPage
                if self.manual_is_open() =>
            {
                self.input(Input::press(nav));
            }
            _ => {}
        }
    }

    fn key_release(&mut self, nav: Nav) {
        self.input(Input::release(nav));
    }

    /// Hovering an entry highlights it, a click opens it; a click anywhere else
    /// (or the right button) closes the overlay; the wheel moves the selection.
    fn pointer(&mut self, event: &PointerInput) {
        if self.is_manual_surface(&event.surface) {
            self.manual_pointer(event);
            return;
        }
        let on_panel = self
            .panel
            .as_ref()
            .is_some_and(|(_, piece)| piece.is(&event.surface));
        let item = if on_panel {
            self.panel
                .as_ref()
                .and_then(|(panel, _)| panel.item_at(event.position.1 as f32))
        } else {
            None
        };
        match event.kind {
            PointerKind::Enter | PointerKind::Motion => self.hover(item),
            PointerKind::Leave => self.hover(None),
            PointerKind::Press(button) => match (button, item) {
                (BTN_LEFT, Some(index)) => self.choose(index),
                (BTN_LEFT, None) if !on_panel => self.navigate(Nav::Back),
                (BTN_RIGHT, _) => self.navigate(Nav::Back),
                _ => {}
            },
            PointerKind::Wheel(steps) if on_panel && steps != 0 => {
                self.navigate(if steps < 0 { Nav::Up } else { Nav::Down });
            }
            _ => {}
        }
    }
}

/// X11's events.
impl State {
    /// The sizes of the windows created since, as configures.
    fn x11_configures(&mut self) {
        let configures = self
            .display
            .x11()
            .map(|x11| std::mem::take(&mut x11.configures))
            .unwrap_or_default();
        for (window, size) in configures {
            self.configured(&SurfaceId::X11(window), size);
        }
    }

    fn x11_events(&mut self) {
        let Some(conn) = self.display.x11().map(|x11| Rc::clone(&x11.conn)) else {
            return;
        };
        loop {
            match conn.poll_for_event() {
                Ok(Some(event)) => self.x11_event(event),
                Ok(None) => break,
                Err(err) => {
                    log::error!("X11 connection lost: {err}");
                    self.outcome = Some(Outcome::Dismiss);
                    break;
                }
            }
        }
    }

    fn x11_event(&mut self, event: XEvent) {
        let pointer = |window, x: i16, y: i16, kind| PointerInput {
            surface: SurfaceId::X11(window),
            position: (f64::from(x), f64::from(y)),
            kind,
        };
        match event {
            XEvent::Expose(event) if event.count == 0 => self.x11_expose(event.window),
            XEvent::KeyPress(event) => {
                if let Some(nav) = x11_key_nav(event.detail) {
                    self.key_press(nav);
                }
            }
            XEvent::KeyRelease(event) => {
                if let Some(nav) = x11_key_nav(event.detail) {
                    self.key_release(nav);
                }
            }
            XEvent::ButtonPress(event) => {
                let kind = match event.detail {
                    1 => PointerKind::Press(BTN_LEFT),
                    3 => PointerKind::Press(BTN_RIGHT),
                    4 => PointerKind::Wheel(-1),
                    5 => PointerKind::Wheel(1),
                    _ => return,
                };
                self.pointer(&pointer(event.event, event.event_x, event.event_y, kind));
            }
            XEvent::ButtonRelease(event) => {
                let kind = match event.detail {
                    1 => PointerKind::Release(BTN_LEFT),
                    3 => PointerKind::Release(BTN_RIGHT),
                    _ => return,
                };
                self.pointer(&pointer(event.event, event.event_x, event.event_y, kind));
            }
            XEvent::MotionNotify(event) => self.pointer(&pointer(
                event.event,
                event.event_x,
                event.event_y,
                PointerKind::Motion,
            )),
            XEvent::EnterNotify(event) => self.pointer(&pointer(
                event.event,
                event.event_x,
                event.event_y,
                PointerKind::Enter,
            )),
            XEvent::LeaveNotify(event) => self.pointer(&pointer(
                event.event,
                event.event_x,
                event.event_y,
                PointerKind::Leave,
            )),
            XEvent::FocusIn(event) if event.mode != NotifyMode::GRAB => self.focus_in(),
            // a grab (a shortcut held by the window manager) or a child
            // window taking it is not another window taking the keyboard
            XEvent::FocusOut(event)
                if event.mode == NotifyMode::NORMAL && event.detail != NotifyDetail::INFERIOR =>
            {
                self.focus_out();
            }
            // the window manager restacking or mapping another window (a
            // fullscreen game raised again as it gets the focus back)
            XEvent::ConfigureNotify(event) => self.x11_restacked(event.window),
            XEvent::MapNotify(event) => self.x11_restacked(event.window),
            XEvent::Error(err) => log::warn!("X11 error: {err:?}"),
            _ => {}
        }
    }

    /// Another top-level window moved in the stack or showed up: put the
    /// overlay back on top of it.
    fn x11_restacked(&mut self, window: xproto::Window) {
        if let Some(x11) = self.display.x11()
            && !x11.is_top(window)
        {
            log::debug!("window {window:#x} restacked: raising the overlay");
            x11.raise();
        }
    }

    /// Paint a window again: X11 keeps nothing of what is not on screen.
    fn x11_expose(&self, window: xproto::Window) {
        let pieces = [
            Some(&self.dim),
            self.panel.as_ref().map(|(_, piece)| piece),
            self.clock.as_ref().map(|(_, piece)| piece),
        ];
        if pieces
            .into_iter()
            .flatten()
            .any(|piece| piece.expose(window))
        {
            return;
        }
        if self
            .menu_help
            .as_ref()
            .is_some_and(|sub| sub.expose(window))
        {
            return;
        }
        self.manual_expose(window);
    }
}

/// What a key does on X11, by its key code: an evdev code plus 8 with the
/// usual drivers (evdev, libinput), whatever the keyboard layout.
fn x11_key_nav(keycode: u8) -> Option<Nav> {
    use evdev::KeyCode;
    let code = KeyCode::new(u16::from(keycode).checked_sub(8)?);
    Some(match code {
        KeyCode::KEY_UP | KeyCode::KEY_KP8 => Nav::Up,
        KeyCode::KEY_DOWN | KeyCode::KEY_KP2 => Nav::Down,
        KeyCode::KEY_LEFT | KeyCode::KEY_KP4 => Nav::Left,
        KeyCode::KEY_RIGHT | KeyCode::KEY_KP6 => Nav::Right,
        KeyCode::KEY_PAGEUP | KeyCode::KEY_KP9 => Nav::PrevPage,
        KeyCode::KEY_PAGEDOWN | KeyCode::KEY_KP3 => Nav::NextPage,
        KeyCode::KEY_EQUAL | KeyCode::KEY_KPPLUS => Nav::ZoomIn,
        KeyCode::KEY_MINUS | KeyCode::KEY_KPMINUS => Nav::ZoomOut,
        KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER | KeyCode::KEY_SPACE => Nav::Confirm,
        KeyCode::KEY_ESC | KeyCode::KEY_BACKSPACE => Nav::Back,
        KeyCode::KEY_F7 => Nav::Close,
        _ => return None,
    })
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.outcome = Some(Outcome::Dismiss);
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.configured(
            &SurfaceId::Wayland(layer.wl_surface().clone()),
            configure.new_size,
        );
    }
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.focus_in();
    }

    /// The compositor gave the keyboard to another window (Alt+Tab, a
    /// click elsewhere): get out of the way.
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.focus_out();
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        log::debug!("key {:?}", event.keysym);
        if let Some(nav) = key_nav(event.keysym) {
            self.key_press(nav);
        }
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if let Some(nav) = key_nav(event.keysym) {
            self.key_repeat(nav);
        }
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if let Some(nav) = key_nav(event.keysym) {
            self.key_release(nav);
        }
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
    }
}

/// What a key does: the arrows, Enter, Esc and F7 like a controller; Page
/// Up/Down turn the manual's pages, + and - zoom it.
fn key_nav(keysym: Keysym) -> Option<Nav> {
    Some(match keysym {
        Keysym::Up | Keysym::KP_Up => Nav::Up,
        Keysym::Down | Keysym::KP_Down => Nav::Down,
        Keysym::Left | Keysym::KP_Left => Nav::Left,
        Keysym::Right | Keysym::KP_Right => Nav::Right,
        Keysym::Prior | Keysym::KP_Prior => Nav::PrevPage,
        Keysym::Next | Keysym::KP_Next => Nav::NextPage,
        Keysym::plus | Keysym::equal | Keysym::KP_Add => Nav::ZoomIn,
        Keysym::minus | Keysym::KP_Subtract => Nav::ZoomOut,
        Keysym::Return | Keysym::KP_Enter | Keysym::space => Nav::Confirm,
        Keysym::Escape | Keysym::BackSpace => Nav::Back,
        Keysym::F7 => Nav::Close,
        _ => return None,
    })
}

/// Linux's button codes (input-event-codes.h).
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let kind = match event.kind {
                PointerEventKind::Enter { .. } => {
                    if let Some(pointer) = &self.display.wl().pointer
                        && let Err(err) = pointer.set_cursor(conn, CursorIcon::Default)
                    {
                        log::debug!("cannot set the cursor: {err}");
                    }
                    PointerKind::Enter
                }
                PointerEventKind::Leave { .. } => PointerKind::Leave,
                PointerEventKind::Motion { .. } => PointerKind::Motion,
                PointerEventKind::Press { button, .. } => PointerKind::Press(button),
                PointerEventKind::Release { button, .. } => PointerKind::Release(button),
                PointerEventKind::Axis { vertical, .. } => {
                    PointerKind::Wheel(if vertical.discrete != 0 {
                        vertical.discrete
                    } else {
                        (vertical.absolute / 10.0) as i32
                    })
                }
            };
            self.pointer(&PointerInput {
                surface: SurfaceId::Wayland(event.surface.clone()),
                position: event.position,
                kind,
            });
        }
    }
}

impl State {
    /// Highlight the entry under the pointer, without selecting it.
    fn hover(&mut self, item: Option<usize>) {
        let changed = self
            .panel
            .as_mut()
            .is_some_and(|(panel, _)| panel.hover(item));
        if changed {
            self.draw_panel();
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.display.wl().seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        log::debug!("seat capability: {capability}");
        let wl = self.display.wl();
        if capability == Capability::Keyboard && wl.keyboard.is_none() {
            match wl.seat_state.get_keyboard(qh, &seat, None) {
                Ok(keyboard) => wl.keyboard = Some(keyboard),
                Err(err) => log::warn!("no keyboard: {err}"),
            }
        }
        if capability == Capability::Pointer && wl.pointer.is_none() {
            let cursor_surface = wl.compositor.create_surface(qh);
            match wl.seat_state.get_pointer_with_theme::<Self, ()>(
                qh,
                &seat,
                wl.shm.wl_shm(),
                cursor_surface,
                ThemeSpec::default(),
            ) {
                Ok(pointer) => wl.pointer = Some(pointer),
                Err(err) => log::warn!("no pointer: {err}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.display.wl().keyboard.take()
        {
            keyboard.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.display.wl().output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.display.wl().shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.display.wl().registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
wayland_client::delegate_noop!(State: WpAlphaModifierV1);
wayland_client::delegate_noop!(State: WpAlphaModifierSurfaceV1);
wayland_client::delegate_noop!(State: WpViewporter);
wayland_client::delegate_noop!(State: WpViewport);
smithay_client_toolkit::delegate_dispatch2!(State);
