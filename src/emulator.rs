//! Talking to the emulator: its pause, and the actions the menu offers.
//!
//! session.json says how the running emulator performs each standard hotkey
//! action (Retrobox's `hotkeys.py`): a key chord, a command, or a combination
//! the emulator reads from the controller itself, which may come with the
//! emulator's own keyboard shortcut for the same thing. The overlay presses
//! the chords on a virtual keyboard of its own (uinput) and runs the commands.
//!
//! A chord only reaches an emulator that reads the keyboard through the
//! compositor while that emulator has the keyboard focus, so it is pressed
//! before the menu takes the focus (the pause) or after the menu has given
//! it back (everything else). Emulators that read the keyboards directly
//! (RetroArch's udev driver) get it either way.

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, KeyCode, KeyEvent};

use crate::session::{Binding, Session};

/// Name of the virtual keyboard (visible in `libinput list-devices`).
const KEYBOARD_NAME: &str = "retrobox-overlay keyboard";
/// How long a chord stays down: a few frames, so emulators that poll the
/// keyboard once per frame (or more slowly while paused) see it.
const HOLD: Duration = Duration::from_millis(60);
/// After the overlay closed, for the compositor to give the keyboard focus
/// back to the game and repaint it (screenshots).
const FOCUS_RETURN: Duration = Duration::from_millis(150);
/// A new uinput device takes a moment to be picked up by the compositor and
/// by the emulators that watch for new keyboards.
pub const KEYBOARD_SETTLE: Duration = Duration::from_millis(300);

/// What the menu can ask of the emulator, besides leaving the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmuAction {
    SaveState,
    LoadState,
    DiscTray,
    NextDisc,
    PrevDisc,
    Reset,
    Menu,
    Screenshot,
}

impl EmuAction {
    /// In menu order.
    pub const ALL: [Self; 8] = [
        Self::SaveState,
        Self::LoadState,
        Self::DiscTray,
        Self::NextDisc,
        Self::PrevDisc,
        Self::Reset,
        Self::Menu,
        Self::Screenshot,
    ];

    /// The key of the entry's label in the translations.
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::SaveState => "menu-save-state",
            Self::LoadState => "menu-load-state",
            Self::DiscTray => "menu-disc-tray",
            Self::NextDisc => "menu-next-disc",
            Self::PrevDisc => "menu-prev-disc",
            Self::Reset => "menu-reset",
            Self::Menu => "menu-emulator-menu",
            Self::Screenshot => "menu-screenshot",
        }
    }

    /// Only for games made of several discs.
    const fn is_disc(self) -> bool {
        matches!(self, Self::DiscTray | Self::NextDisc | Self::PrevDisc)
    }

    /// The hotkey action (session.json's name) that performs it.
    const fn hotkey(self) -> &'static str {
        match self {
            Self::SaveState => "save_state",
            Self::LoadState => "load_state",
            Self::DiscTray => "disk_eject",
            Self::NextDisc => "disk_next",
            Self::PrevDisc => "disk_prev",
            Self::Reset => "reset",
            Self::Menu => "menu",
            Self::Screenshot => "screenshot",
        }
    }
}

/// One thing to do to the emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Keys(Vec<KeyCode>),
    Exec(Vec<String>),
}

/// What a binding lets the overlay do: press its chord or run its command.
fn step_of(binding: &Binding) -> Option<Step> {
    let keys = match binding {
        Binding::Exec { argv } => {
            return (!argv.is_empty()).then(|| Step::Exec(argv.clone()));
        }
        Binding::Keys { keys } | Binding::Native { keys } => keys,
    };
    let codes: Option<Vec<KeyCode>> = keys
        .iter()
        .map(|name| KeyCode::from_str(name).ok())
        .collect();
    match codes {
        Some(codes) if !codes.is_empty() => Some(Step::Keys(codes)),
        Some(_) => None,
        None => {
            log::warn!("unknown key in {keys:?}: ignored");
            None
        }
    }
}

/// Whether the game is a playlist of discs (the only case the disc entries mean anything).
fn is_playlist(rom: &str) -> bool {
    std::path::Path::new(rom)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("m3u"))
}

/// How the emulator does each action and pauses, as session.json says.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Plan {
    pause: Option<Vec<KeyCode>>,
    actions: HashMap<EmuAction, Step>,
}

impl Plan {
    fn new(session: &Session, pause_game: bool) -> Self {
        let step = |name: &str| session.hotkeys.get(name).and_then(step_of);
        let playlist = is_playlist(&session.rom);
        let actions = EmuAction::ALL
            .into_iter()
            .filter(|action| playlist || !action.is_disc())
            .filter_map(|action| Some((action, step(action.hotkey())?)))
            .collect();
        let pause = match step("pause") {
            Some(Step::Keys(keys)) if pause_game => Some(keys),
            _ => None,
        };
        Self { pause, actions }
    }

    /// Every key the plan may press, for the virtual keyboard's capabilities.
    fn keys(&self) -> AttributeSet<KeyCode> {
        let mut set = AttributeSet::new();
        let chords = self.actions.values().filter_map(|step| match step {
            Step::Keys(keys) => Some(keys),
            Step::Exec(_) => None,
        });
        for key in chords.chain(self.pause.iter()).flatten() {
            set.insert(*key);
        }
        set
    }

    fn needs_keyboard(&self) -> bool {
        self.keys().iter().next().is_some()
    }
}

/// The emulator of the running game, as far as the overlay can drive it.
pub struct Emulator {
    plan: Plan,
    keyboard: Option<VirtualDevice>,
    paused: bool,
}

impl Emulator {
    /// No emulator: the demo, or a game without session.
    pub fn none() -> Self {
        Self {
            plan: Plan::default(),
            keyboard: None,
            paused: false,
        }
    }

    pub fn new(session: &Session, pause_game: bool) -> Self {
        let mut plan = Plan::new(session, pause_game);
        let keyboard = if plan.needs_keyboard() {
            match create_keyboard(&plan.keys()) {
                Ok(device) => Some(device),
                Err(err) => {
                    log::warn!("cannot create the virtual keyboard (/dev/uinput): {err}");
                    None
                }
            }
        } else {
            None
        };
        if keyboard.is_none() {
            // without a keyboard only the commands are left
            plan.pause = None;
            plan.actions.retain(|_, step| matches!(step, Step::Exec(_)));
        }
        Self {
            plan,
            keyboard,
            paused: false,
        }
    }

    /// The actions the menu can offer, in menu order.
    pub fn actions(&self) -> Vec<EmuAction> {
        EmuAction::ALL
            .into_iter()
            .filter(|action| self.plan.actions.contains_key(action))
            .collect()
    }

    /// Whether opening the overlay pauses the game (so the menu must wait
    /// for that before taking the keyboard focus).
    pub fn can_pause(&self) -> bool {
        self.plan.pause.is_some()
    }

    /// Pause the game (its pause toggle), once.
    pub fn pause(&mut self) {
        if self.paused {
            return;
        }
        if let Some(keys) = self.plan.pause.clone() {
            log::info!("pausing the game");
            self.press(&keys);
            self.paused = true;
        }
    }

    /// Once the overlay is gone: resume the game and do what the menu asked.
    pub fn after_close(&mut self, action: Option<EmuAction>) {
        if self.paused || action.is_some() {
            sleep(FOCUS_RETURN);
        }
        self.resume();
        if let Some(action) = action {
            self.perform(action);
        }
    }

    /// Undo `pause`: the same toggle again, so a game the user had paused
    /// before opening the overlay stays paused.
    fn resume(&mut self) {
        if !self.paused {
            return;
        }
        if let Some(keys) = self.plan.pause.clone() {
            log::info!("resuming the game");
            self.press(&keys);
        }
        self.paused = false;
    }

    /// Do what the menu entry asks.
    fn perform(&mut self, action: EmuAction) {
        let Some(step) = self.plan.actions.get(&action).cloned() else {
            return;
        };
        log::info!("{action:?}");
        match step {
            Step::Keys(keys) => self.press(&keys),
            Step::Exec(argv) => spawn(&argv),
        }
    }

    /// Press a chord: modifiers first, released in reverse order.
    fn press(&mut self, keys: &[KeyCode]) {
        let Some(device) = self.keyboard.as_mut() else {
            return;
        };
        let down: Vec<_> = keys.iter().map(|key| *KeyEvent::new(*key, 1)).collect();
        let up: Vec<_> = keys
            .iter()
            .rev()
            .map(|key| *KeyEvent::new(*key, 0))
            .collect();
        if let Err(err) = device.emit(&down) {
            log::warn!("cannot press {keys:?}: {err}");
            return;
        }
        sleep(HOLD);
        if let Err(err) = device.emit(&up) {
            log::warn!("cannot release {keys:?}: {err}");
        }
    }
}

/// The virtual keyboard. It declares every ordinary key, not only the ones
/// it presses: udev tags a device as a keyboard (`ID_INPUT_KEYBOARD`) only
/// then, and emulators reading the keyboards themselves (RetroArch's udev
/// driver) ignore anything else.
fn create_keyboard(keys: &AttributeSet<KeyCode>) -> std::io::Result<VirtualDevice> {
    let mut all = AttributeSet::new();
    for code in 1..=KeyCode::KEY_MICMUTE.code() {
        all.insert(KeyCode::new(code));
    }
    for key in keys {
        all.insert(key);
    }
    VirtualDevice::builder()?
        .name(KEYBOARD_NAME)
        .with_keys(&all)?
        .build()
}

/// Run a command detached from the overlay, which exits right after.
pub fn spawn(argv: &[String]) {
    let Some((program, arguments)) = argv.split_first() else {
        return;
    };
    if let Err(err) = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
    {
        log::error!("cannot run {program}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(rom: &str, hotkeys: &str) -> Session {
        serde_json::from_str(&format!(
            r#"{{"pid": 1, "rom": "{rom}", "hotkeys": {hotkeys}}}"#
        ))
        .unwrap()
    }

    const RETROARCH: &str = r#"{
        "pause": {"type": "keys", "keys": ["KEY_LEFTSHIFT", "KEY_P"]},
        "save_state": {"type": "native", "keys": ["KEY_LEFTSHIFT", "KEY_F3"]},
        "menu": {"type": "native", "keys": []},
        "disk_next": {"type": "native", "keys": ["KEY_LEFTSHIFT", "KEY_PAGEDOWN"]},
        "disk_eject": {"type": "keys", "keys": ["KEY_LEFTSHIFT", "KEY_F8"]},
        "disk_prev": {"type": "keys", "keys": ["KEY_LEFTSHIFT", "KEY_PAGEUP"]},
        "screenshot": {"type": "exec", "argv": ["/opt/retrobox/bin/retrobox-screenshot"]},
        "reset": {"type": "keys", "keys": ["KEY_NOPE"]}
    }"#;

    #[test]
    fn plan_keeps_what_can_be_done() {
        let plan = Plan::new(&session("/roms/psx/ff7.m3u", RETROARCH), true);
        assert_eq!(
            plan.pause,
            Some(vec![KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_P])
        );
        // a native binding without keys, and an unknown key, give nothing
        assert!(!plan.actions.contains_key(&EmuAction::Menu));
        assert!(!plan.actions.contains_key(&EmuAction::Reset));
        assert_eq!(
            plan.actions[&EmuAction::SaveState],
            Step::Keys(vec![KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_F3])
        );
        assert_eq!(
            plan.actions[&EmuAction::Screenshot],
            Step::Exec(vec!["/opt/retrobox/bin/retrobox-screenshot".into()])
        );
        for disc in [
            EmuAction::DiscTray,
            EmuAction::NextDisc,
            EmuAction::PrevDisc,
        ] {
            assert!(plan.actions.contains_key(&disc));
        }
        assert!(plan.needs_keyboard());
    }

    #[test]
    fn next_disc_only_for_playlists_and_pause_optional() {
        let plan = Plan::new(&session("/roms/psx/game.chd", RETROARCH), false);
        assert!(!plan.actions.contains_key(&EmuAction::DiscTray));
        assert!(!plan.actions.contains_key(&EmuAction::NextDisc));
        assert_eq!(plan.pause, None);
    }

    #[test]
    fn commands_need_no_keyboard() {
        let plan = Plan::new(
            &session(
                "/roms/steam/game.steam",
                r#"{"screenshot": {"type": "exec", "argv": ["shot"]}}"#,
            ),
            true,
        );
        assert!(!plan.needs_keyboard());
        assert_eq!(plan.pause, None);
    }
}
