//! What Retrobox's input daemon (`retrobox-inputd`) and its overlay
//! (`retrobox-overlay`) share: where things are, which devices are
//! controllers and keyboards, what EmulationStation says about them, where
//! a game's launch is, and how the two processes find each other.

// An internal library of the two programs, not published: the lints meant
// for a public API (must_use on every getter, an "Errors" section on every
// fallible function) only add noise here.
#![allow(clippy::must_use_candidate, clippy::missing_errors_doc)]

pub mod es_input;
pub mod es_settings;
pub mod held;
pub mod instance;
pub mod launch;
pub mod logging;
pub mod pad_profile;
pub mod paths;
pub mod probe;
