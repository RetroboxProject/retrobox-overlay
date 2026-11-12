//! The brightness of a built-in screen (a laptop's eDP or LVDS panel, a
//! handheld's DSI one, the Switch's), when it is the primary screen.
//!
//! External monitors are left alone (DDC/CI is slow and, on NVIDIA, stalls
//! the display on every exchange): without a built-in screen that is the
//! primary one, there is no brightness slider at all.
//!
//! The primary screen is the only one that is on (`/sys/class/drm`), else
//! the display settings' primary one: X11's (`RandR`), KDE's on Wayland
//! (`kscreen-doctor`, priority 1; read in a thread, as it takes a moment),
//! else, on other Wayland compositors (which have no primary screen), the
//! one the overlay shows on.
//!
//! The backlight is written in sysfs when allowed (retrobox-core's udev rule
//! lets the `video` group), else through logind (`SetBrightness`, allowed
//! to the active session), else `brightnessctl`; the first way that works
//! is kept. NVIDIA's `nvidia_0` refuses a change while it is still fading to
//! the last one (`EPROTO`): that change is skipped, the next one comes soon.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::{output, run};

/// The lowest brightness a slider at 0 sets: some panels go black at 0,
/// which would leave no way to see the slider again.
const MIN_BACKLIGHT: f32 = 0.05;
const BACKLIGHT_DIR: &str = "/sys/class/backlight";
const DRM_DIR: &str = "/sys/class/drm";

/// A built-in panel's connector name (DRM's or X11's: `eDP-1`, `eDP1`,
/// `LVDS-1`, `DSI-1`).
fn is_internal(name: &str) -> bool {
    let name = strip_card(name).to_ascii_lowercase();
    ["edp", "lvds", "dsi"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// `card1-eDP-1` -> `eDP-1` (an output's name stays as it is).
fn strip_card(name: &str) -> &str {
    match name.split_once('-') {
        Some((card, connector)) if card.starts_with("card") => connector,
        _ => name,
    }
}

/// How a backlight is set: the first way that works is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Unknown,
    Sysfs,
    Logind,
    Brightnessctl,
}

/// A built-in screen's backlight (`/sys/class/backlight/<name>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    name: String,
    max: u32,
    method: Method,
    /// The value last written.
    written: Option<u32>,
}

/// What is known of the screens, without running anything.
#[derive(Debug, Default)]
pub struct Screens {
    /// The connectors that are on (`card1-eDP-1`).
    on: Vec<String>,
    backlight: Option<(String, u32)>,
}

impl Screens {
    pub fn scan() -> Self {
        Self {
            on: connectors_on(),
            backlight: backlight(),
        }
    }

    /// Whether the primary screen has to be told (more than one is on).
    pub fn ambiguous(&self) -> bool {
        self.on.len() > 1
    }

    /// The backlight of the primary screen (`primary`: its output's name,
    /// when more than one screen is on), if it is a built-in one.
    pub fn resolve(&self, primary: Option<&str>) -> Option<Screen> {
        let internal = match (self.on.as_slice(), primary) {
            // nothing in sysfs (no DRM): a backlight is a built-in panel's
            ([], _) => true,
            ([only], _) => is_internal(only),
            (_, Some(primary)) => !primary.starts_with("XWAYLAND") && is_internal(primary),
            (_, None) => false,
        };
        log::info!("brightness: screens on {:?}, primary {primary:?}", self.on);
        if !internal {
            log::info!("brightness: the primary screen is not a built-in one: no slider");
            return None;
        }
        let (name, max) = self.backlight.clone()?;
        log::info!("brightness: backlight {name}");
        Some(Screen {
            name,
            max,
            method: Method::Unknown,
            written: None,
        })
    }
}

impl Screen {
    /// The brightness, 0 to 1 (a slider at 0 is the lowest step).
    pub fn get(&self) -> Option<f32> {
        let dir = Path::new(BACKLIGHT_DIR).join(&self.name);
        let raw = read_number(&dir.join("brightness"))
            .or_else(|| read_number(&dir.join("actual_brightness")))?;
        let fraction = raw as f32 / self.max as f32;
        Some(((fraction - MIN_BACKLIGHT) / (1.0 - MIN_BACKLIGHT)).clamp(0.0, 1.0))
    }

    /// The backlight's own steps a value is (raw units).
    pub fn raw(&self, value: f32) -> u32 {
        let fraction = MIN_BACKLIGHT + value.clamp(0.0, 1.0) * (1.0 - MIN_BACKLIGHT);
        ((fraction * self.max as f32).round() as u32).clamp(1, self.max)
    }

    /// How fine the backlight is: a value a step apart.
    pub fn step(&self) -> f32 {
        1.0 / (self.max as f32 * (1.0 - MIN_BACKLIGHT))
    }

    /// Set the brightness; false when the driver refused (still fading).
    pub fn set(&mut self, value: f32) -> bool {
        let raw = self.raw(value);
        if self.written == Some(raw) {
            return true;
        }
        let done = set_backlight(&self.name, raw, &mut self.method);
        if done {
            self.written = Some(raw);
        }
        done
    }
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
        return works();
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

/// The display settings' primary screen, when it takes asking: X11's
/// (`RandR`, outside Wayland: `XWayland`'s outputs have names of their
/// own), else KDE's on Wayland (`kscreen-doctor`). None: none set, or
/// nothing to ask.
pub fn primary_output() -> Option<String> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty());
    if wayland {
        kscreen_primary()
    } else {
        x11_primary()
    }
}

/// Whether `primary_output` has a way to know on this desktop (else, on
/// Wayland, the overlay's own output stands for it).
pub fn primary_known() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty());
    !wayland || super::installed("kscreen-doctor")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn screens(on: &[&str]) -> Screens {
        Screens {
            on: on.iter().map(|name| (*name).to_owned()).collect(),
            backlight: Some(("nvidia_0".to_owned(), 100)),
        }
    }

    #[test]
    fn built_in_panels() {
        assert!(is_internal("eDP-1") && is_internal("eDP1") && is_internal("card1-LVDS-1"));
        assert!(is_internal("DSI-1") && !is_internal("HDMI-A-1") && !is_internal("DP-2"));
    }

    #[test]
    fn only_a_primary_built_in_screen() {
        // a laptop or a Switch alone
        assert!(screens(&["card1-eDP-1"]).resolve(None).is_some());
        assert!(
            screens(&["card0-DSI-1"])
                .resolve(Some("XWAYLAND0"))
                .is_some()
        );
        // an external monitor alone: no slider
        assert!(screens(&["card1-HDMI-A-1"]).resolve(None).is_none());
        // both: the primary one decides
        let both = screens(&["card1-HDMI-A-1", "card1-eDP-1"]);
        assert!(both.ambiguous());
        assert!(both.resolve(Some("eDP-1")).is_some());
        assert!(both.resolve(Some("HDMI-A-1")).is_none());
        assert!(both.resolve(None).is_none());
    }

    #[test]
    fn steps() {
        let screen = screens(&["card1-eDP-1"]).resolve(None).unwrap();
        assert_eq!(screen.raw(0.0), 5);
        assert_eq!(screen.raw(1.0), 100);
    }
}
