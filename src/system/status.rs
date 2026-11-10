//! The network and the battery, from sysfs: cheap enough to read every few
//! seconds while the overlay is open.

use std::fs;
use std::path::Path;

const NET_DIR: &str = "/sys/class/net";
const POWER_DIR: &str = "/sys/class/power_supply";
const WIRELESS: &str = "/proc/net/wireless";
/// The best link quality `/proc/net/wireless` reports.
const MAX_QUALITY: f32 = 70.0;

/// How the computer is connected; a cable wins over Wi-Fi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Network {
    Wired,
    /// The signal, 0 to 1 (None: unknown).
    Wifi(Option<f32>),
    /// Network hardware, but no connection.
    Offline,
}

/// The computer's battery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// What the status line shows.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Status {
    /// None: no network hardware (nothing to show).
    pub network: Option<Network>,
    /// None: no battery (a desktop computer).
    pub battery: Option<Battery>,
}

impl Status {
    pub fn read() -> Self {
        Self {
            network: network(Path::new(NET_DIR)),
            battery: battery(Path::new(POWER_DIR)),
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

/// The physical interfaces (with a device behind them, unlike `lo`,
/// bridges, VPNs or containers' links) and whether each is up with a link.
fn network(dir: &Path) -> Option<Network> {
    let mut any = false;
    let mut wifi_up = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let iface = entry.path();
        if !iface.join("device").exists() {
            continue;
        }
        any = true;
        let up = read(&iface.join("operstate")) == "up" && read(&iface.join("carrier")) == "1";
        if !up {
            continue;
        }
        let wireless = iface.join("wireless").exists() || iface.join("phy80211").exists();
        if !wireless {
            return Some(Network::Wired);
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        wifi_up = Some(Network::Wifi(signal(&name)));
    }
    wifi_up.or(any.then_some(Network::Offline))
}

/// The link quality of a wireless interface (`/proc/net/wireless`).
fn signal(iface: &str) -> Option<f32> {
    parse_wireless(&fs::read_to_string(WIRELESS).ok()?, iface)
}

/// `wlan0: 0000   54.  -56.  -256 ...`: the link quality, 0 to 1.
fn parse_wireless(text: &str, iface: &str) -> Option<f32> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("{iface}:")))?;
    let quality: f32 = line
        .split_whitespace()
        .nth(2)?
        .trim_end_matches('.')
        .parse()
        .ok()?;
    Some((quality / MAX_QUALITY).clamp(0.0, 1.0))
}

/// The computer's own battery: a controller's (scope `Device`) is not.
fn battery(dir: &Path) -> Option<Battery> {
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let supply = entry.path();
        if read(&supply.join("type")) != "Battery" || read(&supply.join("scope")) == "Device" {
            return None;
        }
        let percent: u8 = read(&supply.join("capacity")).parse().ok()?;
        let status = read(&supply.join("status"));
        Some(Battery {
            percent: percent.min(100),
            charging: status == "Charging",
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wireless_quality() {
        let text = "Inter-| sta-|   Quality        |   Discarded packets\n \
                    face | tus | link level noise |  nwid  crypt   frag\n \
                    wlp2s0: 0000   56.  -54.  -256        0      0      0\n";
        assert_eq!(parse_wireless(text, "wlp2s0"), Some(0.8));
        assert_eq!(parse_wireless(text, "wlan0"), None);
    }

    #[test]
    fn cable_wins() {
        let dir = std::env::temp_dir().join(format!("overlay-net-{}", std::process::id()));
        let iface = |name: &str, up: bool, wireless: bool| {
            let path = dir.join(name);
            fs::create_dir_all(path.join("device")).unwrap();
            if wireless {
                fs::create_dir_all(path.join("wireless")).unwrap();
            }
            fs::write(path.join("operstate"), if up { "up\n" } else { "down\n" }).unwrap();
            fs::write(path.join("carrier"), if up { "1\n" } else { "0\n" }).unwrap();
        };
        fs::create_dir_all(dir.join("lo")).unwrap();
        iface("wlan0", true, true);
        assert!(matches!(network(&dir), Some(Network::Wifi(_))));
        iface("enp3s0", true, false);
        assert_eq!(network(&dir), Some(Network::Wired));
        iface("enp3s0", false, false);
        iface("wlan0", false, true);
        assert_eq!(network(&dir), Some(Network::Offline));
        fs::remove_dir_all(dir).unwrap();
    }
}
