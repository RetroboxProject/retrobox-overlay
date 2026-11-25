//! The menu's icons: Remix Icon's outlined ("line") ones
//! (<https://remixicon.com>, Apache License 2.0) and a few of Lucide's
//! (<https://lucide.dev>, ISC license) redrawn in Remix Icon's manner (shapes
//! instead of strokes, sharper corners), built into the binary and filled in
//! the colour of the text next to them; see assets/icons/NOTICE for which is
//! which and what was changed for the overlay.

use std::cell::RefCell;
use std::collections::HashMap;

use resvg::usvg;
use tiny_skia::{Pixmap, Transform};

use super::panel::MenuAction;
use crate::emulator::EmuAction;
use crate::savestates::StateMode;
use crate::system::SliderKind;
use crate::system::power::PowerAction;
use crate::system::status::{Battery, Network};
use crate::theme::Rgba;

/// Both sets draw on a 24×24 grid.
const GRID: f32 = 24.0;

/// How many rendered icons are kept (all of them are forgotten past that).
const RENDERS_KEPT: usize = 512;

thread_local! {
    /// The icons already rendered, by size and colour: a menu draws the same
    /// ones every time, and parsing and rendering an SVG costs far more than
    /// copying its pixels.
    static RENDERS: RefCell<HashMap<(Icon, u32, u32), Option<Pixmap>>> =
        RefCell::new(HashMap::new());
}

/// An icon's name (its set's) and its SVG, built in.
macro_rules! icon {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!("../../assets/icons/", $name, ".svg")),
        )
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Play,
    Save,
    History,
    Eject,
    SkipForward,
    SkipBack,
    RotateCcw,
    RotateCw,
    Moon,
    Coffee,
    Settings,
    Camera,
    LogOut,
    Disc,
    BookOpen,
    Info,
    Keyboard,
    Timer,
    Trophy,
    Check,
    Sun,
    VolumeX,
    Volume1,
    Volume2,
    Power,
    System,
    Download,
    Wifi,
    WifiHigh,
    WifiLow,
    WifiZero,
    WifiOff,
    EthernetPort,
    BatteryLow,
    BatteryMedium,
    BatteryFull,
    BatteryCharging,
    BatteryWarning,
    CornerDownLeft,
    ArrowBigUp,
    ArrowBigUpDash,
    Delete,
    Space,
    X,
    ArrowLeft,
    ArrowRight,
    ArrowRightToLine,
    Gamepad,
}

impl Icon {
    /// Every icon (for `named`).
    pub const ALL: [Self; 48] = [
        Self::Play,
        Self::Save,
        Self::History,
        Self::Eject,
        Self::SkipForward,
        Self::SkipBack,
        Self::RotateCcw,
        Self::RotateCw,
        Self::Moon,
        Self::Coffee,
        Self::Settings,
        Self::Camera,
        Self::LogOut,
        Self::Disc,
        Self::BookOpen,
        Self::Info,
        Self::Keyboard,
        Self::Timer,
        Self::Trophy,
        Self::Check,
        Self::Sun,
        Self::VolumeX,
        Self::Volume1,
        Self::Volume2,
        Self::Power,
        Self::System,
        Self::Download,
        Self::Wifi,
        Self::WifiHigh,
        Self::WifiLow,
        Self::WifiZero,
        Self::WifiOff,
        Self::EthernetPort,
        Self::BatteryLow,
        Self::BatteryMedium,
        Self::BatteryFull,
        Self::BatteryCharging,
        Self::BatteryWarning,
        Self::CornerDownLeft,
        Self::ArrowBigUp,
        Self::ArrowBigUpDash,
        Self::Delete,
        Self::Space,
        Self::X,
        Self::ArrowLeft,
        Self::ArrowRight,
        Self::ArrowRightToLine,
        Self::Gamepad,
    ];

    /// The icon of `name`, its file's (`trophy`, `history-line`,
    /// `signal-wifi-off-line`...), with or without a Remix Icon's `-line`:
    /// what other programs ask for (see `retrobox-overlay --toast`).
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|icon| {
            let own = icon.name();
            own == name || own.strip_suffix("-line") == Some(name)
        })
    }

    /// The icon's name.
    pub const fn name(self) -> &'static str {
        self.file().0
    }

    /// The icon of a power option.
    pub const fn of_power(action: PowerAction) -> Self {
        match action {
            PowerAction::Standby => Self::Moon,
            PowerAction::Reboot => Self::RotateCw,
            PowerAction::PowerOff => Self::Power,
            PowerAction::Update => Self::Download,
        }
    }

    /// The icon of a menu entry (none for the save state slots).
    pub const fn for_action(action: MenuAction) -> Option<Self> {
        Some(match action {
            MenuAction::Continue => Self::Play,
            MenuAction::States(StateMode::Save) | MenuAction::Emulator(EmuAction::SaveState) => {
                Self::Save
            }
            MenuAction::States(StateMode::Load) | MenuAction::Emulator(EmuAction::LoadState) => {
                Self::History
            }
            MenuAction::Emulator(EmuAction::DiscTray) => Self::Eject,
            MenuAction::Emulator(EmuAction::NextDisc) => Self::SkipForward,
            MenuAction::Emulator(EmuAction::PrevDisc) => Self::SkipBack,
            MenuAction::Emulator(EmuAction::Reset) => Self::RotateCcw,
            MenuAction::Emulator(EmuAction::Menu) => Self::Settings,
            MenuAction::Emulator(EmuAction::Screenshot) => Self::Camera,
            MenuAction::Manual => Self::BookOpen,
            MenuAction::Info => Self::Info,
            MenuAction::Controls => Self::Keyboard,
            MenuAction::Achievements => Self::Trophy,
            MenuAction::Quit { .. } => Self::LogOut,
            MenuAction::Power => Self::System,
            MenuAction::Slider(SliderKind::Brightness) => Self::Sun,
            MenuAction::Slider(SliderKind::Volume) => Self::Volume2,
            MenuAction::Slot(..) | MenuAction::Choice(_) => return None,
        })
    }

    /// The volume slider's icon at a value: more waves, louder.
    pub fn volume(value: f32) -> Self {
        if value <= 0.0 {
            Self::VolumeX
        } else if value < 0.5 {
            Self::Volume1
        } else {
            Self::Volume2
        }
    }

    /// The network's icon: a cable, or the Wi-Fi's signal.
    pub fn network(network: Network) -> Self {
        match network {
            Network::Wired => Self::EthernetPort,
            Network::Wifi(None) => Self::Wifi,
            Network::Wifi(Some(signal)) if signal >= 0.66 => Self::Wifi,
            Network::Wifi(Some(signal)) if signal >= 0.4 => Self::WifiHigh,
            Network::Wifi(Some(signal)) if signal >= 0.15 => Self::WifiLow,
            Network::Wifi(Some(_)) => Self::WifiZero,
            Network::Offline => Self::WifiOff,
        }
    }

    /// The battery's icon: its charge, or that it is charging.
    pub fn battery(battery: Battery) -> Self {
        match battery.percent {
            _ if battery.charging => Self::BatteryCharging,
            75.. => Self::BatteryFull,
            40.. => Self::BatteryMedium,
            15.. => Self::BatteryLow,
            _ => Self::BatteryWarning,
        }
    }

    /// The icon's file: its name in its set (Remix Icon's or Lucide's; the
    /// ones made for the overlay are named alike).
    const fn file(self) -> (&'static str, &'static str) {
        match self {
            Self::Play => icon!("play-large-line"),
            Self::Save => icon!("save-line"),
            Self::History => icon!("history-line"),
            Self::Eject => icon!("eject-line"),
            Self::SkipForward => icon!("skip-forward-line"),
            Self::SkipBack => icon!("skip-back-line"),
            Self::RotateCcw => icon!("reset-left-line"),
            Self::RotateCw => icon!("reset-right-line"),
            Self::Moon => icon!("moon-line"),
            Self::Coffee => icon!("coffee"),
            Self::Settings => icon!("settings"),
            Self::Camera => icon!("camera-line"),
            Self::LogOut => icon!("logout-box-r-line"),
            Self::Disc => icon!("disc-3"),
            Self::BookOpen => icon!("book-open"),
            Self::Info => icon!("information-line"),
            Self::Keyboard => icon!("keyboard"),
            Self::Timer => icon!("timer-line"),
            Self::Trophy => icon!("trophy"),
            Self::Check => icon!("check-line"),
            Self::Sun => icon!("sun"),
            Self::VolumeX => icon!("volume-mute-line"),
            Self::Volume1 => icon!("volume-down-line"),
            Self::Volume2 => icon!("volume-up-line"),
            Self::Power => icon!("shut-down-line"),
            Self::System => icon!("system"),
            Self::Download => icon!("download-line"),
            Self::Wifi => icon!("signal-wifi-fill"),
            Self::WifiHigh => icon!("signal-wifi-high-line"),
            Self::WifiLow => icon!("signal-wifi-low-line"),
            Self::WifiZero => icon!("signal-wifi-line"),
            Self::WifiOff => icon!("signal-wifi-off-line"),
            Self::EthernetPort => icon!("ethernet-port"),
            Self::BatteryLow => icon!("battery-low-line"),
            Self::BatteryMedium => icon!("battery-medium-line"),
            Self::BatteryFull => icon!("battery-full-line"),
            Self::BatteryCharging => icon!("battery-charge-line"),
            Self::BatteryWarning => icon!("battery-line"),
            Self::CornerDownLeft => icon!("corner-down-left-line"),
            Self::ArrowBigUp => icon!("arrow-up-line"),
            Self::ArrowBigUpDash => icon!("upload-line"),
            Self::Delete => icon!("delete-back-2-line"),
            Self::Space => icon!("space"),
            Self::X => icon!("close-line"),
            Self::ArrowLeft => icon!("arrow-left-line"),
            Self::ArrowRight => icon!("arrow-right-line"),
            Self::ArrowRightToLine => icon!("skip-right-line"),
            Self::Gamepad => icon!("gamepad-2"),
        }
    }

    const fn svg(self) -> &'static str {
        self.file().1
    }

    /// The icon as a `size`×`size` image in `color`.
    pub fn render(self, size: u32, color: Rgba) -> Option<Pixmap> {
        let key = (self, size, color.0);
        if let Some(rendered) = RENDERS.with_borrow(|renders| renders.get(&key).cloned()) {
            return rendered;
        }
        let rendered = self.rasterize(size, color);
        RENDERS.with_borrow_mut(|renders| {
            if renders.len() >= RENDERS_KEPT {
                renders.clear();
            }
            renders.insert(key, rendered.clone());
        });
        rendered
    }

    fn rasterize(self, size: u32, color: Rgba) -> Option<Pixmap> {
        let [red, green, blue, alpha] = color.0.to_be_bytes();
        let fill = format!(
            "fill=\"#{red:02x}{green:02x}{blue:02x}\" fill-opacity=\"{:.3}\"",
            f32::from(alpha) / 255.0
        );
        let svg = self.svg().replace("fill=\"currentColor\"", &fill);
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())
            .inspect_err(|err| log::warn!("icon {self:?}: {err}"))
            .ok()?;
        let mut pixmap = Pixmap::new(size, size)?;
        let scale = size as f32 / GRID;
        resvg::render(
            &tree,
            Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        Some(pixmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_go_by_their_sets_names() {
        assert_eq!(Icon::named("trophy"), Some(Icon::Trophy));
        assert_eq!(Icon::named("history-line"), Some(Icon::History));
        assert_eq!(Icon::named("history"), Some(Icon::History));
        assert_eq!(Icon::named("coffee"), Some(Icon::Coffee));
        assert_eq!(Icon::named("signal-wifi-fill"), Some(Icon::Wifi));
        assert_eq!(Icon::named("space"), Some(Icon::Space));
        assert_eq!(Icon::named("nope"), None);
        for icon in Icon::ALL {
            assert_eq!(Icon::named(icon.name()), Some(icon), "{icon:?}");
        }
    }

    #[test]
    fn every_icon_renders_in_its_colour() {
        for icon in Icon::ALL {
            let pixmap = icon.render(48, Rgba(0xff00_00ff)).unwrap();
            let drawn: Vec<_> = pixmap.pixels().iter().filter(|p| p.alpha() > 200).collect();
            assert!(!drawn.is_empty(), "{icon:?} is empty");
            assert!(
                drawn.iter().all(|p| p.red() > 200 && p.green() < 30),
                "{icon:?} is not red"
            );
        }
    }
}
