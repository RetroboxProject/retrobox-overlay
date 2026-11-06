//! The overlay on screen: three layer-shell surfaces in the overlay layer.
//!
//! * the dim: one translucent black buffer over the whole output;
//! * the panel: the side menu, slid in from its edge by animating the layer
//!   surface's margin (its buffer is drawn once, not on every frame); it takes
//!   the keyboard, once the game is paused (the pause key has to reach the
//!   game first, see emulator.rs);
//! * the clock, on the other side, redrawn once a second.
//!
//! With `wp_alpha_modifier` (KWin 6.1+, labwc 0.9+) the three fade in as well,
//! by the compositor: nothing is drawn again for the fade. (A single-pixel
//! buffer stretched by `wp_viewporter` would make the dim free, but software
//! compositors filter it to nothing; a full buffer works everywhere.)
//!
//! The dim surface comes first: its configure tells the output size, which
//! the other two are sized from.

use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use chrono::{Local, Timelike};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::signals::{Signal, Signals};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState, SimpleGlobal};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use tiny_skia::Pixmap;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1;
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_v1::WpAlphaModifierV1;

use crate::config::{Config, Side, VerticalPosition};
use crate::emulator::{Emulator, Request};
use crate::input::{self, Nav, Pad};
use crate::savestates::StateMode;
use crate::theme::MenuTheme;
use crate::ui::clock::Clock;
use crate::ui::panel::{MenuAction, MenuItem, Panel};
use crate::ui::states::StatesMenu;
use crate::ui::text::Fonts;
use crate::ui::to_argb8888;

const NAMESPACE: &str = "retrobox-overlay";
const FRAME: Duration = Duration::from_millis(16);
/// Space between the clock and the screen edge when it is at the top or bottom.
const CLOCK_MARGIN: f32 = 0.08;

/// How the overlay was left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// "Continue": back to the game, running.
    Continue,
    /// An emulator action, then back to the game, running.
    Request(Request),
    /// Closed without a choice (back, Home, a click outside, another window
    /// taking the focus): the game stays as it is, paused.
    Dismiss,
    Quit,
}

/// Everything the overlay shows, prepared before it connects to Wayland.
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
    /// When the process started, to log how long the menu took to show.
    pub launched: Instant,
}

/// A layer surface and what is shown on it.
struct Piece {
    layer: LayerSurface,
    size: (u32, u32),
    buffer: Option<Buffer>,
    fade: Option<WpAlphaModifierSurfaceV1>,
}

impl Piece {
    fn new(
        layer: LayerSurface,
        alpha: Option<&WpAlphaModifierV1>,
        qh: &QueueHandle<State>,
    ) -> Self {
        let fade = alpha.map(|alpha| {
            let fade = alpha.get_surface(layer.wl_surface(), qh, ());
            fade.set_multiplier(0);
            fade
        });
        Self {
            layer,
            size: (0, 0),
            buffer: None,
            fade,
        }
    }

    /// Set the opacity the compositor applies (0..1), for the next commit.
    fn set_opacity(&self, opacity: f32) {
        if let Some(fade) = &self.fade {
            fade.set_multiplier((f64::from(opacity.clamp(0.0, 1.0)) * f64::from(u32::MAX)) as u32);
        }
    }

    fn is(&self, surface: &wl_surface::WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    /// Attach a pixmap as the surface's content.
    fn present(&mut self, pool: &mut SlotPool, pixmap: &Pixmap) {
        let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
        let Ok((buffer, canvas)) =
            pool.create_buffer(width, height, width * 4, wl_shm::Format::Argb8888)
        else {
            log::warn!("cannot allocate a {width}x{height} buffer");
            return;
        };
        to_argb8888(pixmap, canvas);
        let surface = self.layer.wl_surface();
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, width, height);
            self.layer.commit();
            self.buffer = Some(buffer);
        }
    }
}

#[allow(clippy::struct_field_names, reason = "SCTK's usual names")]
struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    alpha: Option<WpAlphaModifierV1>,
    pool: SlotPool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    /// Whether the menu has the keyboard focus.
    focused: bool,
    qh: QueueHandle<State>,
    handle: LoopHandle<'static, State>,

    content: Content,
    dim: Piece,
    panel: Option<(Panel, Piece)>,
    clock: Option<(Clock, Piece)>,
    started: Option<Instant>,
    outcome: Option<Outcome>,
}

/// Show the overlay until the user leaves it.
pub fn run(content: Content) -> Result<(Outcome, Emulator)> {
    let conn =
        Connection::connect_to_env().context("no Wayland compositor to show the overlay on")?;
    let (globals, queue) = registry_queue_init(&conn).context("cannot list the Wayland globals")?;
    let qh: QueueHandle<State> = queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor missing")?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| {
        anyhow!(
            "the compositor has no layer-shell (KWin and wlroots compositors such as labwc have it)"
        )
    })?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm missing")?;
    let pool = SlotPool::new(4 * 1024 * 1024, &shm).context("cannot create the shm pool")?;
    let alpha = SimpleGlobal::<WpAlphaModifierV1, 1>::bind(&globals, &qh)
        .ok()
        .and_then(|global| global.get().ok().cloned());

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some(NAMESPACE), None);
    layer.set_anchor(Anchor::all());
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(0, 0);
    layer.commit();

    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("cannot create the event loop")?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|err| anyhow!("cannot watch the Wayland socket: {}", err.error))?;

    // SIGUSR1: the overlay started again (hotkey, Shift+F7) asks this one to close
    let signals = Signals::new(&[Signal::SIGUSR1, Signal::SIGTERM, Signal::SIGINT])?;
    handle
        .insert_source(signals, |_, &mut (), state| {
            state.outcome = Some(Outcome::Dismiss);
        })
        .map_err(|err| anyhow!("cannot watch signals: {}", err.error))?;

    for index in 0..content.pads.len() {
        let fd = content.pads[index].device.as_fd().try_clone_to_owned()?;
        handle
            .insert_source(
                Generic::new(fd, Interest::READ, Mode::Level),
                move |_, _, state| {
                    for nav in state.content.pads[index].read() {
                        state.navigate(nav);
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| anyhow!("cannot watch a controller: {}", err.error))?;
    }
    // a controller whose buttons are not all up yet is grabbed by then anyway
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

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        dim: Piece::new(layer, alpha.as_ref(), &qh),
        alpha,
        pool,
        keyboard: None,
        pointer: None,
        focused: false,
        qh,
        handle,
        content,
        panel: None,
        clock: None,
        started: None,
        outcome: None,
    };

    while state.outcome.is_none() {
        event_loop
            .dispatch(None, &mut state)
            .context("event loop error")?;
    }
    let outcome = state.outcome.unwrap_or(Outcome::Dismiss);
    // Hide the overlay first, then hand the controllers back once the button
    // that closed it is up (see input.rs).
    let mut pads = std::mem::take(&mut state.content.pads);
    let emulator = std::mem::replace(&mut state.content.emulator, Emulator::none());
    drop(state);
    conn.flush().ok();
    input::wait_released(&mut pads);
    drop(pads);
    Ok((outcome, emulator))
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
        let (width, height) = self.dim.size;
        let alpha = (self.content.config.dim.clamp(0.0, 1.0) * 255.0).round() as u8;
        let (width, height) = (width.max(1) as i32, height.max(1) as i32);
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(width, height, width * 4, wl_shm::Format::Argb8888)
        else {
            log::warn!("cannot allocate the {width}x{height} dim buffer");
            return;
        };
        // premultiplied black: only the alpha byte (B, G, R, A in memory)
        for pixel in canvas.as_chunks_mut::<4>().0 {
            *pixel = [0, 0, 0, alpha];
        }
        let surface = self.dim.layer.wl_surface();
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, width, height);
            self.dim.set_opacity(ease_out(self.progress()));
            self.dim.layer.commit();
            self.dim.buffer = Some(buffer);
        }
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

        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some(NAMESPACE),
            None,
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | edge);
        layer.set_exclusive_zone(-1);
        // The keyboard on demand, not exclusively: the compositor may give it
        // to another window (Alt+Tab, a click), and then the overlay closes.
        layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        layer.set_size(panel.width, 0);
        let hidden = -(panel.width as i32);
        set_edge_margin(&layer, config.side, hidden);
        layer.commit();
        self.panel = Some((panel, Piece::new(layer, self.alpha.as_ref(), &self.qh)));

        if config.clock.enabled {
            let free =
                width.saturating_sub(self.panel.as_ref().map_or(0, |(panel, _)| panel.width));
            let clock = Clock::new(
                &self.content.theme.title.font.path,
                height,
                self.content.clock_formats.clone(),
                self.content.locale,
                config.clock.color,
            );
            let surface = self.compositor.create_surface(&self.qh);
            let layer = self.layer_shell.create_layer_surface(
                &self.qh,
                surface,
                Layer::Overlay,
                Some(NAMESPACE),
                None,
            );
            let margin = (height as f32 * CLOCK_MARGIN) as i32;
            let (vertical, top, bottom) = match config.clock.position {
                VerticalPosition::Top => (Anchor::TOP, margin, 0),
                VerticalPosition::Bottom => (Anchor::BOTTOM, 0, margin),
                VerticalPosition::Center => (Anchor::empty(), 0, 0),
            };
            layer.set_anchor(other | vertical);
            layer.set_margin(top, 0, bottom, 0);
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.set_size(free.max(1), clock.height());
            layer.commit();
            self.clock = Some((clock, Piece::new(layer, self.alpha.as_ref(), &self.qh)));
            self.schedule_clock();
        }

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
        self.dim.set_opacity(t);
        self.dim.layer.commit();
        if let Some((panel, piece)) = &self.panel {
            let margin = (-(panel.width as f32) * (1.0 - t)).round() as i32;
            set_edge_margin(&piece.layer, self.content.config.side, margin);
            piece.set_opacity(t);
            piece.layer.commit();
        }
        if let Some((_, piece)) = &self.clock {
            piece.set_opacity(t);
            piece.layer.commit();
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
                if piece.buffer.is_none() {
                    log::debug!(
                        "menu shown {} ms after start",
                        self.content.launched.elapsed().as_millis()
                    );
                }
                piece.present(&mut self.pool, &pixmap);
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
                piece.present(&mut self.pool, &pixmap);
            }
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
            Some(MenuAction::Continue) => Some(Outcome::Continue),
            Some(MenuAction::Quit) => Some(Outcome::Quit),
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
        } else {
            self.outcome = Some(Outcome::Dismiss);
        }
    }

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
            Nav::Close => self.outcome = Some(Outcome::Dismiss),
        }
    }
}

fn set_edge_margin(layer: &LayerSurface, side: Side, margin: i32) {
    match side {
        Side::Right => layer.set_margin(0, margin, 0, 0),
        Side::Left => layer.set_margin(0, 0, 0, margin),
    }
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
        let size = configure.new_size;
        let surface = layer.wl_surface();
        if self.dim.is(surface) {
            let first = self.dim.size == (0, 0);
            self.dim.size = size;
            self.draw_dim();
            if first && size.0 > 0 && size.1 > 0 {
                self.create_pieces();
            }
        } else if let Some((_, piece)) = self.panel.as_mut().filter(|(_, piece)| piece.is(surface))
        {
            piece.size = size;
            self.draw_panel();
        } else if let Some((_, piece)) = self.clock.as_mut().filter(|(_, piece)| piece.is(surface))
        {
            piece.size = size;
            self.draw_clock();
        }
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
        log::debug!("keyboard focus");
        self.focused = true;
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
        if self.focused && self.outcome.is_none() {
            log::info!("keyboard focus lost: closing");
            self.outcome = Some(Outcome::Dismiss);
        }
        self.focused = false;
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
        let nav = match event.keysym {
            Keysym::Up | Keysym::KP_Up => Some(Nav::Up),
            Keysym::Down | Keysym::KP_Down => Some(Nav::Down),
            Keysym::Return | Keysym::KP_Enter | Keysym::space => Some(Nav::Confirm),
            Keysym::Escape | Keysym::BackSpace => Some(Nav::Back),
            Keysym::F7 => Some(Nav::Close),
            _ => None,
        };
        if let Some(nav) = nav {
            self.navigate(nav);
        }
    }

    fn repeat_key(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        if matches!(
            event.keysym,
            Keysym::Up | Keysym::Down | Keysym::KP_Up | Keysym::KP_Down
        ) {
            self.press_key(conn, qh, keyboard, serial, event);
        }
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
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

/// Linux's button codes (input-event-codes.h).
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

impl PointerHandler for State {
    /// Hovering an entry highlights it, a click opens it; a click anywhere else
    /// (or the right button) closes the overlay; the wheel moves the selection.
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
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
                PointerEventKind::Enter { .. } => {
                    if let Some(pointer) = &self.pointer
                        && let Err(err) = pointer.set_cursor(conn, CursorIcon::Default)
                    {
                        log::debug!("cannot set the cursor: {err}");
                    }
                    self.hover(item);
                }
                PointerEventKind::Motion { .. } => self.hover(item),
                PointerEventKind::Leave { .. } => self.hover(None),
                PointerEventKind::Press { button, .. } => match (button, item) {
                    (BTN_LEFT, Some(index)) => self.choose(index),
                    (BTN_LEFT, None) if !on_panel => self.navigate(Nav::Back),
                    (BTN_RIGHT, _) => self.navigate(Nav::Back),
                    _ => {}
                },
                PointerEventKind::Axis { vertical, .. } if on_panel => {
                    let steps = if vertical.discrete != 0 {
                        vertical.discrete
                    } else {
                        (vertical.absolute / 10.0) as i32
                    };
                    if steps != 0 {
                        self.navigate(if steps < 0 { Nav::Up } else { Nav::Down });
                    }
                }
                _ => {}
            }
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
        &mut self.seat_state
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
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard(qh, &seat, None) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(err) => log::warn!("no keyboard: {err}"),
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            let cursor_surface = self.compositor.create_surface(qh);
            match self.seat_state.get_pointer_with_theme::<Self, ()>(
                qh,
                &seat,
                self.shm.wl_shm(),
                cursor_surface,
                ThemeSpec::default(),
            ) {
                Ok(pointer) => self.pointer = Some(pointer),
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
            && let Some(keyboard) = self.keyboard.take()
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
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
wayland_client::delegate_noop!(State: WpAlphaModifierV1);
wayland_client::delegate_noop!(State: WpAlphaModifierSurfaceV1);
smithay_client_toolkit::delegate_dispatch2!(State);
