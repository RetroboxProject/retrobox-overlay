//! The brightness of the primary screen, on Wayland and on X11.
//!
//! Finding it starts no program, so the slider is there as the menu shows:
//!
//! * the screens: the connectors that are on (`/sys/class/drm`), the
//!   kernel's backlight (`/sys/class/backlight`) and the monitors that
//!   answer DDC/CI, as the daemon found them when it started
//!   (`overlay-ddc.json` in the runtime directory: `ddcutil detect` takes a
//!   second or more);
//! * the primary one: the only screen that is on; else the output the
//!   overlay shows on (Wayland: where the compositor put it, that is the
//!   screen in use), or X11's primary output (`RandR`).
//!
//! A built-in panel (eDP, LVDS, DSI) uses the backlight: written in sysfs
//! when allowed, else through logind (`SetBrightness`, allowed to the active
//! session), else `brightnessctl`; whichever works first is kept, and a
//! failure later is the driver's (NVIDIA's `nvidia_0` refuses a change while
//! it is still fading to the last one, `EPROTO`): retried, never another way.
//! Each change costs the driver some work (NVIDIA's stalls the display a
//! little), so the same value is never written twice and changes are spaced
//! (`Control::spacing`). An external monitor uses `ddcutil` on its I2C bus
//! (`--bus`: no detection on each change, which is what made it slow). Every
//! DDC/CI exchange goes through the GPU's driver, and NVIDIA's stalls the
//! display meanwhile: the monitor is never read when the overlay opens (the
//! slider shows the value last read or set, kept in the cache), and it is
//! written once per slide, like a backlight.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{output, run};

/// The lowest brightness a slider at 0 sets on a backlight: some panels go
/// black at 0, which would leave no way to see the slider again.
const MIN_BACKLIGHT: f32 = 0.05;
const BACKLIGHT_DIR: &str = "/sys/class/backlight";
const DRM_DIR: &str = "/sys/class/drm";
/// The monitors that answer DDC/CI, in the runtime directory.
pub const DDC_CACHE: &str = "overlay-ddc.json";

/// A built-in panel's connector name (DRM's or X11's: `eDP-1`, `eDP1`,
/// `LVDS-1`, `DSI-1`).
fn is_internal(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["edp", "lvds", "dsi"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// `card1-HDMI-A-1` and `HDMI-A-1` (Wayland) or `HDMI-1` (X11's
/// modesetting): the same connector.
fn same_connector(drm: &str, output: &str) -> bool {
    let connector = strip_card(drm);
    let simple = |name: &str| name.replace("-A-", "-").to_ascii_lowercase();
    connector.eq_ignore_ascii_case(output) || simple(connector) == simple(output)
}

/// `card1-HDMI-A-1` -> `HDMI-A-1` (an output's name stays as it is).
fn strip_card(name: &str) -> &str {
    match name.split_once('-') {
        Some((card, connector)) if card.starts_with("card") => connector,
        _ => name,
    }
}

/// A connector's kind: `card1-HDMI-A-1` -> `hdmi`.
fn kind_of(name: &str) -> String {
    strip_card(name)
        .split(|c: char| c == '-' || c.is_ascii_digit())
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// A monitor that answers DDC/CI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdcMonitor {
    /// `card1-HDMI-A-1`
    pub connector: String,
    /// `/dev/i2c-<bus>`
    pub bus: u32,
    pub max: u32,
    /// The brightness last read or set.
    pub value: u32,
}

/// How a backlight is set: the first way that works is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Unknown,
    Sysfs,
    Logind,
    Brightnessctl,
}

/// The screen whose brightness the slider changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// A kernel backlight (`/sys/class/backlight/<name>`).
    Backlight {
        name: String,
        max: u32,
        method: Method,
        /// The value last written.
        written: Option<u32>,
    },
    /// A monitor reached by `ddcutil`.
    Ddc { monitor: DdcMonitor, cache: PathBuf },
}

/// What is known of the screens, without running anything.
#[derive(Debug, Default)]
pub struct Screens {
    /// The connectors that are on (`card1-eDP-1`).
    on: Vec<String>,
    backlight: Option<(String, u32)>,
    ddc: Vec<DdcMonitor>,
    cache: PathBuf,
}

impl Screens {
    pub fn scan(runtime_dir: &Path) -> Self {
        let cache = runtime_dir.join(DDC_CACHE);
        Self {
            on: connectors_on(),
            backlight: backlight(),
            ddc: load_cache(&cache),
            cache,
        }
    }

    /// Whether the primary screen has to be told (more than one is on).
    pub fn ambiguous(&self) -> bool {
        self.on.len() > 1
    }

    /// Whether DDC/CI could reach the primary screen but the cache does not
    /// know it (the daemon did not run, or the monitor came later).
    pub fn needs_detection(&self, primary: Option<&str>) -> bool {
        self.connector(primary).is_some_and(|connector| {
            !is_internal(&kind_of(&connector)) && self.monitor(&connector).is_none()
        })
    }

    /// The monitors found again (`detect_ddc`).
    pub fn set_ddc(&mut self, monitors: Vec<DdcMonitor>) {
        self.ddc = monitors;
    }

    /// The primary screen's connector (DRM's name when one matches).
    fn connector(&self, primary: Option<&str>) -> Option<String> {
        if let Some(primary) = primary.filter(|name| !name.starts_with("XWAYLAND")) {
            // a connector that is on, else one of the same kind (X11
            // numbers them its own way)
            let kind = kind_of(primary);
            let found = self
                .on
                .iter()
                .find(|on| same_connector(on, primary))
                .or_else(|| self.on.iter().find(|on| kind_of(on) == kind));
            return Some(found.map_or_else(|| primary.to_owned(), Clone::clone));
        }
        match self.on.as_slice() {
            [] => None,
            [only] => Some(only.clone()),
            // unknown: a built-in panel that is on wins
            several => several
                .iter()
                .find(|on| is_internal(&kind_of(on)))
                .or_else(|| several.first())
                .cloned(),
        }
    }

    fn monitor(&self, connector: &str) -> Option<&DdcMonitor> {
        if connector.is_empty() {
            // which screen is unknown: the only monitor that answers
            return match self.ddc.as_slice() {
                [only] => Some(only),
                _ => None,
            };
        }
        self.ddc.iter().find(|monitor| {
            monitor.connector == connector
                || same_connector(&monitor.connector, strip_card(connector))
        })
    }

    /// The primary screen's brightness control (`primary`: its output's name,
    /// when known), if it has one.
    pub fn resolve(&self, primary: Option<&str>) -> Option<Screen> {
        let connector = self.connector(primary);
        // nothing known (no DRM in sysfs): a backlight is a laptop's panel
        let internal = connector
            .as_deref()
            .map_or(self.backlight.is_some(), |c| is_internal(&kind_of(c)));
        log::info!("brightness: primary screen {connector:?} (output {primary:?})");
        if internal {
            let (name, max) = self.backlight.clone()?;
            log::info!("brightness: backlight {name}");
            return Some(Screen::Backlight {
                name,
                max,
                method: Method::Unknown,
                written: None,
            });
        }
        let monitor = self
            .monitor(connector.as_deref().unwrap_or_default())?
            .clone();
        log::info!("brightness: DDC/CI on /dev/i2c-{}", monitor.bus);
        Some(Screen::Ddc {
            monitor,
            cache: self.cache.clone(),
        })
    }
}

impl Screen {
    /// The brightness, 0 to 1: a backlight's from sysfs, a monitor's from the
    /// cache (never read over DDC/CI when the overlay opens: see the module).
    pub fn get(&self) -> Option<f32> {
        match self {
            Self::Backlight { name, max, .. } => {
                let dir = Path::new(BACKLIGHT_DIR).join(name);
                let raw = read_number(&dir.join("brightness"))
                    .or_else(|| read_number(&dir.join("actual_brightness")))?;
                let fraction = raw as f32 / *max as f32;
                Some(((fraction - MIN_BACKLIGHT) / (1.0 - MIN_BACKLIGHT)).clamp(0.0, 1.0))
            }
            Self::Ddc { monitor, .. } => Some(fraction(monitor.value, monitor.max)),
        }
    }

    /// Set the brightness; false when the driver or the monitor refused
    /// (worth trying again a moment later).
    pub fn set(&mut self, value: f32) -> bool {
        let value = value.clamp(0.0, 1.0);
        match self {
            Self::Backlight {
                name,
                max,
                method,
                written,
            } => {
                let fraction = MIN_BACKLIGHT + value * (1.0 - MIN_BACKLIGHT);
                let raw = ((fraction * *max as f32).round() as u32).clamp(1, *max);
                if *written == Some(raw) {
                    return true;
                }
                let done = set_backlight(name, raw, method);
                if done {
                    *written = Some(raw);
                }
                done
            }
            Self::Ddc { monitor, cache } => {
                let raw = (value * monitor.max as f32).round() as u32;
                if raw == monitor.value {
                    return true;
                }
                let raw_text = raw.to_string();
                let done =
                    ddcutil(monitor.bus, &["setvcp", "10", &raw_text, "--noverify"]).is_some();
                if done {
                    monitor.value = raw;
                    remember(cache, monitor);
                }
                done
            }
        }
    }
}

fn fraction(value: u32, max: u32) -> f32 {
    (value as f32 / max.max(1) as f32).clamp(0.0, 1.0)
}

/// Set a backlight the first way that works, and keep it.
fn set_backlight(name: &str, raw: u32, method: &mut Method) -> bool {
    let raw_text = raw.to_string();
    let sysfs = || {
        fs::OpenOptions::new()
            .write(true)
            .open(Path::new(BACKLIGHT_DIR).join(name).join("brightness"))
            .and_then(|mut file| file.write_all(raw_text.as_bytes()))
            .is_ok()
    };
    let logind = || {
        run(
            "busctl",
            &[
                "call",
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "SetBrightness",
                "ssu",
                "backlight",
                name,
                &raw_text,
            ],
        )
    };
    let brightnessctl = || run("brightnessctl", &["--quiet", "-d", name, "set", &raw_text]);
    let ways: [(Method, &dyn Fn() -> bool); 3] = [
        (Method::Sysfs, &sysfs),
        (Method::Logind, &logind),
        (Method::Brightnessctl, &brightnessctl),
    ];
    // a way that worked once is the way: a failure now is the driver's
    if let Some((_, works)) = ways.iter().find(|(way, _)| way == method) {
        let done = works();
        if !done {
            log::debug!("backlight {name}: {raw} refused (the driver is busy?)");
        }
        return done;
    }
    for (way, works) in ways {
        if works() {
            log::info!("backlight {name}: set through {way:?}");
            *method = way;
            return true;
        }
    }
    log::warn!("backlight {name}: cannot be set");
    false
}

/// The connectors that are on: `card1-eDP-1`.
fn connectors_on() -> Vec<String> {
    let Ok(entries) = fs::read_dir(DRM_DIR) else {
        return Vec::new();
    };
    let mut on: Vec<String> = entries
        .flatten()
        .filter(|entry| {
            fs::read_to_string(entry.path().join("enabled"))
                .is_ok_and(|enabled| enabled.trim() == "enabled")
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    on.sort();
    on
}

fn read_number(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The kernel's backlight: the firmware's first, then the platform's, then
/// the GPU's (raw), as the kernel's documentation recommends.
fn backlight() -> Option<(String, u32)> {
    let mut found: Vec<(u8, String, u32)> = fs::read_dir(BACKLIGHT_DIR)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let dir: PathBuf = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let max = read_number(&dir.join("max_brightness")).filter(|max| *max > 0)?;
            let rank = match fs::read_to_string(dir.join("type")).ok()?.trim() {
                "firmware" => 0,
                "platform" => 1,
                _ => 2,
            };
            Some((rank, name, max))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, name, max)| (name, max))
}

fn load_cache(path: &Path) -> Vec<DdcMonitor> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_cache(path: &Path, monitors: &[DdcMonitor]) {
    let Ok(text) = serde_json::to_string_pretty(monitors) else {
        return;
    };
    // whole or nothing: readers never see half a file
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, path))
        .is_err()
    {
        log::debug!("cannot write {}", path.display());
    }
}

/// Keep a monitor's last value in the cache.
fn remember(cache: &Path, monitor: &DdcMonitor) {
    let mut monitors = load_cache(cache);
    match monitors.iter_mut().find(|known| known.bus == monitor.bus) {
        Some(known) => known.clone_from(monitor),
        None => monitors.push(monitor.clone()),
    }
    save_cache(cache, &monitors);
}

/// Find the monitors that answer DDC/CI and write the cache (slow: the
/// daemon does it once when it starts, the overlay only when the cache does
/// not know the primary screen). Nothing without `ddcutil`.
pub fn detect_ddc(runtime_dir: &Path) -> Vec<DdcMonitor> {
    let Some(detected) = output("ddcutil", &["detect", "--brief"]) else {
        return Vec::new();
    };
    let monitors: Vec<DdcMonitor> = parse_detect(&detected)
        .into_iter()
        .filter_map(|(bus, connector)| {
            let (value, max) = parse_vcp(&ddcutil(bus, &["getvcp", "10", "--brief"])?)?;
            Some(DdcMonitor {
                connector,
                bus,
                max: max.max(1),
                value,
            })
        })
        .collect();
    log::info!("DDC/CI: {} monitor(s) with a brightness", monitors.len());
    save_cache(&runtime_dir.join(DDC_CACHE), &monitors);
    monitors
}

fn ddcutil(bus: u32, args: &[&str]) -> Option<String> {
    let bus = bus.to_string();
    let mut full = vec!["--bus", bus.as_str()];
    full.extend_from_slice(args);
    output("ddcutil", &full)
}

/// `ddcutil detect --brief`: each display's I2C bus and DRM connector.
fn parse_detect(text: &str) -> Vec<(u32, String)> {
    let mut displays = Vec::new();
    let mut bus = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("I2C bus:") {
            bus = rest
                .trim()
                .strip_prefix("/dev/i2c-")
                .and_then(|n| n.parse().ok());
        } else if let Some(connector) = line.strip_prefix("DRM connector:")
            && let Some(bus) = bus.take()
        {
            displays.push((bus, connector.trim().to_owned()));
        }
    }
    displays
}

/// `VCP 10 C 50 100`: the current value and the maximum.
fn parse_vcp(text: &str) -> Option<(u32, u32)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        ["VCP", "10", "C", current, max, ..] => Some((current.parse().ok()?, max.parse().ok()?)),
        _ => None,
    }
}

/// X11's primary output (not under Wayland: `XWayland`'s outputs have
/// names of their own).
pub fn x11_primary() -> Option<String> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::randr::ConnectionExt as _;

    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()) {
        return None;
    }
    std::env::var_os("DISPLAY").filter(|display| !display.is_empty())?;
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let primary = conn
        .randr_get_output_primary(root)
        .ok()?
        .reply()
        .ok()?
        .output;
    if primary == 0 {
        return None;
    }
    let info = conn.randr_get_output_info(primary, 0).ok()?.reply().ok()?;
    Some(String::from_utf8_lossy(&info.name).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screens(on: &[&str], backlight: bool, ddc: &[(&str, u32)]) -> Screens {
        Screens {
            on: on.iter().map(|name| (*name).to_owned()).collect(),
            backlight: backlight.then(|| ("amdgpu_bl1".to_owned(), 255)),
            ddc: ddc
                .iter()
                .map(|(connector, bus)| DdcMonitor {
                    connector: (*connector).to_owned(),
                    bus: *bus,
                    max: 100,
                    value: 50,
                })
                .collect(),
            cache: PathBuf::from("/nonexistent/overlay-ddc.json"),
        }
    }

    #[test]
    fn built_in_panels() {
        assert!(is_internal("eDP-1") && is_internal("eDP1") && is_internal("LVDS-1"));
        assert!(is_internal("DSI-1") && !is_internal("HDMI-A-1") && !is_internal("DP-2"));
        assert_eq!(kind_of("card1-eDP-1"), "edp");
        assert_eq!(kind_of("eDP1"), "edp");
        assert_eq!(kind_of("HDMI-A-0"), "hdmi");
    }

    #[test]
    fn picks_the_primary_screen() {
        // a laptop alone, whatever XWayland calls its output
        let laptop = screens(&["card1-eDP-1"], true, &[]);
        assert!(!laptop.ambiguous());
        assert!(matches!(
            laptop.resolve(None),
            Some(Screen::Backlight { .. })
        ));
        assert!(matches!(
            laptop.resolve(Some("XWAYLAND0")),
            Some(Screen::Backlight { .. })
        ));
        // a laptop and a TV: whichever the overlay is on
        let both = screens(
            &["card1-HDMI-A-1", "card1-eDP-1"],
            true,
            &[("card1-HDMI-A-1", 5)],
        );
        assert!(both.ambiguous());
        assert!(matches!(
            both.resolve(Some("eDP-1")),
            Some(Screen::Backlight { .. })
        ));
        assert!(matches!(
            both.resolve(Some("HDMI-A-1")),
            Some(Screen::Ddc {
                monitor: DdcMonitor { bus: 5, .. },
                ..
            })
        ));
        // X11's amdgpu counts from 0
        assert!(matches!(
            both.resolve(Some("HDMI-A-0")),
            Some(Screen::Ddc { .. })
        ));
        // a desktop whose monitor is not in the cache
        let desktop = screens(&["card0-DP-1"], false, &[]);
        assert!(desktop.resolve(None).is_none());
        assert!(desktop.needs_detection(None));
        assert!(!laptop.needs_detection(None));
    }

    #[test]
    fn ddcutil_answers() {
        let detect = "Display 1\n   I2C bus:  /dev/i2c-5\n   DRM connector:  card1-HDMI-A-1\n\
                      \nDisplay 2\n   I2C bus:  /dev/i2c-7\n   DRM connector:  card1-DP-2\n";
        assert_eq!(
            parse_detect(detect),
            [
                (5, "card1-HDMI-A-1".to_owned()),
                (7, "card1-DP-2".to_owned())
            ]
        );
        assert_eq!(parse_vcp("VCP 10 C 50 100\n"), Some((50, 100)));
        assert_eq!(parse_vcp("VCP 10 ERR"), None);
        assert!(same_connector("card1-HDMI-A-1", "HDMI-A-1"));
        assert!(same_connector("card1-HDMI-A-1", "HDMI-1"));
        assert!(!same_connector("card1-DP-2", "DP-1"));
    }
}
