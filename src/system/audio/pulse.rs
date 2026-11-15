//! `PulseAudio` (and `PipeWire`'s `PulseAudio` server): the sinks and cards read
//! with `pactl --format=json`, changed with `pactl`, and the changes followed
//! with `pactl subscribe`.
//!
//! `pactl` writes its messages in the user's language, `subscribe`'s events
//! included: it is always run with `LC_ALL=C`.

use std::collections::HashMap;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use super::devices::{Card, Output, Profile, Snapshot};

/// What a `pactl subscribe` event is about, when the menu cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facility {
    /// An output: its volume, mute or state changed, or it came or went.
    Sink,
    Card,
    /// The default output changed.
    Server,
}

/// `Event 'change' on sink #52`; None for what does not matter (every
/// `pactl` is a client that comes and goes, streams start and stop...).
pub fn parse_event(line: &str) -> Option<Facility> {
    let (_, rest) = line.strip_prefix("Event '")?.split_once("' on ")?;
    match rest.split(" #").next()?.trim() {
        "sink" => Some(Facility::Sink),
        "card" => Some(Facility::Card),
        "server" => Some(Facility::Server),
        _ => None,
    }
}

/// A `pactl` command's output, when it succeeds.
fn pactl(args: &[&str]) -> Option<String> {
    let output = Command::new("pactl")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        log::debug!("pactl {args:?}: {}", output.status);
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// A `pactl` command for its effect; true when it succeeded.
fn pactl_run(args: &[&str]) -> bool {
    pactl(args).is_some()
}

/// Whether `pactl` can write JSON (`PulseAudio` 16 and later).
pub fn supports_json() -> bool {
    pactl(&["--format=json", "info"]).is_some()
}

#[derive(Deserialize)]
struct Info {
    default_sink_name: Option<String>,
}

#[derive(Deserialize)]
struct RawSink {
    name: String,
    description: Option<String>,
    #[serde(default)]
    properties: HashMap<String, Value>,
}

#[derive(Deserialize)]
struct RawCard {
    index: u32,
    name: String,
    #[serde(default)]
    properties: HashMap<String, Value>,
    #[serde(default)]
    profiles: HashMap<String, RawProfile>,
    active_profile: Option<String>,
}

#[derive(Deserialize)]
struct RawProfile {
    description: Option<String>,
    #[serde(default)]
    priority: i64,
    #[serde(default = "yes")]
    available: bool,
}

const fn yes() -> bool {
    true
}

fn text(props: &HashMap<String, Value>, key: &str) -> Option<String> {
    match props.get(key)? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The card a sink belongs to. `pactl` does not say, so it is matched by
/// what both carry: `PipeWire`'s `PulseAudio` server gives the sink its device
/// id (the card's index), `PulseAudio`'s ALSA sinks the ALSA card's number,
/// and Bluetooth ones the device's path.
fn card_of<'a>(sink: &RawSink, cards: &'a [RawCard]) -> Option<&'a RawCard> {
    cards.iter().find(|card| {
        let same = |key: &str| {
            matches!(
                (text(&sink.properties, key), text(&card.properties, key)),
                (Some(a), Some(b)) if a == b
            )
        };
        text(&sink.properties, "device.id").is_some_and(|id| id == card.index.to_string())
            || same("alsa.card")
            || same("bluez.path")
    })
}

/// The outputs and cards from `pactl --format=json` lists of sinks and
/// cards, and its `info`.
pub fn parse_snapshot(sinks: &str, cards: &str, info: &str) -> Option<Snapshot> {
    let default = serde_json::from_str::<Info>(info).ok()?.default_sink_name;
    let sinks: Vec<RawSink> = serde_json::from_str(sinks).ok()?;
    let cards: Vec<RawCard> = serde_json::from_str(cards).ok()?;
    let outputs = sinks
        .iter()
        .map(|sink| Output {
            id: sink.name.clone(),
            label: sink
                .description
                .clone()
                .unwrap_or_else(|| sink.name.clone()),
            default: default.as_deref() == Some(sink.name.as_str()),
            card: card_of(sink, &cards).map(|card| card.name.clone()),
        })
        .collect();
    let cards = cards
        .into_iter()
        .map(|card| {
            let mut profiles: Vec<_> = card.profiles.into_iter().collect();
            // best first, then by name: a map has no order
            profiles.sort_by(|a, b| b.1.priority.cmp(&a.1.priority).then_with(|| a.0.cmp(&b.0)));
            Card {
                label: text(&card.properties, "device.description")
                    .or_else(|| text(&card.properties, "device.product.name"))
                    .unwrap_or_else(|| card.name.clone()),
                profiles: profiles
                    .into_iter()
                    .map(|(name, profile)| Profile {
                        active: card.active_profile.as_deref() == Some(name.as_str()),
                        available: profile.available,
                        off: name == "off",
                        label: profile.description.unwrap_or_else(|| name.clone()),
                        id: name,
                    })
                    .collect(),
                id: card.name,
            }
        })
        .collect();
    Some(Snapshot { outputs, cards })
}

/// The outputs and cards now (three `pactl` runs).
pub fn snapshot() -> Option<Snapshot> {
    parse_snapshot(
        &pactl(&["--format=json", "list", "sinks"])?,
        &pactl(&["--format=json", "list", "cards"])?,
        &pactl(&["--format=json", "info"])?,
    )
}

/// Make a sink (its name) the default output, and move what is playing to
/// it, as the desktops' sound settings do (the server leaves playing streams
/// where they are); true when the default changed.
pub fn set_default(name: &str) -> bool {
    if !pactl_run(&["set-default-sink", name]) {
        return false;
    }
    if let Some(streams) = pactl(&["list", "short", "sink-inputs"]) {
        for id in streams
            .lines()
            .filter_map(|line| line.split('\t').next())
            .filter(|id| id.parse::<u32>().is_ok())
        {
            // a stream that cannot be moved stays: the others go on
            pactl_run(&["move-sink-input", id, name]);
        }
    }
    true
}

/// Switch a card (its name) to a profile (its name); true when done.
pub fn set_profile(card: &str, profile: &str) -> bool {
    pactl_run(&["set-card-profile", card, profile])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_events_that_matter() {
        assert_eq!(
            parse_event("Event 'change' on sink #52"),
            Some(Facility::Sink)
        );
        assert_eq!(parse_event("Event 'new' on sink #7"), Some(Facility::Sink));
        assert_eq!(
            parse_event("Event 'remove' on card #3"),
            Some(Facility::Card)
        );
        assert_eq!(
            parse_event("Event 'change' on server"),
            Some(Facility::Server)
        );
        // streams and clients come and go with every `pactl`
        assert_eq!(parse_event("Event 'new' on client #50"), None);
        assert_eq!(parse_event("Event 'change' on sink-input #9"), None);
        assert_eq!(parse_event("Event 'change' on source #2"), None);
        assert_eq!(parse_event("something else"), None);
    }

    const SINKS: &str = r#"[
      {"index":1,"state":"RUNNING","name":"alsa_output.pci-0000_00_1f.3.analog-stereo","description":"Built-in Audio Analog Stereo","properties":{"alsa.card":"0","device.string":"front:0"}},
      {"index":2,"state":"SUSPENDED","name":"alsa_output.pci-0000_01_00.1.hdmi-stereo","description":"HDMI / DisplayPort","properties":{"alsa.card":"1"}},
      {"index":3,"name":"bluez_output.AA_BB.1","properties":{"bluez.path":"/org/bluez/hci0/dev_AA_BB"}},
      {"index":4,"name":"virtual","description":"Virtual","properties":{}}]"#;
    const CARDS: &str = r#"[
      {"index":10,"name":"alsa_card.pci-0000_00_1f.3","properties":{"alsa.card":"0","device.description":"Built-in Audio"},
       "profiles":{"off":{"description":"Off","sinks":0,"sources":0,"priority":0,"available":true},
                   "output:analog-stereo":{"description":"Analog Stereo Output","sinks":1,"sources":0,"priority":6500,"available":true},
                   "output:hdmi-stereo":{"description":"Digital Stereo (HDMI) Output","sinks":1,"sources":0,"priority":5900,"available":false}},
       "active_profile":"output:analog-stereo"},
      {"index":11,"name":"alsa_card.pci-0000_01_00.1","properties":{"alsa.card":"1"},"profiles":{},"active_profile":null},
      {"index":12,"name":"bluez_card.AA_BB","properties":{"bluez.path":"/org/bluez/hci0/dev_AA_BB","device.description":"Headset"},"profiles":{},"active_profile":null}]"#;
    const INFO: &str = r#"{"server_name":"PulseAudio","default_sink_name":"alsa_output.pci-0000_00_1f.3.analog-stereo"}"#;

    #[test]
    fn reads_the_outputs_and_their_cards() {
        let snapshot = parse_snapshot(SINKS, CARDS, INFO).unwrap();
        let cards: Vec<_> = snapshot
            .outputs
            .iter()
            .map(|output| output.card.as_deref())
            .collect();
        assert_eq!(
            cards,
            [
                Some("alsa_card.pci-0000_00_1f.3"),
                Some("alsa_card.pci-0000_01_00.1"),
                Some("bluez_card.AA_BB"),
                None
            ]
        );
        assert!(snapshot.outputs[0].default && !snapshot.outputs[1].default);
        // no description: the name
        assert_eq!(snapshot.outputs[2].label, "bluez_output.AA_BB.1");
    }

    #[test]
    fn reads_the_profiles_best_first() {
        let snapshot = parse_snapshot(SINKS, CARDS, INFO).unwrap();
        let card = snapshot.default_card().unwrap();
        assert_eq!(card.label, "Built-in Audio");
        let ids: Vec<_> = card.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["output:analog-stereo", "output:hdmi-stereo", "off"]);
        assert!(card.profiles[0].active);
        assert!(!card.profiles[1].available);
        assert!(card.profiles[2].off);
        // no description for the card: its name
        assert_eq!(snapshot.cards[1].label, "alsa_card.pci-0000_01_00.1");
    }

    #[test]
    fn matches_pipewire_pulse_sinks_by_device_id() {
        let sinks = r#"[{"name":"s","description":"S","properties":{"device.id":"10"}}]"#;
        let snapshot = parse_snapshot(sinks, CARDS, INFO).unwrap();
        assert_eq!(
            snapshot.outputs[0].card.as_deref(),
            Some("alsa_card.pci-0000_00_1f.3")
        );
    }

    #[test]
    fn rejects_what_is_not_json() {
        assert!(parse_snapshot("Sink #1", CARDS, INFO).is_none());
    }
}
