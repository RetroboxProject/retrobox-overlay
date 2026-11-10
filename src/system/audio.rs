//! The system's volume: the default output of whatever sound server runs,
//! `PipeWire` (`wpctl`) or `PulseAudio` (`pactl`, which also works with
//! `PipeWire`'s `PulseAudio` server).

use super::{output, run};

/// The sound server's command line tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mixer {
    Wpctl,
    Pactl,
}

const WPCTL_SINK: &str = "@DEFAULT_AUDIO_SINK@";
const PACTL_SINK: &str = "@DEFAULT_SINK@";

impl Mixer {
    /// The first tool that can read the default output's volume.
    pub fn detect() -> Option<Self> {
        let mixer = [Self::Wpctl, Self::Pactl]
            .into_iter()
            .find(|mixer| mixer.get().is_some());
        if mixer.is_none() {
            log::info!("no PipeWire or PulseAudio output found: no volume slider");
        }
        mixer
    }

    /// The default output's volume, 0 to 1 (0 when muted).
    pub fn get(self) -> Option<f32> {
        match self {
            Self::Wpctl => parse_wpctl(&output("wpctl", &["get-volume", WPCTL_SINK])?),
            Self::Pactl => {
                let volume = parse_pactl(&output("pactl", &["get-sink-volume", PACTL_SINK])?)?;
                let muted = output("pactl", &["get-sink-mute", PACTL_SINK])
                    .is_some_and(|mute| mute.contains("yes"));
                Some(if muted { 0.0 } else { volume })
            }
        }
    }

    /// Set the default output's volume (unmuted, unless it is 0).
    pub fn set(self, value: f32) {
        let percent = (value.clamp(0.0, 1.0) * 100.0).round() as u32;
        let mute = if percent == 0 { "1" } else { "0" };
        match self {
            Self::Wpctl => {
                let volume = format!("{percent}%");
                run("wpctl", &["set-volume", "-l", "1.0", WPCTL_SINK, &volume]);
                run("wpctl", &["set-mute", WPCTL_SINK, mute]);
            }
            Self::Pactl => {
                let volume = format!("{percent}%");
                run("pactl", &["set-sink-volume", PACTL_SINK, &volume]);
                run("pactl", &["set-sink-mute", PACTL_SINK, mute]);
            }
        }
    }
}

/// `Volume: 0.40` or `Volume: 0.40 [MUTED]`.
fn parse_wpctl(text: &str) -> Option<f32> {
    let rest = text.trim().strip_prefix("Volume:")?;
    let value: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some(if rest.contains("[MUTED]") {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    })
}

/// `Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: ...`: the
/// first channel's percentage.
fn parse_pactl(text: &str) -> Option<f32> {
    let percent = text
        .split('/')
        .map(str::trim)
        .find_map(|part| part.strip_suffix('%')?.trim().parse::<f32>().ok())?;
    Some((percent / 100.0).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wpctl() {
        assert_eq!(parse_wpctl("Volume: 0.40\n"), Some(0.4));
        assert_eq!(parse_wpctl("Volume: 0.40 [MUTED]\n"), Some(0.0));
        assert_eq!(parse_wpctl("Volume: 1.50"), Some(1.0));
        assert_eq!(parse_wpctl("nothing"), None);
    }

    #[test]
    fn reads_pactl() {
        let text = "Volume: front-left: 26214 /  40% / -23.88 dB,   \
                    front-right: 26214 /  40% / -23.88 dB\n";
        assert_eq!(parse_pactl(text), Some(0.4));
        assert_eq!(parse_pactl("Volume: none"), None);
    }
}
