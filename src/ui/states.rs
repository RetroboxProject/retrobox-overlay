//! The save and load pages: the slots with their previews and dates.

use chrono::{DateTime, Local};
use tiny_skia::Pixmap;

use super::panel::{SlotPage, SlotView};
use crate::i18n::Translator;
use crate::savestates::{SLOTS, SaveStates, StateMode};

/// Everything the pages need, translated beforehand.
pub struct StatesMenu {
    states: SaveStates,
    save_title: String,
    load_title: String,
    /// "Slot N" for each of `SLOTS`.
    slot_labels: Vec<String>,
    empty: String,
    date_format: String,
    locale: chrono::Locale,
}

impl StatesMenu {
    pub fn new(states: SaveStates, translator: &Translator) -> Self {
        Self {
            states,
            save_title: translator.text("menu-save-state"),
            load_title: translator.text("menu-load-state"),
            slot_labels: SLOTS
                .iter()
                .map(|number| translator.text_with("slot-label", "number", i64::from(*number)))
                .collect(),
            empty: translator.text("slot-empty"),
            date_format: translator.text("slot-date-format"),
            locale: translator.locale(),
        }
    }

    /// The page for saving or loading, with what each slot holds now.
    pub fn page(&self, mode: StateMode) -> SlotPage {
        let slots = self
            .states
            .slots()
            .into_iter()
            .zip(&self.slot_labels)
            .map(|(slot, label)| {
                let detail = slot.saved.map_or_else(
                    || self.empty.clone(),
                    |saved| {
                        DateTime::<Local>::from(saved)
                            .format_localized(&self.date_format, self.locale)
                            .to_string()
                    },
                );
                let preview = slot.preview.as_ref().and_then(|path| {
                    Pixmap::load_png(path)
                        .inspect_err(|err| log::warn!("cannot read {}: {err}", path.display()))
                        .ok()
                });
                SlotView {
                    number: slot.number,
                    label: label.clone(),
                    detail,
                    preview,
                    enabled: mode == StateMode::Save || slot.saved.is_some(),
                }
            })
            .collect();
        SlotPage {
            mode,
            title: match mode {
                StateMode::Save => self.save_title.to_uppercase(),
                StateMode::Load => self.load_title.to_uppercase(),
            },
            slots,
        }
    }
}
