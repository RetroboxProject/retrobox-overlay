//! The game's controllers while the overlay is open.
//!
//! Each controller's evdev device (from session.json) is grabbed, so neither
//! the emulator nor gamepadly's mappers see a press until the overlay closes.
//! Events are read with the Linux gamepad conventions (`BTN_SOUTH`/`BTN_EAST`,
//! d-pad buttons or hat, left stick, `BTN_MODE` for the guide button), which
//! the usual drivers follow (xpad, xone, hid-playstation, hid-nintendo...).

use std::collections::HashMap;
use std::path::Path;

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};

/// What the overlay does with a controller or keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Confirm,
    Back,
    Close,
}

/// Fraction of an axis' half-range past which the stick counts as pushed.
const STICK_THRESHOLD: f32 = 0.6;

/// A grabbed controller.
pub struct Pad {
    pub device: Device,
    swap_confirm: bool,
    /// Axis -> (centre, half range, last direction)
    axes: HashMap<AbsoluteAxisCode, (i32, i32, i32)>,
}

impl Pad {
    /// Open and grab a controller; None (logged) when it cannot be.
    pub fn open(path: &Path, swap_confirm: bool) -> Option<Self> {
        let mut device = match Device::open(path) {
            Ok(device) => device,
            Err(err) => {
                log::warn!("cannot open {}: {err}", path.display());
                return None;
            }
        };
        if let Err(err) = device.grab() {
            // still usable: the game just sees the presses too
            log::warn!("cannot grab {}: {err}", path.display());
        }
        if let Err(err) = device.set_nonblocking(true) {
            log::warn!("cannot make {} non-blocking: {err}", path.display());
            return None;
        }
        let mut axes = HashMap::new();
        if let Ok(infos) = device.get_absinfo() {
            for (axis, info) in infos {
                if [AbsoluteAxisCode::ABS_Y, AbsoluteAxisCode::ABS_HAT0Y].contains(&axis) {
                    let centre = i32::midpoint(info.minimum(), info.maximum());
                    let half = ((info.maximum() - info.minimum()) / 2).max(1);
                    axes.insert(axis, (centre, half, 0));
                }
            }
        }
        log::info!(
            "grabbed {} ({})",
            device.name().unwrap_or("controller"),
            path.display()
        );
        Some(Self {
            device,
            swap_confirm,
            axes,
        })
    }

    /// The navigation the pending events ask for.
    pub fn read(&mut self) -> Vec<Nav> {
        let events: Vec<_> = match self.device.fetch_events() {
            Ok(events) => events.collect(),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Vec::new(),
            Err(err) => {
                log::warn!("controller read error: {err}");
                return Vec::new();
            }
        };
        let mut navs = Vec::new();
        for event in events {
            match event.destructure() {
                EventSummary::Key(_, code, 1) => navs.extend(self.button(code)),
                EventSummary::AbsoluteAxis(_, axis, value) => navs.extend(self.axis(axis, value)),
                _ => {}
            }
        }
        navs
    }

    fn button(&self, code: KeyCode) -> Option<Nav> {
        let (confirm, back) = if self.swap_confirm {
            (KeyCode::BTN_EAST, KeyCode::BTN_SOUTH)
        } else {
            (KeyCode::BTN_SOUTH, KeyCode::BTN_EAST)
        };
        match code {
            KeyCode::BTN_DPAD_UP => Some(Nav::Up),
            KeyCode::BTN_DPAD_DOWN => Some(Nav::Down),
            KeyCode::BTN_MODE => Some(Nav::Close),
            KeyCode::BTN_START => Some(Nav::Confirm),
            code if code == confirm => Some(Nav::Confirm),
            code if code == back => Some(Nav::Back),
            _ => None,
        }
    }

    fn axis(&mut self, axis: AbsoluteAxisCode, value: i32) -> Option<Nav> {
        let (centre, half, last) = self.axes.get_mut(&axis)?;
        let position = (value - *centre) as f32 / *half as f32;
        let direction = if position <= -STICK_THRESHOLD {
            -1
        } else {
            i32::from(position >= STICK_THRESHOLD)
        };
        if direction == *last {
            return None;
        }
        *last = direction;
        match direction {
            -1 => Some(Nav::Up),
            1 => Some(Nav::Down),
            _ => None,
        }
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        if let Err(err) = self.device.ungrab() {
            log::debug!("ungrab: {err}");
        }
    }
}
