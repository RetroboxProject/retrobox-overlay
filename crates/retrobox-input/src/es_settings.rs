//! EmulationStation's own settings (`es_settings.cfg`, the options of its
//! advanced menu).

use crate::paths;

/// The option "RIGHT ANALOG STICK CHANGES VOLUME" of ES's advanced menu.
const RIGHT_STICK_VOLUME: &str = "RightStickVolume";

/// The option "ENABLE NAVIGATION SOUNDS" of ES's sound settings.
const NAVIGATION_SOUNDS: &str = "EnableSounds";

/// Whether the right stick sets the volume: ES's option, which is on unless
/// the file says otherwise (ES does not write an option that has its default
/// value).
pub fn right_stick_volume() -> bool {
    option(RIGHT_STICK_VOLUME).unwrap_or(true)
}

/// Whether the menu plays its sounds: ES's option, on by default (Retrobox's
/// EmulationStation turns it on, upstream's has it off).
pub fn navigation_sounds() -> bool {
    option(NAVIGATION_SOUNDS).unwrap_or(true)
}

/// A boolean option of the user's `es_settings.cfg`, when it is written.
fn option(name: &str) -> Option<bool> {
    paths::config_dirs()
        .pop()
        .map(|dir| dir.join("frontend/.emulationstation/es_settings.cfg"))
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| boolean(&text, name))
}

/// The boolean option `name` of `text` (`es_settings.cfg`), if it has it. The
/// file may have no single root element, so it is read as the lines it has:
/// `<bool name="..." value="true" />`.
fn boolean(text: &str, name: &str) -> Option<bool> {
    let wrapped = format!(
        "<settings>{}</settings>",
        text.lines()
            .filter(|line| !line.trim_start().starts_with("<?xml"))
            .collect::<String>()
    );
    let document = roxmltree::Document::parse(&wrapped).ok()?;
    document
        .descendants()
        .find(|node| node.has_tag_name("bool") && node.attribute("name") == Some(name))
        .and_then(|node| node.attribute("value"))
        .map(|value| value.trim().eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_boolean_option() {
        let text = "<?xml version=\"1.0\"?>\n<bool name=\"A\" value=\"true\" />\n<string name=\"B\" value=\"true\" />\n<bool name=\"C\" value=\"false\" />\n";
        assert_eq!(boolean(text, "A"), Some(true));
        assert_eq!(boolean(text, "C"), Some(false));
        // not a boolean, and not there
        assert_eq!(boolean(text, "B"), None);
        assert_eq!(boolean(text, "missing"), None);
        assert_eq!(boolean("not xml <", "A"), None);
    }
}
