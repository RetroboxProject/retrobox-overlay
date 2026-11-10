//! The menu's icons: Lucide (<https://lucide.dev>, ISC license, see
//! assets/icons/LICENSE), built into the binary and drawn in the colour of
//! the text next to them.

use resvg::usvg;
use tiny_skia::{Pixmap, Transform};

use super::panel::MenuAction;
use crate::emulator::EmuAction;
use crate::savestates::StateMode;
use crate::system::SliderKind;
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
    Settings,
    Camera,
    LogOut,
    Disc,
    BookOpen,
    Sun,
    VolumeX,
    Volume1,
    Volume2,
    Moon,
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
}

impl Icon {
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
            MenuAction::Quit => Self::LogOut,
            MenuAction::Standby => Self::Moon,
            MenuAction::Slider(SliderKind::Brightness) => Self::Sun,
            MenuAction::Slider(SliderKind::Volume) => Self::Volume2,
            MenuAction::Slot(..) => return None,
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
            Self::Settings => include_str!("../../assets/icons/settings.svg"),
            Self::Camera => include_str!("../../assets/icons/camera.svg"),
            Self::LogOut => include_str!("../../assets/icons/log-out.svg"),
            Self::Disc => include_str!("../../assets/icons/disc-3.svg"),
            Self::BookOpen => include_str!("../../assets/icons/book-open.svg"),
            Self::Sun => include_str!("../../assets/icons/sun.svg"),
            Self::VolumeX => include_str!("../../assets/icons/volume-x.svg"),
            Self::Volume1 => include_str!("../../assets/icons/volume-1.svg"),
            Self::Volume2 => include_str!("../../assets/icons/volume-2.svg"),
            Self::Moon => include_str!("../../assets/icons/moon.svg"),
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
    fn every_icon_renders_in_its_colour() {
        let icons = [
            Icon::Play,
            Icon::Save,
            Icon::History,
            Icon::Eject,
            Icon::SkipForward,
            Icon::SkipBack,
            Icon::RotateCcw,
            Icon::Settings,
            Icon::Camera,
            Icon::LogOut,
            Icon::Disc,
            Icon::BookOpen,
            Icon::Sun,
            Icon::VolumeX,
            Icon::Volume1,
            Icon::Volume2,
            Icon::Moon,
            Icon::Wifi,
            Icon::WifiHigh,
            Icon::WifiLow,
            Icon::WifiZero,
            Icon::WifiOff,
            Icon::EthernetPort,
            Icon::BatteryLow,
            Icon::BatteryMedium,
            Icon::BatteryFull,
            Icon::BatteryCharging,
            Icon::BatteryWarning,
        ];
        for icon in icons {
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
