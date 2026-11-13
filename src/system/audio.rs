//! The system's volume: the default output of whatever sound server runs,
//! `PipeWire` (`wpctl`) or `PulseAudio` (`pactl`, which also works with
//! `PipeWire`'s `PulseAudio` server).

use super::{output, run};

/// The sound server's command line tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Wpctl,
    Pactl,
}

/// The default output, through its tool; whether it is muted is kept so a
/// change runs one program, not two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mixer {
    tool: Tool,
    muted: Option<bool>,
}

const WPCTL_SINK: &str = "@DEFAULT_AUDIO_SINK@";
const PACTL_SINK: &str = "@DEFAULT_SINK@";

impl Mixer {
    /// The first tool that can read the default output's volume.
    pub fn detect() -> Option<Self> {
        let mixer = [Tool::Wpctl, Tool::Pactl]
            .into_iter()
            .map(|tool| Self { tool, muted: None })
            .find(|mixer| mixer.read().is_some());
        if mixer.is_none() {
            log::info!("no PipeWire or PulseAudio output found: no volume slider");
        }
        mixer
    }

    /// The default output's volume, 0 to 1, and whether it is muted.
    pub fn read(self) -> Option<(f32, bool)> {
        match self.tool {
            Tool::Wpctl => parse_wpctl(&output("wpctl", &["get-volume", WPCTL_SINK])?),
            Tool::Pactl => {
                let volume = parse_pactl(&output("pactl", &["get-sink-volume", PACTL_SINK])?)?;
                let muted = output("pactl", &["get-sink-mute", PACTL_SINK])
                    .is_some_and(|mute| mute.contains("yes"));
                Some((volume, muted))
            }
        }
    }

    /// Mute or unmute the default output, its volume kept.
    pub fn set_mute(&mut self, muted: bool) {
        let mute = if muted { "1" } else { "0" };
        let done = match self.tool {
            Tool::Wpctl => run("wpctl", &["set-mute", WPCTL_SINK, mute]),
            Tool::Pactl => run("pactl", &["set-sink-mute", PACTL_SINK, mute]),
        };
        if done {
            self.muted = Some(muted);
        }
    }

    /// Set the default output's volume (unmuted, unless it is 0; the mute
    /// only changes when it has to).
    pub fn set(&mut self, value: f32) {
        let percent = (value.clamp(0.0, 1.0) * 100.0).round() as u32;
        let volume = format!("{percent}%");
        let muted = percent == 0;
        let mute = if muted { "1" } else { "0" };
        let (tool, set_volume, set_mute): (&str, Vec<&str>, Vec<&str>) = match self.tool {
            Tool::Wpctl => (
                "wpctl",
                vec!["set-volume", "-l", "1.0", WPCTL_SINK, &volume],
                vec!["set-mute", WPCTL_SINK, mute],
            ),
            Tool::Pactl => (
                "pactl",
                vec!["set-sink-volume", PACTL_SINK, &volume],
                vec!["set-sink-mute", PACTL_SINK, mute],
            ),
        };
        // muting first, unmuting after: never a burst at the old volume
        if muted && self.muted != Some(true) && run(tool, &set_mute) {
            self.muted = Some(true);
        }
        run(tool, &set_volume);
        if !muted && self.muted != Some(false) && run(tool, &set_mute) {
            self.muted = Some(false);
        }
    }
}

/// `Volume: 0.40` or `Volume: 0.40 [MUTED]`.
fn parse_wpctl(text: &str) -> Option<(f32, bool)> {
    let rest = text.trim().strip_prefix("Volume:")?;
    let value: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some((value.clamp(0.0, 1.0), rest.contains("[MUTED]")))
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
        assert_eq!(parse_wpctl("Volume: 0.40\n"), Some((0.4, false)));
        assert_eq!(parse_wpctl("Volume: 0.40 [MUTED]\n"), Some((0.4, true)));
        assert_eq!(parse_wpctl("Volume: 1.50"), Some((1.0, false)));
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
