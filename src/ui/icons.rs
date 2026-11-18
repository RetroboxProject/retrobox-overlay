//! The menu's icons: Lucide (<https://lucide.dev>, ISC license, see
//! assets/icons/LICENSE), built into the binary and drawn in the colour of
//! the text next to them.

use resvg::usvg;
use tiny_skia::{Pixmap, Transform};

use super::panel::MenuAction;
use crate::emulator::EmuAction;
use crate::savestates::StateMode;
use crate::system::SliderKind;
use crate::system::power::PowerAction;
use crate::system::status::{Battery, Network};
use crate::theme::Rgba;

/// Lucide draws on a 24×24 grid.
const GRID: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub const ALL: [Self; 46] = [
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

    /// The icon of Lucide's `name` (`trophy`, `coffee`, `wifi-off`...): what
    /// other programs ask for (see `retrobox-overlay --toast`).
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|icon| icon.name() == name)
    }

    /// Lucide's name of the icon, from its SVG (`class="lucide lucide-<name>"`).
    pub fn name(self) -> &'static str {
        let svg = self.svg();
        svg.split_once("lucide lucide-")
            .and_then(|(_, rest)| rest.split(['"', ' ']).next())
            .unwrap_or_default()
    }

    /// The icon of a power option.
    pub const fn of_power(action: PowerAction) -> Self {
        match action {
            PowerAction::Standby => Self::Moon,
            PowerAction::Reboot => Self::RotateCw,
            PowerAction::PowerOff => Self::Power,
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
            MenuAction::Power => Self::Power,
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

    const fn svg(self) -> &'static str {
        match self {
            Self::Play => include_str!("../../assets/icons/play.svg"),
            Self::Save => include_str!("../../assets/icons/save.svg"),
            Self::History => include_str!("../../assets/icons/history.svg"),
            Self::Eject => include_str!("../../assets/icons/eject.svg"),
            Self::SkipForward => include_str!("../../assets/icons/skip-forward.svg"),
            Self::SkipBack => include_str!("../../assets/icons/skip-back.svg"),
            Self::RotateCcw => include_str!("../../assets/icons/rotate-ccw.svg"),
            Self::RotateCw => include_str!("../../assets/icons/rotate-cw.svg"),
            Self::Moon => include_str!("../../assets/icons/moon.svg"),
            Self::Coffee => include_str!("../../assets/icons/coffee.svg"),
            Self::Settings => include_str!("../../assets/icons/settings.svg"),
            Self::Camera => include_str!("../../assets/icons/camera.svg"),
            Self::LogOut => include_str!("../../assets/icons/log-out.svg"),
            Self::Disc => include_str!("../../assets/icons/disc-3.svg"),
            Self::BookOpen => include_str!("../../assets/icons/book-open.svg"),
            Self::Info => include_str!("../../assets/icons/info.svg"),
            Self::Keyboard => include_str!("../../assets/icons/keyboard.svg"),
            Self::Timer => include_str!("../../assets/icons/timer.svg"),
            Self::Trophy => include_str!("../../assets/icons/trophy.svg"),
            Self::Check => include_str!("../../assets/icons/check.svg"),
            Self::Sun => include_str!("../../assets/icons/sun.svg"),
            Self::VolumeX => include_str!("../../assets/icons/volume-x.svg"),
            Self::Volume1 => include_str!("../../assets/icons/volume-1.svg"),
            Self::Volume2 => include_str!("../../assets/icons/volume-2.svg"),
            Self::Power => include_str!("../../assets/icons/power.svg"),
            Self::Wifi => include_str!("../../assets/icons/wifi.svg"),
            Self::WifiHigh => include_str!("../../assets/icons/wifi-high.svg"),
            Self::WifiLow => include_str!("../../assets/icons/wifi-low.svg"),
            Self::WifiZero => include_str!("../../assets/icons/wifi-zero.svg"),
            Self::WifiOff => include_str!("../../assets/icons/wifi-off.svg"),
            Self::EthernetPort => include_str!("../../assets/icons/ethernet-port.svg"),
            Self::BatteryLow => include_str!("../../assets/icons/battery-low.svg"),
            Self::BatteryMedium => include_str!("../../assets/icons/battery-medium.svg"),
            Self::BatteryFull => include_str!("../../assets/icons/battery-full.svg"),
            Self::BatteryCharging => include_str!("../../assets/icons/battery-charging.svg"),
            Self::BatteryWarning => include_str!("../../assets/icons/battery-warning.svg"),
            Self::CornerDownLeft => include_str!("../../assets/icons/corner-down-left.svg"),
            Self::ArrowBigUp => include_str!("../../assets/icons/arrow-big-up.svg"),
            Self::ArrowBigUpDash => include_str!("../../assets/icons/arrow-big-up-dash.svg"),
            Self::Delete => include_str!("../../assets/icons/delete.svg"),
            Self::Space => include_str!("../../assets/icons/space.svg"),
            Self::X => include_str!("../../assets/icons/x.svg"),
            Self::ArrowLeft => include_str!("../../assets/icons/arrow-left.svg"),
            Self::ArrowRight => include_str!("../../assets/icons/arrow-right.svg"),
            Self::ArrowRightToLine => include_str!("../../assets/icons/arrow-right-to-line.svg"),
            Self::Gamepad => include_str!("../../assets/icons/gamepad-2.svg"),
        }
    }

    /// The icon as a `size`×`size` image in `color`.
    pub fn render(self, size: u32, color: Rgba) -> Option<Pixmap> {
        let [red, green, blue, alpha] = color.0.to_be_bytes();
        let stroke = format!(
            "stroke=\"#{red:02x}{green:02x}{blue:02x}\" stroke-opacity=\"{:.3}\"",
            f32::from(alpha) / 255.0
        );
        let svg = self.svg().replace("stroke=\"currentColor\"", &stroke);
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
    fn icons_go_by_lucide_s_names() {
        assert_eq!(Icon::named("trophy"), Some(Icon::Trophy));
        assert_eq!(
            Icon::named("arrow-right-to-line"),
            Some(Icon::ArrowRightToLine)
        );
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
