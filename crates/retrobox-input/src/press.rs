//! What counts as a normal press of a button (a tap) everywhere in
//! Retrobox's input: let go within `TAP_MAX` of going down. Held longer, it
//! is a hold, and what a tap does is not done: the dedicated hotkey's tap
//! (the overlay), the profiles' taps (the mapper), the overlay's Quit (held,
//! it saves first).

use std::time::Duration;

/// The longest a press lasts and still is a tap.
pub const TAP_MAX: Duration = Duration::from_millis(300);
