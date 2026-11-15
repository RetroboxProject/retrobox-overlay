//! `PipeWire`: its graph, followed through `pw-dump --monitor` (an initial dump
//! of every object, then each object that changes, whole, as JSON arrays),
//! and changed with `wpctl`.
//!
//! Only three kinds of object matter: the audio sinks (the outputs), the
//! audio devices (the cards, whose profiles are among their params) and the
//! `default` metadata (which sink is the default). Each one that changes is
//! parsed on its own, so an object the parser does not understand is skipped
//! and nothing else is lost.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;

use super::devices::{Card, Output, Profile, Snapshot};
use crate::system::run;

/// An audio sink, from its node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sink {
    name: String,
    label: String,
    /// The id of the device (card) it belongs to.
    device: Option<u32>,
}

/// An audio device, from its object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    label: String,
    profiles: Vec<Profile>,
    /// The index of the profile in use.
    active: Option<u32>,
}

impl Device {
    /// An update that leaves out some of the params (only one changed) keeps
    /// what the last one said about the others.
    fn keep(&mut self, old: &Self) {
        if self.profiles.is_empty() {
            self.profiles.clone_from(&old.profiles);
        }
        if self.active.is_none() {
            self.active = old.active;
        }
        for profile in &mut self.profiles {
            profile.active = self
                .active
                .is_some_and(|active| profile.id == active.to_string());
        }
    }
}

/// A change of the graph, as far as the menu cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Sink(u32, Sink),
    Device(u32, Device),
    /// The default sink's node name (None: none).
    Default(Option<String>),
    /// An object is gone (whatever it was).
    Removed(u32),
}

/// What a change touched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Touched {
    /// The volume may have changed (a sink or its device changed, or another
    /// sink became the default).
    pub volume: bool,
    /// What the menu lists changed.
    pub devices: bool,
}

/// The audio objects of the graph.
#[derive(Debug, Default)]
pub struct Graph {
    sinks: BTreeMap<u32, Sink>,
    devices: BTreeMap<u32, Device>,
    default: Option<String>,
}

impl Graph {
    /// Take a change in.
    pub fn update(&mut self, update: Update) -> Touched {
        match update {
            Update::Sink(id, sink) => Touched {
                volume: true,
                devices: self.sinks.insert(id, sink.clone()) != Some(sink),
            },
            Update::Device(id, mut device) => {
                if let Some(old) = self.devices.get(&id) {
                    device.keep(old);
                }
                Touched {
                    volume: true,
                    devices: self.devices.insert(id, device.clone()) != Some(device),
                }
            }
            Update::Default(name) => {
                let changed = self.default != name;
                self.default = name;
                Touched {
                    volume: changed,
                    devices: changed,
                }
            }
            Update::Removed(id) => {
                // objects of other kinds come and go all the time (every
                // `wpctl` is a client): only ours count
                let known = self.sinks.remove(&id).is_some() | self.devices.remove(&id).is_some();
                Touched {
                    volume: known,
                    devices: known,
                }
            }
        }
    }

    /// The outputs and cards as they are now.
    pub fn snapshot(&self) -> Snapshot {
        let outputs = self
            .sinks
            .iter()
            .map(|(id, sink)| Output {
                id: id.to_string(),
                label: sink.label.clone(),
                default: self.default.as_deref() == Some(sink.name.as_str()),
                card: sink
                    .device
                    .filter(|device| self.devices.contains_key(device))
                    .map(|device| device.to_string()),
            })
            .collect();
        let cards = self
            .devices
            .iter()
            .map(|(id, device)| Card {
                id: id.to_string(),
                label: device.label.clone(),
                profiles: device.profiles.clone(),
            })
            .collect();
        Snapshot { outputs, cards }
    }
}

#[derive(Deserialize)]
struct Head {
    id: u32,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct NodeObject {
    info: Option<NodeInfo>,
}

#[derive(Deserialize, Default)]
struct NodeInfo {
    #[serde(default)]
    props: HashMap<String, Value>,
}

#[derive(Deserialize)]
struct DeviceObject {
    info: Option<DeviceInfo>,
}

#[derive(Deserialize, Default)]
struct DeviceInfo {
    #[serde(default)]
    props: HashMap<String, Value>,
    #[serde(default)]
    params: DeviceParams,
}

#[derive(Deserialize, Default)]
struct DeviceParams {
    #[serde(rename = "EnumProfile", default)]
    enum_profile: Vec<RawProfile>,
    /// The profile in use.
    #[serde(rename = "Profile", default)]
    profile: Vec<RawProfile>,
}

#[derive(Deserialize)]
struct RawProfile {
    index: Option<u32>,
    name: Option<String>,
    description: Option<String>,
    priority: Option<i64>,
    /// `yes`, `no` or `unknown`.
    available: Option<Value>,
}

#[derive(Deserialize)]
struct MetadataObject {
    #[serde(default)]
    props: HashMap<String, Value>,
    #[serde(default)]
    metadata: Vec<MetadataEntry>,
}

#[derive(Deserialize)]
struct MetadataEntry {
    key: Option<String>,
    #[serde(default)]
    value: Value,
}

/// A property as text (`PipeWire` writes some numbers as numbers).
fn text(props: &HashMap<String, Value>, key: &str) -> Option<String> {
    match props.get(key)? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn number(props: &HashMap<String, Value>, key: &str) -> Option<u32> {
    text(props, key)?.parse().ok()
}

/// What one object of `pw-dump`'s output says, when it is about an audio
/// sink or device, or the default sink; an object that went away.
pub fn parse_object(json: &str) -> Option<Update> {
    let head: Head = serde_json::from_str(json).ok()?;
    let Some(kind) = head.kind else {
        // {"id": 49, "info": null}
        return Some(Update::Removed(head.id));
    };
    match kind.rsplit(':').next()? {
        "Node" => parse_node(head.id, json),
        "Device" => parse_device(head.id, json),
        "Metadata" => parse_metadata(json),
        _ => None,
    }
}

fn parse_node(id: u32, json: &str) -> Option<Update> {
    let props = serde_json::from_str::<NodeObject>(json)
        .ok()?
        .info
        .unwrap_or_default()
        .props;
    if text(&props, "media.class").as_deref() != Some("Audio/Sink") {
        // it was one, once
        return Some(Update::Removed(id));
    }
    let name = text(&props, "node.name")?;
    let label = text(&props, "node.description")
        .or_else(|| text(&props, "node.nick"))
        .unwrap_or_else(|| name.clone());
    Some(Update::Sink(
        id,
        Sink {
            name,
            label,
            device: number(&props, "device.id"),
        },
    ))
}

fn parse_device(id: u32, json: &str) -> Option<Update> {
    let DeviceInfo { props, params } = serde_json::from_str::<DeviceObject>(json)
        .ok()?
        .info
        .unwrap_or_default();
    if text(&props, "media.class").as_deref() != Some("Audio/Device") {
        return Some(Update::Removed(id));
    }
    let label = text(&props, "device.description")
        .or_else(|| text(&props, "device.nick"))
        .or_else(|| text(&props, "device.name"))?;
    let active = params.profile.first().and_then(|profile| profile.index);
    let mut raw = params.enum_profile;
    // best first (the sort is stable: the server's order breaks ties)
    raw.sort_by_key(|profile| std::cmp::Reverse(profile.priority.unwrap_or(0)));
    let profiles = raw
        .into_iter()
        .filter_map(|profile| {
            let index = profile.index?;
            let label = profile.description.or_else(|| profile.name.clone())?;
            Some(Profile {
                id: index.to_string(),
                label,
                active: active == Some(index),
                available: !matches!(&profile.available, Some(Value::String(no)) if no == "no"),
                off: profile.name.as_deref() == Some("off"),
            })
        })
        .collect();
    Some(Update::Device(
        id,
        Device {
            label,
            profiles,
            active,
        },
    ))
}

fn parse_metadata(json: &str) -> Option<Update> {
    let object: MetadataObject = serde_json::from_str(json).ok()?;
    if text(&object.props, "metadata.name").as_deref() != Some("default") {
        return None;
    }
    // an update without the entry says nothing about the default
    let entry = object
        .metadata
        .into_iter()
        .find(|entry| entry.key.as_deref() == Some("default.audio.sink"))?;
    let name = match entry.value {
        // {"name": "alsa_output..."}, or the same as a string
        Value::Object(fields) => fields
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Value::String(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| value.get("name")?.as_str().map(str::to_owned))
            .or(Some(text)),
        _ => None,
    };
    Some(Update::Default(name))
}

/// Make a sink (its node id) the default output; true when done.
pub fn set_default(id: &str) -> bool {
    run("wpctl", &["set-default", id])
}

/// Switch a device (its id) to a profile (its index); true when done.
pub fn set_profile(device: &str, index: &str) -> bool {
    run("wpctl", &["set-profile", device, index])
}

#[cfg(test)]
mod tests {
    use super::*;

    // Objects as `pw-dump` (PipeWire 1.0.5) prints them, params trimmed.
    const SINK: &str = r#"{"id":31,"type":"PipeWire:Interface:Node","version":3,"info":{"max-input-ports":65,"n-input-ports":2,"state":"suspended","error":null,"props":{"audio.position":"[ FL FR ]","factory.name":"support.null-audio-sink","media.class":"Audio/Sink","node.description":"Test Speakers","node.driver":true,"node.name":"test_sink_a","object.id":31,"device.id":47}}}"#;
    const DEFAULT: &str = r#"{"id":36,"type":"PipeWire:Interface:Metadata","version":3,"permissions":["r","w","x"],"props":{"client.id":34,"metadata.name":"default"},"metadata":[{"subject":0,"key":"default.configured.audio.sink","type":"Spa:String:JSON","value":{"name":"test_sink_b"}},{"subject":0,"key":"default.audio.sink","type":"Spa:String:JSON","value":{"name":"test_sink_a"}},{"subject":0,"key":"default.audio.source","type":"Spa:String:JSON","value":{"name":"test_sink_a"}}]}"#;
    const DEVICE: &str = r#"{"id":47,"type":"PipeWire:Interface:Device","version":3,"permissions":["r","w","x","m"],"info":{"change-mask":["props","params"],"props":{"alsa.card":0,"device.description":"Built-in Audio","device.name":"alsa_card.pci-0000_00_1f.3","device.nick":"HDA Intel PCH","media.class":"Audio/Device","object.id":47},"params":{"EnumProfile":[{"index":0,"name":"off","description":"Off","priority":0,"available":"yes","info":[],"classes":[]},{"index":2,"name":"output:hdmi-stereo","description":"Digital Stereo (HDMI) Output","priority":5900,"available":"no"},{"index":1,"name":"output:analog-stereo","description":"Analog Stereo Output","priority":6500,"available":"yes"}],"Profile":[{"index":1,"name":"output:analog-stereo","description":"Analog Stereo Output","save":false}],"Route":[{"index":0}]}}}"#;

    fn graph() -> Graph {
        let mut graph = Graph::default();
        for json in [SINK, DEFAULT, DEVICE] {
            graph.update(parse_object(json).unwrap());
        }
        graph
    }

    #[test]
    fn reads_a_sink() {
        let Some(Update::Sink(31, sink)) = parse_object(SINK) else {
            panic!("not a sink");
        };
        assert_eq!(sink.name, "test_sink_a");
        assert_eq!(sink.label, "Test Speakers");
        // a number in the dump
        assert_eq!(sink.device, Some(47));
    }

    #[test]
    fn reads_the_default_sink_from_the_effective_entry() {
        assert_eq!(
            parse_object(DEFAULT),
            Some(Update::Default(Some("test_sink_a".into())))
        );
        // a string holding JSON, as older versions print it
        let old = DEFAULT.replace(
            r#"{"name":"test_sink_a"}},{"subject":0,"key":"default.audio.source""#,
            r#""{\"name\":\"test_sink_a\"}"},{"subject":0,"key":"default.audio.source""#,
        );
        assert_eq!(
            parse_object(&old),
            Some(Update::Default(Some("test_sink_a".into())))
        );
    }

    #[test]
    fn reads_a_devices_profiles_best_first() {
        let Some(Update::Device(47, device)) = parse_object(DEVICE) else {
            panic!("not a device");
        };
        assert_eq!(device.label, "Built-in Audio");
        let ids: Vec<_> = device.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["1", "2", "0"]);
        let analog = &device.profiles[0];
        assert!(analog.active && analog.available && !analog.off);
        assert!(!device.profiles[1].available);
        assert!(device.profiles[2].off);
    }

    #[test]
    fn leaves_other_objects_alone() {
        assert_eq!(
            parse_object(r#"{"id":4,"type":"PipeWire:Interface:Profiler","version":3}"#),
            None
        );
        assert_eq!(
            parse_object(
                r#"{"id":1,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"settings"},"metadata":[]}"#
            ),
            None
        );
        // a video node: not ours, but if it was ours it is gone
        let video = r#"{"id":9,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Video/Source"}}}"#;
        assert_eq!(parse_object(video), Some(Update::Removed(9)));
        assert_eq!(parse_object("not json"), None);
        assert_eq!(
            parse_object(r#"{"id":49,"info":null}"#),
            Some(Update::Removed(49))
        );
    }

    #[test]
    fn builds_the_snapshot() {
        let snapshot = graph().snapshot();
        assert_eq!(snapshot.outputs.len(), 1);
        let output = &snapshot.outputs[0];
        assert_eq!(output.id, "31");
        assert!(output.default);
        assert_eq!(output.card.as_deref(), Some("47"));
        assert_eq!(snapshot.default_card().unwrap().label, "Built-in Audio");
    }

    #[test]
    fn a_sink_without_its_device_has_no_card() {
        let mut graph = Graph::default();
        graph.update(parse_object(SINK).unwrap());
        assert_eq!(graph.snapshot().outputs[0].card, None);
    }

    #[test]
    fn says_what_a_change_touched() {
        let mut graph = graph();
        let same = parse_object(SINK).unwrap();
        // a volume change prints the same sink again: the volume may have
        // changed, the list has not
        assert_eq!(
            graph.update(same),
            Touched {
                volume: true,
                devices: false
            }
        );
        let renamed = SINK.replace("Test Speakers", "Speakers");
        assert!(graph.update(parse_object(&renamed).unwrap()).devices);
        // every `wpctl` is a client that comes and goes: nothing to do
        assert_eq!(graph.update(Update::Removed(49)), Touched::default());
        assert_eq!(
            graph.update(Update::Removed(31)),
            Touched {
                volume: true,
                devices: true
            }
        );
        assert_eq!(graph.snapshot().outputs.len(), 0);
        let moved = graph.update(Update::Default(Some("other".into())));
        assert!(moved.volume && moved.devices);
        assert_eq!(
            graph.update(Update::Default(Some("other".into()))),
            Touched::default()
        );
    }

    #[test]
    fn a_metadata_update_without_the_default_says_nothing() {
        let json = r#"{"id":36,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},"metadata":[{"subject":0,"key":"default.configured.audio.sink","type":"Spa:String:JSON","value":{"name":"x"}}]}"#;
        assert_eq!(parse_object(json), None);
    }

    #[test]
    fn a_device_update_without_profiles_keeps_them() {
        let mut graph = graph();
        let only_route = r#"{"id":47,"type":"PipeWire:Interface:Device","info":{"props":{"device.description":"Built-in Audio","media.class":"Audio/Device"},"params":{"Route":[{"index":0}]}}}"#;
        graph.update(parse_object(only_route).unwrap());
        let snapshot = graph.snapshot();
        let profiles = &snapshot.cards[0].profiles;
        assert_eq!(profiles.len(), 3);
        assert!(profiles.iter().any(|profile| profile.active));
        // and one that only says the profile in use moves the mark
        let profile_only = r#"{"id":47,"type":"PipeWire:Interface:Device","info":{"props":{"device.description":"Built-in Audio","media.class":"Audio/Device"},"params":{"Profile":[{"index":2,"name":"output:hdmi-stereo"}]}}}"#;
        graph.update(parse_object(profile_only).unwrap());
        let snapshot = graph.snapshot();
        let active: Vec<_> = snapshot.cards[0]
            .profiles
            .iter()
            .filter(|profile| profile.active)
            .map(|profile| profile.id.as_str())
            .collect();
        assert_eq!(active, ["2"]);
    }
}
