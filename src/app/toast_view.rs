//! The toast mode (`--toast`, see `toast`): one card at the top of the
//! screen, on the menu's side (`ui/toast.rs`), which takes neither the
//! keyboard nor the controllers. It fades in with its sound, stays its time
//! and fades out; the overlay then ends. The card is the dim's own surface,
//! sized to it (see `run`).
//!
//! The volume keys' toast (see `volume`) also listens to the presses that
//! come after the one that showed it: each one redraws its bar and makes it
//! stay its time again (back from fading out, if it was).

use std::os::unix::net::UnixDatagram;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{Interest, Mode, PostAction};
use smithay_client_toolkit::shell::wlr_layer::Anchor;

use super::display::{Display, Spec};
use super::{Content, FRAME, Outcome, State, ease_out};
use crate::config::Side;
use crate::sound::{Sound, Sounds};
use crate::toast::Toast;
use crate::ui::toast::ToastCard;
use crate::volume::{self, Level};

/// The screen a card is sized for when the display does not say it.
const SCREEN: (u32, u32) = (1920, 1080);

/// A toast on screen: its card, its sound and how long it stays.
pub(super) struct Shown {
    card: ToastCard,
    sound: Option<Sound>,
    stays: Duration,
    /// What it shows, and the screen it is drawn for (the volume's card is
    /// drawn again as the volume changes).
    toast: Toast,
    screen: (u32, u32),
}

/// The toast's card, drawn for the screen, when one is asked for: it only
/// shows (no keyboard, no controllers).
pub(super) fn card(content: &mut Content, display: &Display) -> Result<Option<Shown>> {
    let Some(toast) = content.toast.take() else {
        return Ok(None);
    };
    content.take_keyboard = false;
    let screen = display.screen_size().unwrap_or(SCREEN);
    let card = ToastCard::new(&content.theme, screen, &mut content.fonts, &toast)
        .context("cannot draw the toast")?;
    Ok(Some(Shown {
        card,
        sound: toast.sound,
        stays: toast.shown,
        toast,
        screen,
    }))
}

/// The menu's sounds, at `volume`; none for a silent toast, which then
/// starts no player (no stream next to the game's).
pub(super) fn sounds(shown: Option<&Shown>, volume: u8) -> Sounds {
    let silent = shown.is_some_and(|shown| shown.sound.is_none());
    Sounds::of_menu(if silent { 0 } else { volume })
}

/// Where the card goes: at the top of the screen, on the menu's side.
pub(super) fn spec(shown: &Shown, side: Side) -> Spec {
    let card = &shown.card;
    let margin = i32::try_from(card.margin).unwrap_or(0);
    let (edge, margins) = match side {
        Side::Right => (Anchor::RIGHT, [margin, margin, 0, 0]),
        Side::Left => (Anchor::LEFT, [margin, 0, 0, margin]),
    };
    Spec {
        anchor: Anchor::TOP | edge,
        size: card.size(),
        margin: margins,
        keyboard: false,
        fade: true,
    }
}

impl State {
    /// The card is on screen: the sound, and its fades.
    pub(super) fn start_toast(&mut self) {
        if let Some(sound) = self.toast.as_ref().and_then(|shown| shown.sound) {
            self.sounds.play(sound);
        }
        self.started = Some(Instant::now());
        if !self.toast_timer() {
            self.outcome = Some(Outcome::Back);
        }
        if let Some(socket) = self.content.toast_feed.take() {
            self.listen_to_presses(socket);
        }
    }

    /// Start the steps of the card's life; false when it cannot.
    fn toast_timer(&self) -> bool {
        self.handle
            .insert_source(Timer::immediate(), |_, &mut (), state| state.toast_frame())
            .is_ok()
    }

    /// The volume keys' presses after the first: their volume, as it comes.
    fn listen_to_presses(&self, socket: UnixDatagram) {
        let source = Generic::new(socket, Interest::READ, Mode::Level);
        let listening = self.handle.insert_source(source, |_, socket, state| {
            if let Some(level) = volume::latest(socket) {
                state.toast_volume(level);
            }
            Ok(PostAction::Continue)
        });
        if let Err(err) = listening {
            log::warn!("cannot listen to the volume keys: {err}");
        }
    }

    /// A press changed the volume: the bar shows it, and the card stays
    /// from now (at full opacity again, if it was fading out; brought back,
    /// if it had just gone).
    fn toast_volume(&mut self, level: Level) {
        let fade = Duration::from_millis(self.content.config.animation_ms);
        let Some(shown) = self.toast.as_mut() else {
            return;
        };
        if shown.toast.level != Some(level) {
            shown.toast.set_level(level);
            let card = ToastCard::new(
                &self.content.theme,
                shown.screen,
                &mut self.content.fonts,
                &shown.toast,
            );
            if let Some(card) = card {
                shown.card = card;
                self.draw_toast();
            }
        }
        let now = Instant::now();
        let fading_in = self
            .started
            .is_some_and(|started| now.duration_since(started) < fade);
        if !fading_in {
            self.started = Some(now.checked_sub(fade).unwrap_or(now));
        }
        // it had ended: its steps start again
        if self.outcome == Some(Outcome::Back) {
            self.outcome = None;
            if !self.toast_timer() {
                self.outcome = Some(Outcome::Back);
            }
        }
    }

    pub(super) fn draw_toast(&mut self) {
        if let Some(shown) = &self.toast {
            self.dim.present(&mut self.display, &shown.card.pixmap);
        }
    }

    /// One step of the card's life: its opacity, and when the next step is.
    fn toast_frame(&mut self) -> TimeoutAction {
        let (Some(started), Some(shown)) = (self.started, &self.toast) else {
            return TimeoutAction::Drop;
        };
        let stays = shown.stays;
        let fade = Duration::from_millis(self.content.config.animation_ms);
        let elapsed = started.elapsed();
        let fade_out = fade + stays;
        if elapsed >= fade_out + fade {
            self.outcome = Some(Outcome::Back);
            return TimeoutAction::Drop;
        }
        // (no fade, with animations off: the middle step only)
        let (opacity, next) = if elapsed < fade {
            (ease_out(elapsed.as_secs_f32() / fade.as_secs_f32()), FRAME)
        } else if elapsed < fade_out {
            (1.0, fade_out.saturating_sub(elapsed))
        } else {
            let out = elapsed.saturating_sub(fade_out).as_secs_f32() / fade.as_secs_f32();
            (1.0 - ease_out(out), FRAME)
        };
        self.dim.set_opacity(&self.display, opacity);
        self.dim.commit();
        TimeoutAction::ToDuration(next)
    }
}
