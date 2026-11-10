//! The brightness of the primary screen, on Wayland and on X11.
//!
//! Which screen is the primary one: X11's (`RandR`'s primary output, which
//! KWin also sets for `XWayland`), else KDE's (`kscreen-doctor`, the output
//! with priority 1), else the first enabled output of a wlroots compositor
//! (`wlr-randr`). Its brightness is then
//!
//! * a laptop's built-in panel (eDP, LVDS, DSI): the kernel's backlight,
//!   read from sysfs and set through logind (`SetBrightness`, allowed to the
//!   active session without root), else `brightnessctl`;
//! * an external monitor: DDC/CI through `ddcutil` (VCP feature 0x10), when
//!   it is installed and the monitor answers.
//!
//! Without knowing the primary screen, a built-in panel that is on wins.

use std::fs;
use std::path::{Path, PathBuf};

use super::{output, run};

/// The lowest brightness a slider at 0 sets on a backlight: some panels go
/// black at 0, which would leave no way to see the slider again.
const MIN_BACKLIGHT: f32 = 0.05;
const BACKLIGHT_DIR: &str = "/sys/class/backlight";
const DRM_DIR: &str = "/sys/class/drm";

/// The screen whose brightness the slider changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// A kernel backlight (`/sys/class/backlight/<name>`).
    Backlight { name: String, max: u32 },
    /// A monitor reached by `ddcutil` (its display number, when several).
    Ddc { display: Option<u32>, max: u32 },
}

/// A built-in panel's connector name (DRM's or X11's: `eDP-1`, `eDP1`, `LVDS-1`, `DSI-1`).
fn is_internal(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["edp", "lvds", "dsi"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

impl Screen {
    /// The primary screen's brightness control, if it has one that can be
    /// reached.
    pub fn detect(primary: Option<&str>) -> Option<Self> {
        let internal = primary.map_or_else(internal_panel_on, is_internal);
        log::debug!("primary output: {primary:?} (built-in: {internal})");
        let screen = if internal { backlight() } else { ddc(primary) };
        if screen.is_none() {
            log::info!("no brightness control for the primary screen ({primary:?})");
        }
        screen
    }

    /// The brightness, 0 to 1 (of the slider: a backlight's lowest step is
    /// its 0).
    pub fn get(&self) -> Option<f32> {
        match self {
            Self::Backlight { name, max } => {
                let dir = Path::new(BACKLIGHT_DIR).join(name);
                let raw = read_number(&dir.join("actual_brightness"))
                    .or_else(|| read_number(&dir.join("brightness")))?;
                let fraction = raw as f32 / *max as f32;
                Some(((fraction - MIN_BACKLIGHT) / (1.0 - MIN_BACKLIGHT)).clamp(0.0, 1.0))
            }
            Self::Ddc { display, max } => {
                let answer = ddcutil(*display, &["getvcp", "10", "--brief"])?;
                let (current, _) = parse_vcp(&answer)?;
                Some((current as f32 / *max as f32).clamp(0.0, 1.0))
            }
        }
    }

    pub fn set(&self, value: f32) {
        let value = value.clamp(0.0, 1.0);
        match self {
            Self::Backlight { name, max } => {
                let fraction = MIN_BACKLIGHT + value * (1.0 - MIN_BACKLIGHT);
                let raw = ((fraction * *max as f32).round() as u32).clamp(1, *max);
                let raw_text = raw.to_string();
                let through_logind = run(
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
                );
                if !through_logind {
                    run("brightnessctl", &["--quiet", "-d", name, "set", &raw_text]);
                }
            }
            Self::Ddc { display, max } => {
                let raw = ((value * *max as f32).round() as u32).to_string();
                ddcutil(*display, &["setvcp", "10", &raw, "--noverify"]);
            }
        }
    }
}

/// Whether a built-in panel is connected and on (when the primary screen is unknown).
fn internal_panel_on() -> bool {
    let Ok(entries) = fs::read_dir(DRM_DIR) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        // card1-eDP-1
        let connector = name.split_once('-').map_or("", |(_, connector)| connector);
        is_internal(connector)
            && fs::read_to_string(entry.path().join("enabled"))
                .is_ok_and(|enabled| enabled.trim() == "enabled")
    })
}

fn read_number(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The kernel's backlight: the firmware's first, then the platform's, then
/// the GPU's (raw), as the kernel's documentation recommends.
fn backlight() -> Option<Screen> {
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
    let (_, name, max) = found.into_iter().next()?;
    log::info!("brightness: backlight {name}");
    Some(Screen::Backlight { name, max })
}

/// An external monitor through `ddcutil`: the one on the primary output's
/// connector, or the only one.
fn ddc(primary: Option<&str>) -> Option<Screen> {
    let detected = output("ddcutil", &["detect", "--brief"])?;
    let displays = parse_detect(&detected);
    let display = match (primary, displays.as_slice()) {
        (_, []) => return None,
        (_, [(number, _)]) | (None, [(number, _), ..]) => *number,
        (Some(primary), several) => several
            .iter()
            .find(|(_, connector)| same_connector(connector, primary))
            .map(|(number, _)| *number)?,
    };
    let display = (displays.len() > 1).then_some(display);
    let (_, max) = parse_vcp(&ddcutil(display, &["getvcp", "10", "--brief"])?)?;
    log::info!("brightness: DDC/CI, display {display:?}");
    Some(Screen::Ddc {
        display,
        max: max.max(1),
    })
}

fn ddcutil(display: Option<u32>, args: &[&str]) -> Option<String> {
    let number = display.map(|number| number.to_string());
    let mut full: Vec<&str> = Vec::new();
    if let Some(number) = &number {
        full.extend(["--display", number.as_str()]);
    }
    full.extend_from_slice(args);
    let answer = output("ddcutil", &full);
    if answer.is_none() && args.first() == Some(&"setvcp") {
        log::warn!("ddcutil could not set the brightness");
    }
    answer
}

/// `ddcutil detect --brief`: each display's number and DRM connector.
fn parse_detect(text: &str) -> Vec<(u32, String)> {
    let mut displays = Vec::new();
    let mut number = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Display ") {
            number = rest.trim().parse().ok();
        } else if let Some(connector) = line.strip_prefix("DRM connector:")
            && let Some(number) = number.take()
        {
            displays.push((number, connector.trim().to_owned()));
        }
    }
    displays
}

/// `card1-HDMI-A-1` and `HDMI-A-1` (Wayland) or `HDMI-1` (X11's modesetting).
fn same_connector(drm: &str, output: &str) -> bool {
    let connector = drm.split_once('-').map_or(drm, |(_, connector)| connector);
    let simple = |name: &str| name.replace("-A-", "-").to_ascii_lowercase();
    connector.eq_ignore_ascii_case(output) || simple(connector) == simple(output)
}

/// `VCP 10 C 50 100`: the current value and the maximum.
fn parse_vcp(text: &str) -> Option<(u32, u32)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        ["VCP", "10", "C", current, max, ..] => Some((current.parse().ok()?, max.parse().ok()?)),
        _ => None,
    }
}

/// The primary output's name, as the display server knows it.
pub fn primary_output() -> Option<String> {
    x11_primary()
        .or_else(kscreen_primary)
        .or_else(wlr_randr_first)
}

fn x11_primary() -> Option<String> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::randr::ConnectionExt as _;

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

fn kscreen_primary() -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(&output("kscreen-doctor", &["-j"])?).ok()?;
    json.get("outputs")?
        .as_array()?
        .iter()
        .filter(|output| output.get("enabled").and_then(serde_json::Value::as_bool) == Some(true))
        .find(|output| {
            output.get("priority").and_then(serde_json::Value::as_u64) == Some(1)
                || output.get("primary").and_then(serde_json::Value::as_bool) == Some(true)
        })
        .and_then(|output| output.get("name")?.as_str().map(str::to_owned))
}

fn wlr_randr_first() -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(&output("wlr-randr", &["--json"])?).ok()?;
    json.as_array()?
        .iter()
        .find(|output| output.get("enabled").and_then(serde_json::Value::as_bool) == Some(true))
        .and_then(|output| output.get("name")?.as_str().map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_panels() {
        assert!(is_internal("eDP-1") && is_internal("eDP1") && is_internal("LVDS-1"));
        assert!(is_internal("DSI-1") && !is_internal("HDMI-A-1") && !is_internal("DP-2"));
    }

    #[test]
    fn ddcutil_answers() {
        let detect = "Display 1\n   I2C bus:  /dev/i2c-5\n   DRM connector:  card1-HDMI-A-1\n\
                      \nDisplay 2\n   I2C bus:  /dev/i2c-7\n   DRM connector:  card1-DP-2\n";
        assert_eq!(
            parse_detect(detect),
            [
                (1, "card1-HDMI-A-1".to_owned()),
                (2, "card1-DP-2".to_owned())
            ]
        );
        assert_eq!(parse_vcp("VCP 10 C 50 100\n"), Some((50, 100)));
        assert_eq!(parse_vcp("VCP 10 ERR"), None);
        assert!(same_connector("card1-HDMI-A-1", "HDMI-A-1"));
        assert!(same_connector("card1-HDMI-A-1", "HDMI-1"));
        assert!(!same_connector("card1-DP-2", "DP-1"));
    }
}
