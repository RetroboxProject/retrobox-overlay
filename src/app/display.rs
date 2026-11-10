//! Where the overlay shows itself: a Wayland compositor with layer-shell
//! (KWin, wlroots compositors such as labwc), or an X11 server (KDE Plasma on
//! X11, for instance where the GPU driver has no good Wayland support).
//!
//! The rest of the overlay works with [`Piece`]s (a surface of its own,
//! placed like a layer-shell surface: anchored to screen edges, with
//! margins) and [`Sub`]s (a surface inside a piece), whatever shows them:
//!
//! * on Wayland a piece is a layer surface in the overlay layer, a sub a
//!   subsurface; the compositor fades them (`wp_alpha_modifier`) and may
//!   crop and scale the manual's page (`wp_viewporter`);
//! * on X11 a piece is an override-redirect window with an ARGB visual
//!   (translucent where a compositing manager runs, opaque otherwise), a sub
//!   a child window; the fade is `_NET_WM_WINDOW_OPACITY`, the windows are
//!   kept out of KWin's window animations as on-screen displays, raised
//!   again whenever the window manager restacks another window (xfwm4 puts
//!   a fullscreen game back on top when it gets the focus back), and the
//!   overlay grabs the keyboard, so that a window manager handing the focus
//!   back to the game does not take the menu's keys away. Closing gives the
//!   focus back to the window that had it (xfwm4 leaves it nowhere, and the
//!   game then gets no input and drops below the panel). Pixels go to the
//!   server with `PutImage`.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Context as _, Result, anyhow};
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::{RegistryState, SimpleGlobal};
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::seat::pointer::ThemedPointer;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface,
};
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::subcompositor::SubcompositorState;
use tiny_skia::Pixmap;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::protocol::{wl_keyboard, wl_shm, wl_surface::WlSurface};
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1;
use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_v1::WpAlphaModifierV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    self, AtomEnum, ChangeWindowAttributesAux, ColormapAlloc, ConfigureWindowAux,
    ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask, GrabMode, GrabStatus, ImageFormat,
    InputFocus, PropMode, StackMode, VisualClass, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::State;
use crate::ui::to_argb8888;

/// The layer surfaces' namespace on Wayland, which compositors may match in
/// their rules.
const NAMESPACE: &str = "retrobox-overlay";
/// KWin animates layer surfaces whose namespace it does not know as normal
/// windows (its Scale, Fade or Glide effects, on top of the overlay's own
/// slide and fade); an on-screen display it leaves alone, and it changes
/// nothing else for a layer surface.
const KWIN_NAMESPACE: &str = "on-screen-display";
/// The X11 windows' class and name.
const WINDOW_CLASS: &[u8] = b"retrobox-overlay\0retrobox-overlay\0";

/// The namespace for this desktop.
fn namespace() -> &'static str {
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktops| {
        desktops
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("KDE"))
    });
    if kde { KWIN_NAMESPACE } else { NAMESPACE }
}

/// A surface, whichever the windowing system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SurfaceId {
    Wayland(WlSurface),
    X11(xproto::Window),
}

/// Where a piece goes, as layer-shell says it: the edges it is anchored to,
/// its size (0: as long as the screen, between two opposite anchors) and
/// its margins (top, right, bottom, left).
#[derive(Debug, Clone, Copy)]
pub(super) struct Spec {
    pub anchor: Anchor,
    pub size: (u32, u32),
    pub margin: [i32; 4],
    /// Whether it takes the keyboard (the panel).
    pub keyboard: bool,
    /// Whether it fades in.
    pub fade: bool,
}

/// What is on Wayland's side.
#[allow(clippy::struct_field_names, reason = "SCTK's usual names")]
pub(super) struct Wl {
    pub conn: Connection,
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub compositor: CompositorState,
    pub layer_shell: LayerShell,
    pub shm: Shm,
    pub alpha: Option<WpAlphaModifierV1>,
    pub subcompositor: Option<SubcompositorState>,
    pub viewporter: Option<WpViewporter>,
    pub pool: SlotPool,
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub pointer: Option<ThemedPointer>,
    pub touch: Option<wayland_client::protocol::wl_touch::WlTouch>,
    pub qh: QueueHandle<State>,
}

/// What is on X11's side.
pub(super) struct X11 {
    pub conn: Rc<RustConnection>,
    root: xproto::Window,
    /// The monitor shown on (the primary one): position and size.
    monitor: (i16, i16, u32, u32),
    depth: u8,
    visual: xproto::Visualid,
    colormap: xproto::Colormap,
    opacity: xproto::Atom,
    bypass_compositor: xproto::Atom,
    window_type: xproto::Atom,
    window_types: [xproto::Atom; 2],
    /// Sizes to tell, as a Wayland compositor's configures would.
    pub configures: Vec<(xproto::Window, (u32, u32))>,
    /// The overlay's top-level windows, bottom to top.
    tops: Rc<RefCell<Vec<xproto::Window>>>,
    /// The window with the keyboard focus before the overlay (the game's).
    previous_focus: Option<xproto::Window>,
    active_window: xproto::Atom,
}

pub(super) enum Display {
    Wayland(Box<Wl>),
    X11(X11),
}

impl Display {
    /// Wayland's side (only called from its handlers).
    pub fn wl(&mut self) -> &mut Wl {
        match self {
            Self::Wayland(wl) => wl,
            Self::X11(_) => unreachable!("a Wayland handler on X11"),
        }
    }

    pub fn x11(&mut self) -> Option<&mut X11> {
        match self {
            Self::X11(x11) => Some(x11),
            Self::Wayland(_) => None,
        }
    }

    /// Send what is pending.
    pub fn flush(&self) {
        let flushed = match self {
            Self::Wayland(wl) => wl.conn.flush().map_err(|err| err.to_string()),
            Self::X11(x11) => x11.conn.flush().map_err(|err| err.to_string()),
        };
        if let Err(err) = flushed {
            log::warn!("cannot flush the display connection: {err}");
        }
    }

    /// Whether the manual's page can be shown (in a subsurface).
    pub fn has_subsurfaces(&self) -> bool {
        match self {
            Self::Wayland(wl) => wl.subcompositor.is_some(),
            Self::X11(_) => true,
        }
    }

    /// A new piece; its size comes later (Wayland's configure, or the
    /// configures X11 keeps).
    pub fn create_piece(&mut self, spec: Spec) -> Piece {
        let kind = match self {
            Self::Wayland(wl) => {
                let surface = wl.compositor.create_surface(&wl.qh);
                let layer = wl.layer_shell.create_layer_surface(
                    &wl.qh,
                    surface,
                    Layer::Overlay,
                    Some(namespace()),
                    None,
                );
                layer.set_anchor(spec.anchor);
                layer.set_exclusive_zone(-1);
                // The keyboard on demand, not exclusively: the compositor may
                // give it to another window (Alt+Tab, a click), and then the
                // overlay closes.
                layer.set_keyboard_interactivity(if spec.keyboard {
                    KeyboardInteractivity::OnDemand
                } else {
                    KeyboardInteractivity::None
                });
                layer.set_size(spec.size.0, spec.size.1);
                let [top, right, bottom, left] = spec.margin;
                layer.set_margin(top, right, bottom, left);
                layer.commit();
                let fade = spec
                    .fade
                    .then_some(wl.alpha.as_ref())
                    .flatten()
                    .map(|alpha| {
                        let fade = alpha.get_surface(layer.wl_surface(), &wl.qh, ());
                        fade.set_multiplier(0);
                        fade
                    });
                Kind::Layer {
                    layer,
                    buffer: None,
                    fade,
                }
            }
            Self::X11(x11) => Kind::Window(x11.create_window(&spec)),
        };
        Piece {
            size: (0, 0),
            spec,
            kind,
            shown: false,
        }
    }
}

impl X11 {
    /// Connect to the X server, if there is one (`DISPLAY`).
    pub fn connect() -> Result<Self> {
        let (conn, screen) = RustConnection::connect(None).context("no X server")?;
        let setup = conn.setup();
        let root_screen = setup
            .roots
            .get(screen)
            .ok_or_else(|| anyhow!("no X screen {screen}"))?;
        let root = root_screen.root;
        // a 32-bit visual: translucency where a compositing manager runs
        let argb = root_screen
            .allowed_depths
            .iter()
            .filter(|depth| depth.depth == 32)
            .flat_map(|depth| {
                depth
                    .visuals
                    .iter()
                    .map(move |visual| (depth.depth, visual))
            })
            .find(|(_, visual)| visual.class == VisualClass::TRUE_COLOR)
            .map(|(depth, visual)| (depth, visual.visual_id));
        let (depth, visual) = argb.unwrap_or_else(|| {
            log::warn!("no 32-bit visual: the overlay is opaque");
            (root_screen.root_depth, root_screen.root_visual)
        });
        let whole = (
            0,
            0,
            u32::from(root_screen.width_in_pixels),
            u32::from(root_screen.height_in_pixels),
        );
        let colormap = conn.generate_id()?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)?;
        let atom = |name: &str| -> Result<xproto::Atom> {
            Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
        };
        let opacity = atom("_NET_WM_WINDOW_OPACITY")?;
        let bypass_compositor = atom("_NET_WM_BYPASS_COMPOSITOR")?;
        let active_window = atom("_NET_ACTIVE_WINDOW")?;
        let previous_focus = focused_window(&conn, root, active_window);
        let window_type = atom("_NET_WM_WINDOW_TYPE")?;
        let window_types = [
            atom("_KDE_NET_WM_WINDOW_TYPE_ON_SCREEN_DISPLAY")?,
            atom("_NET_WM_WINDOW_TYPE_NOTIFICATION")?,
        ];
        let monitor = primary_monitor(&conn, root).unwrap_or(whole);
        // other windows being mapped or restacked: the overlay goes back on top
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )?;
        log::info!(
            "X11: {}x{} at {},{}{}",
            monitor.2,
            monitor.3,
            monitor.0,
            monitor.1,
            if compositing(&conn, screen) {
                ""
            } else {
                " (no compositing manager: no translucency)"
            }
        );
        Ok(Self {
            conn: Rc::new(conn),
            root,
            monitor,
            depth,
            visual,
            colormap,
            opacity,
            bypass_compositor,
            window_type,
            window_types,
            configures: Vec::new(),
            tops: Rc::default(),
            previous_focus,
            active_window,
        })
    }

    /// Give the keyboard focus back to the window that had it, once the
    /// overlay's windows are gone: directly, and through the window manager
    /// (`_NET_ACTIVE_WINDOW`, as a pager asks, which focus stealing
    /// prevention lets through) so that it raises it again too.
    pub fn restore_focus(&self) {
        let Some(window) = self.previous_focus else {
            return;
        };
        log::debug!("focus back to {window:#x}");
        log_error(
            self.conn
                .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME),
        );
        let event = xproto::ClientMessageEvent::new(
            32,
            window,
            self.active_window,
            [2, x11rb::CURRENT_TIME, 0, 0, 0],
        );
        log_error(self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        ));
    }

    /// Whether a window is one of the overlay's top-level ones.
    pub fn is_top(&self, window: xproto::Window) -> bool {
        self.tops.borrow().contains(&window)
    }

    /// Put the overlay's windows back above every other one, in their order.
    pub fn raise(&self) {
        for &window in self.tops.borrow().iter() {
            log_error(self.conn.configure_window(
                window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            ));
        }
    }

    /// Take the whole keyboard for a window, so that keys reach the overlay
    /// whichever window has the focus. False while another client (a
    /// window manager shortcut being held) has it.
    pub fn grab_keyboard(&self, window: xproto::Window) -> bool {
        let status = self
            .conn
            .grab_keyboard(
                true,
                window,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.status);
        status == Some(GrabStatus::SUCCESS)
    }

    /// Where a piece goes on the monitor: x, y, width, height.
    fn geometry(&self, spec: &Spec) -> (i32, i32, u32, u32) {
        geometry_on(self.monitor, spec)
    }

    fn create_window(&mut self, spec: &Spec) -> Window {
        let (x, y, width, height) = self.geometry(spec);
        let mut window = self.new_window(self.root, (x, y, width, height));
        if let Some(window) = &mut window {
            self.tops.borrow_mut().push(window.id);
            window.tops = Some(Rc::clone(&self.tops));
            let conn = &self.conn;
            let types: Vec<u32> = self.window_types.to_vec();
            log_error(conn.change_property32(
                PropMode::REPLACE,
                window.id,
                self.window_type,
                AtomEnum::ATOM,
                &types,
            ));
            // 2: never shown around the compositor. xfwm4 does that to an
            // opaque fullscreen override-redirect window ("display fullscreen
            // overlay windows directly"), which the dim is without a 32-bit
            // visual, and then nothing composited shows above it.
            log_error(conn.change_property32(
                PropMode::REPLACE,
                window.id,
                self.bypass_compositor,
                AtomEnum::CARDINAL,
                &[2],
            ));
            log_error(conn.change_property8(
                PropMode::REPLACE,
                window.id,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                WINDOW_CLASS,
            ));
            if spec.fade {
                log_error(conn.change_property32(
                    PropMode::REPLACE,
                    window.id,
                    self.opacity,
                    AtomEnum::CARDINAL,
                    &[0],
                ));
            }
            log_error(conn.map_window(window.id));
            if spec.keyboard {
                // override-redirect windows take the focus themselves
                log_error(conn.set_input_focus(InputFocus::PARENT, window.id, x11rb::CURRENT_TIME));
            }
            self.configures.push((window.id, (width, height)));
        }
        window.unwrap_or_default()
    }

    /// An override-redirect window (a child one inside `parent`).
    fn new_window(
        &self,
        parent: xproto::Window,
        (x, y, width, height): (i32, i32, u32, u32),
    ) -> Option<Window> {
        let conn = &self.conn;
        let id = conn.generate_id().ok()?;
        let events = EventMask::EXPOSURE
            | EventMask::KEY_PRESS
            | EventMask::KEY_RELEASE
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::ENTER_WINDOW
            | EventMask::LEAVE_WINDOW
            | EventMask::FOCUS_CHANGE;
        let attributes = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(self.colormap)
            .override_redirect(1)
            .event_mask(events);
        log_error(conn.create_window(
            self.depth,
            id,
            parent,
            clamp_i16(x),
            clamp_i16(y),
            clamp_u16(width),
            clamp_u16(height),
            0,
            WindowClass::INPUT_OUTPUT,
            self.visual,
            &attributes,
        ))?;
        let gc = conn.generate_id().ok()?;
        log_error(conn.create_gc(gc, id, &CreateGCAux::new()))?;
        Some(Window {
            conn: Some(Rc::clone(conn)),
            id,
            gc,
            depth: self.depth,
            size: (width, height),
            pixels: Vec::new(),
            tops: None,
        })
    }
}

/// Where a piece goes on a monitor (x, y, width, height), as layer-shell
/// places a surface.
fn geometry_on(monitor: (i16, i16, u32, u32), spec: &Spec) -> (i32, i32, u32, u32) {
    let (origin_x, origin_y, width, height) = monitor;
    let [top, right, bottom, left] = spec.margin;
    let axis = |low: bool, high: bool, size: u32, total: u32, margins: (i32, i32)| {
        let total = i64::from(total);
        if size == 0 {
            let length = (total - i64::from(margins.0) - i64::from(margins.1)).max(1);
            return (margins.0, u32::try_from(length).unwrap_or(1));
        }
        let start = match (low, high) {
            (true, false) => i64::from(margins.0),
            (false, true) => total - i64::from(size) - i64::from(margins.1),
            _ => (total - i64::from(size)) / 2,
        };
        (i32::try_from(start).unwrap_or(0), size)
    };
    let (x, w) = axis(
        spec.anchor.contains(Anchor::LEFT),
        spec.anchor.contains(Anchor::RIGHT),
        spec.size.0,
        width,
        (left, right),
    );
    let (y, h) = axis(
        spec.anchor.contains(Anchor::TOP),
        spec.anchor.contains(Anchor::BOTTOM),
        spec.size.1,
        height,
        (top, bottom),
    );
    (x + i32::from(origin_x), y + i32::from(origin_y), w, h)
}

/// The primary monitor (`RandR`), else the first one.
fn primary_monitor(conn: &RustConnection, root: xproto::Window) -> Option<(i16, i16, u32, u32)> {
    let monitors = conn.randr_get_monitors(root, true).ok()?.reply().ok()?;
    let monitor = monitors
        .monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or_else(|| monitors.monitors.first())?;
    Some((
        monitor.x,
        monitor.y,
        u32::from(monitor.width),
        u32::from(monitor.height),
    ))
}

/// Whether a compositing manager runs (it owns `_NET_WM_CM_Sn`).
/// The window with the keyboard focus: the focused one, else the window
/// manager's active one (when the focus is on the root or nowhere).
fn focused_window(
    conn: &RustConnection,
    root: xproto::Window,
    active_window: xproto::Atom,
) -> Option<xproto::Window> {
    let focus = conn.get_input_focus().ok()?.reply().ok()?.focus;
    // 0 is None, 1 PointerRoot
    if focus > 1 && focus != root {
        return Some(focus);
    }
    conn.get_property(false, root, active_window, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
        .filter(|&window| window != 0)
}

fn compositing(conn: &RustConnection, screen: usize) -> bool {
    let name = format!("_NET_WM_CM_S{screen}");
    conn.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|atom| conn.get_selection_owner(atom.atom).ok())
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|owner| owner.owner != x11rb::NONE)
}

fn log_error<T, E: std::fmt::Display>(result: Result<T, E>) -> Option<T> {
    result
        .inspect_err(|err| log::warn!("X11 request failed: {err}"))
        .ok()
}

fn clamp_i16(value: i32) -> i16 {
    i16::try_from(value.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or_default()
}

fn clamp_u16(value: u32) -> u16 {
    u16::try_from(value.clamp(1, u32::from(u16::MAX))).unwrap_or(1)
}

/// An X11 window and the pixels shown in it (kept for exposures).
#[derive(Default)]
pub(super) struct Window {
    conn: Option<Rc<RustConnection>>,
    id: xproto::Window,
    gc: xproto::Gcontext,
    depth: u8,
    size: (u32, u32),
    pixels: Vec<u8>,
    /// The list of top-level windows it is in, if it is one.
    tops: Option<Rc<RefCell<Vec<xproto::Window>>>>,
}

impl Window {
    /// Show ARGB8888 pixels (B, G, R, A in memory; the X server's order on
    /// little-endian machines), resizing the window to them.
    fn show(&mut self, width: u32, height: u32, fill: impl FnOnce(&mut [u8])) {
        let mut pixels = vec![0; width as usize * height as usize * 4];
        fill(&mut pixels);
        if self.size != (width, height)
            && let Some(conn) = &self.conn
        {
            log_error(
                conn.configure_window(
                    self.id,
                    &ConfigureWindowAux::new()
                        .width(u32::from(clamp_u16(width)))
                        .height(u32::from(clamp_u16(height))),
                ),
            );
            self.size = (width, height);
        }
        self.pixels = pixels;
        self.repaint();
    }

    /// Put the pixels on the window again (after an exposure), in strips
    /// small enough for one request each.
    pub fn repaint(&self) {
        let Some(conn) = &self.conn else {
            return;
        };
        let (width, height) = self.size;
        let row = width as usize * 4;
        if row == 0 || self.pixels.len() < row * height as usize {
            return;
        }
        let rows_per_request = ((conn.maximum_request_bytes().saturating_sub(64)) / row).max(1);
        for (index, strip) in self.pixels.chunks(row * rows_per_request).enumerate() {
            let top = index * rows_per_request;
            let rows = strip.len() / row;
            log_error(conn.put_image(
                ImageFormat::Z_PIXMAP,
                self.id,
                self.gc,
                clamp_u16(width),
                u16::try_from(rows).unwrap_or(u16::MAX),
                0,
                i16::try_from(top).unwrap_or(i16::MAX),
                0,
                self.depth,
                strip,
            ));
        }
    }

    fn move_to(&self, x: i32, y: i32) {
        if let Some(conn) = &self.conn {
            log_error(conn.configure_window(self.id, &ConfigureWindowAux::new().x(x).y(y)));
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if let Some(tops) = &self.tops {
            tops.borrow_mut().retain(|&window| window != self.id);
        }
        if let Some(conn) = &self.conn {
            log_error(conn.free_gc(self.gc));
            log_error(conn.destroy_window(self.id));
        }
    }
}

enum Kind {
    Layer {
        layer: LayerSurface,
        buffer: Option<Buffer>,
        fade: Option<WpAlphaModifierSurfaceV1>,
    },
    Window(Window),
}

/// A surface of the overlay's own, placed like a layer-shell surface.
pub(super) struct Piece {
    /// Its size, once the display has said it.
    pub size: (u32, u32),
    spec: Spec,
    kind: Kind,
    /// Whether something has been shown on it.
    shown: bool,
}

impl Piece {
    pub fn is(&self, surface: &SurfaceId) -> bool {
        match (&self.kind, surface) {
            (Kind::Layer { layer, .. }, SurfaceId::Wayland(surface)) => {
                layer.wl_surface() == surface
            }
            (Kind::Window(window), SurfaceId::X11(id)) => window.id == *id,
            _ => false,
        }
    }

    /// Its X11 window, on X11.
    pub const fn x11_window(&self) -> Option<xproto::Window> {
        match &self.kind {
            Kind::Window(window) => Some(window.id),
            Kind::Layer { .. } => None,
        }
    }

    pub const fn has_content(&self) -> bool {
        self.shown
    }

    /// Set the opacity (0..1), for the next commit.
    pub fn set_opacity(&self, display: &Display, opacity: f32) {
        let opacity = f64::from(opacity.clamp(0.0, 1.0));
        match (&self.kind, display) {
            (
                Kind::Layer {
                    fade: Some(fade), ..
                },
                _,
            ) => {
                fade.set_multiplier((opacity * f64::from(u32::MAX)) as u32);
            }
            (Kind::Window(window), Display::X11(x11)) if self.spec.fade => {
                log_error(x11.conn.change_property32(
                    PropMode::REPLACE,
                    window.id,
                    x11.opacity,
                    AtomEnum::CARDINAL,
                    &[(opacity * f64::from(u32::MAX)) as u32],
                ));
            }
            _ => {}
        }
    }

    /// Change the margins (the panel slides in by its own).
    pub fn set_margin(&mut self, display: &Display, margin: [i32; 4]) {
        self.spec.margin = margin;
        match (&self.kind, display) {
            (Kind::Layer { layer, .. }, _) => {
                let [top, right, bottom, left] = margin;
                layer.set_margin(top, right, bottom, left);
            }
            (Kind::Window(window), Display::X11(x11)) => {
                let (x, y, _, _) = x11.geometry(&self.spec);
                window.move_to(x, y);
            }
            _ => {}
        }
    }

    /// Apply what changed (Wayland's commit; X11 applies at once).
    pub fn commit(&self) {
        if let Kind::Layer { layer, .. } = &self.kind {
            layer.commit();
        }
    }

    /// Show a pixmap on it.
    pub fn present(&mut self, display: &mut Display, pixmap: &Pixmap) {
        let (width, height) = (pixmap.width(), pixmap.height());
        self.show(display, width, height, |canvas| to_argb8888(pixmap, canvas));
    }

    /// Fill it with one ARGB8888 pixel (B, G, R, A in memory), at its size.
    pub fn fill(&mut self, display: &mut Display, pixel: [u8; 4]) {
        let (width, height) = (self.size.0.max(1), self.size.1.max(1));
        self.show(display, width, height, |canvas| {
            for target in canvas.as_chunks_mut::<4>().0 {
                *target = pixel;
            }
        });
    }

    fn show(
        &mut self,
        display: &mut Display,
        width: u32,
        height: u32,
        fill: impl FnOnce(&mut [u8]),
    ) {
        match (&mut self.kind, display) {
            (Kind::Layer { layer, buffer, .. }, Display::Wayland(wl)) => {
                if let Some(attached) =
                    attach(&mut wl.pool, layer.wl_surface(), width, height, fill)
                {
                    layer.commit();
                    *buffer = Some(attached);
                    self.shown = true;
                }
            }
            (Kind::Window(window), Display::X11(_)) => {
                window.show(width, height, fill);
                self.shown = true;
            }
            _ => {}
        }
    }

    /// Paint an X11 window again after an exposure.
    pub fn expose(&self, id: xproto::Window) -> bool {
        match &self.kind {
            Kind::Window(window) if window.id == id => {
                window.repaint();
                true
            }
            _ => false,
        }
    }
}

/// A buffer of this size, filled and attached to a surface.
fn attach(
    pool: &mut SlotPool,
    surface: &WlSurface,
    width: u32,
    height: u32,
    fill: impl FnOnce(&mut [u8]),
) -> Option<Buffer> {
    let (width, height) = (width as i32, height as i32);
    let Ok((buffer, canvas)) =
        pool.create_buffer(width, height, width * 4, wl_shm::Format::Argb8888)
    else {
        log::warn!("cannot allocate a {width}x{height} buffer");
        return None;
    };
    fill(canvas);
    buffer.attach_to(surface).ok()?;
    surface.damage_buffer(0, 0, width, height);
    Some(buffer)
}

enum SubKind {
    Wayland {
        subsurface: WlSubsurface,
        surface: WlSurface,
        viewport: Option<WpViewport>,
        fade: Option<WpAlphaModifierSurfaceV1>,
        buffer: Option<Buffer>,
    },
    X11 {
        window: Window,
        mapped: bool,
    },
}

/// A surface inside a piece (the help row, the manual's page).
pub(super) struct Sub {
    kind: SubKind,
    pub position: (i32, i32),
}

impl Sub {
    /// A sub of `parent`; `viewport`: cropped and scaled by the compositor
    /// when it can (Wayland's `wp_viewporter`); `fade`: faded with the rest.
    pub fn new(display: &mut Display, parent: &Piece, viewport: bool, fade: bool) -> Option<Self> {
        let kind = match (display, &parent.kind) {
            (Display::Wayland(wl), Kind::Layer { layer, .. }) => {
                let subcompositor = wl.subcompositor.as_ref()?;
                let (subsurface, surface) =
                    subcompositor.create_subsurface(layer.wl_surface().clone(), &wl.qh);
                let viewport = viewport
                    .then_some(wl.viewporter.as_ref())
                    .flatten()
                    .map(|viewporter| viewporter.get_viewport(&surface, &wl.qh, ()));
                let fade = fade.then_some(wl.alpha.as_ref()).flatten().map(|alpha| {
                    let fade = alpha.get_surface(&surface, &wl.qh, ());
                    fade.set_multiplier(0);
                    fade
                });
                SubKind::Wayland {
                    subsurface,
                    surface,
                    viewport,
                    fade,
                    buffer: None,
                }
            }
            (Display::X11(x11), Kind::Window(window)) => SubKind::X11 {
                window: x11.new_window(window.id, (0, 0, 1, 1))?,
                mapped: false,
            },
            _ => return None,
        };
        Some(Self {
            kind,
            position: (0, 0),
        })
    }

    pub fn is(&self, surface: &SurfaceId) -> bool {
        match (&self.kind, surface) {
            (SubKind::Wayland { surface: own, .. }, SurfaceId::Wayland(surface)) => own == surface,
            (SubKind::X11 { window, .. }, SurfaceId::X11(id)) => window.id == *id,
            _ => false,
        }
    }

    /// The compositor's crop and scale, when it does them.
    pub fn viewport(&self) -> Option<WpViewport> {
        match &self.kind {
            SubKind::Wayland { viewport, .. } => viewport.clone(),
            SubKind::X11 { .. } => None,
        }
    }

    pub fn set_opacity(&self, opacity: f32) {
        if let SubKind::Wayland {
            fade: Some(fade), ..
        } = &self.kind
        {
            fade.set_multiplier((f64::from(opacity.clamp(0.0, 1.0)) * f64::from(u32::MAX)) as u32);
        }
    }

    pub fn move_to(&mut self, x: i32, y: i32) {
        self.position = (x, y);
        match &self.kind {
            SubKind::Wayland { subsurface, .. } => subsurface.set_position(x, y),
            SubKind::X11 { window, .. } => window.move_to(x, y),
        }
    }

    /// Show a pixmap, at its size.
    pub fn show_pixmap(&mut self, display: &mut Display, pixmap: &Pixmap) {
        let (width, height) = (pixmap.width(), pixmap.height());
        self.show(display, width, height, |canvas| to_argb8888(pixmap, canvas));
    }

    /// Show ARGB8888 pixels (rows of `width * 4` bytes).
    pub fn show_pixels(&mut self, display: &mut Display, width: u32, height: u32, pixels: &[u8]) {
        self.show(display, width, height, |canvas| {
            let length = canvas.len().min(pixels.len());
            canvas[..length].copy_from_slice(&pixels[..length]);
        });
    }

    fn show(
        &mut self,
        display: &mut Display,
        width: u32,
        height: u32,
        fill: impl FnOnce(&mut [u8]),
    ) {
        match (&mut self.kind, display) {
            (
                SubKind::Wayland {
                    surface, buffer, ..
                },
                Display::Wayland(wl),
            ) => {
                if let Some(attached) = attach(&mut wl.pool, surface, width, height, fill) {
                    *buffer = Some(attached);
                }
            }
            (SubKind::X11 { window, mapped }, Display::X11(x11)) => {
                if !*mapped {
                    log_error(x11.conn.map_window(window.id));
                    *mapped = true;
                }
                window.show(width, height, fill);
            }
            _ => {}
        }
    }

    pub fn hide(&mut self) {
        match &mut self.kind {
            SubKind::Wayland {
                surface, buffer, ..
            } => {
                surface.attach(None, 0, 0);
                *buffer = None;
            }
            SubKind::X11 { window, mapped } => {
                if let Some(conn) = &window.conn {
                    log_error(conn.unmap_window(window.id));
                }
                *mapped = false;
            }
        }
    }

    /// Apply what changed (on Wayland, with the parent's commit after).
    pub fn commit(&self) {
        if let SubKind::Wayland { surface, .. } = &self.kind {
            surface.commit();
        }
    }

    /// Paint an X11 window again after an exposure.
    pub fn expose(&self, id: xproto::Window) -> bool {
        match &self.kind {
            SubKind::X11 { window, .. } if window.id == id => {
                window.repaint();
                true
            }
            _ => false,
        }
    }
}

impl Drop for Sub {
    fn drop(&mut self) {
        if let SubKind::Wayland {
            subsurface,
            surface,
            viewport,
            fade,
            ..
        } = &self.kind
        {
            if let Some(viewport) = viewport {
                viewport.destroy();
            }
            if let Some(fade) = fade {
                fade.destroy();
            }
            subsurface.destroy();
            surface.destroy();
        }
    }
}

/// Bind what the overlay uses on Wayland.
pub(super) fn bind_wayland(
    conn: Connection,
    globals: &GlobalList,
    qh: QueueHandle<State>,
    compositor_scaling: bool,
) -> Result<Wl> {
    let compositor = CompositorState::bind(globals, &qh).context("wl_compositor missing")?;
    let layer_shell = LayerShell::bind(globals, &qh).map_err(|_| {
        anyhow!(
            "the compositor has no layer-shell (KWin and wlroots compositors such as labwc have it)"
        )
    })?;
    let shm = Shm::bind(globals, &qh).context("wl_shm missing")?;
    let pool = SlotPool::new(4 * 1024 * 1024, &shm).context("cannot create the shm pool")?;
    let alpha = SimpleGlobal::<WpAlphaModifierV1, 1>::bind(globals, &qh)
        .ok()
        .and_then(|global| global.get().ok().cloned());
    let subcompositor =
        SubcompositorState::bind(compositor.wl_compositor().clone(), globals, &qh).ok();
    let viewporter = SimpleGlobal::<WpViewporter, 1>::bind(globals, &qh)
        .ok()
        .and_then(|global| global.get().ok().cloned())
        .filter(|_| compositor_scaling);
    if viewporter.is_none() {
        log::info!("the manual is scaled by the overlay (no wp_viewporter, or the setting)");
    }
    Ok(Wl {
        conn,
        registry_state: RegistryState::new(globals),
        seat_state: SeatState::new(globals, &qh),
        output_state: OutputState::new(globals, &qh),
        compositor,
        layer_shell,
        shm,
        alpha,
        subcompositor,
        viewporter,
        pool,
        keyboard: None,
        pointer: None,
        touch: None,
        qh,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x11_geometry(spec: &Spec) -> (i32, i32, u32, u32) {
        geometry_on((100, 0, 1920, 1080), spec)
    }

    #[test]
    fn places_like_layer_shell() {
        let full = Spec {
            anchor: Anchor::all(),
            size: (0, 0),
            margin: [0; 4],
            keyboard: false,
            fade: true,
        };
        assert_eq!(x11_geometry(&full), (100, 0, 1920, 1080));
        let panel = Spec {
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::RIGHT,
            size: (640, 0),
            margin: [0, -640, 0, 0],
            ..full
        };
        // hidden past the right edge, then slid in
        assert_eq!(x11_geometry(&panel), (100 + 1920, 0, 640, 1080));
        let clock = Spec {
            anchor: Anchor::LEFT | Anchor::TOP,
            size: (1280, 200),
            margin: [48, 0, 0, 0],
            ..full
        };
        assert_eq!(x11_geometry(&clock), (100, 48, 1280, 200));
        let centred = Spec {
            anchor: Anchor::LEFT,
            ..clock
        };
        assert_eq!(x11_geometry(&centred), (100, 440, 1280, 200));
    }
}
