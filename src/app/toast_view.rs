//! The toast mode (`--toast`, see `toast`): one card at the top of the
//! screen, on the menu's side (`ui/toast.rs`), which takes neither the
//! keyboard nor the controllers. It fades in with its sound, stays its time
//! and fades out; the overlay then ends. The card is the dim's own surface,
//! sized to it (see `run`).

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::shell::wlr_layer::Anchor;

use super::display::{Display, Spec};
use super::{Content, FRAME, Outcome, State, ease_out};
use crate::config::Side;
use crate::sound::Sound;
use crate::ui::toast::ToastCard;

/// The screen a card is sized for when the display does not say it.
const SCREEN: (u32, u32) = (1920, 1080);

/// A toast on screen: its card, its sound and how long it stays.
pub(super) struct Shown {
    card: ToastCard,
    sound: Option<Sound>,
    stays: Duration,
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
    }))
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
        let started = self
            .handle
            .insert_source(Timer::immediate(), |_, &mut (), state| state.toast_frame());
        if started.is_err() {
            self.outcome = Some(Outcome::Back);
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
