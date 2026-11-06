//! Translations (Fluent files built into the binary) and the clock's locale.
//!
//! The language is EmulationStation's (`system.language`, exported with the
//! menu theme), else the system's (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`,
//! `LANG`), else English.

use std::env;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

/// Language code -> Fluent file and the chrono locale of its main country.
const LANGUAGES: &[(&str, &str, &str)] = &[
    ("en", include_str!("../i18n/en.ftl"), "en_US"),
    ("es", include_str!("../i18n/es.ftl"), "es_ES"),
    ("ca", include_str!("../i18n/ca.ftl"), "ca_ES"),
    ("gl", include_str!("../i18n/gl.ftl"), "gl_ES"),
    ("eu", include_str!("../i18n/eu.ftl"), "eu_ES"),
    ("pt", include_str!("../i18n/pt.ftl"), "pt_PT"),
    ("fr", include_str!("../i18n/fr.ftl"), "fr_FR"),
    ("it", include_str!("../i18n/it.ftl"), "it_IT"),
    ("de", include_str!("../i18n/de.ftl"), "de_DE"),
    ("nl", include_str!("../i18n/nl.ftl"), "nl_NL"),
    ("pl", include_str!("../i18n/pl.ftl"), "pl_PL"),
    ("ru", include_str!("../i18n/ru.ftl"), "ru_RU"),
];

const FALLBACK: &str = "en";

/// The texts of one language, with English for anything it lacks.
pub struct Translator {
    bundle: FluentBundle<FluentResource>,
    fallback: FluentBundle<FluentResource>,
    locale: chrono::Locale,
}

fn bundle(code: &str, source: &str) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = code.parse().unwrap_or_default();
    let mut bundle = FluentBundle::new(vec![id]);
    // no Unicode isolation marks around arguments: they would show as boxes
    bundle.set_use_isolating(false);
    let resource =
        FluentResource::try_new(source.to_owned()).unwrap_or_else(|(resource, errors)| {
            log::warn!("errors in the {code} translation: {errors:?}");
            resource
        });
    if let Err(errors) = bundle.add_resource(resource) {
        log::warn!("duplicate messages in the {code} translation: {errors:?}");
    }
    bundle
}

/// The language and region of a locale-ish string: `es_ES.UTF-8` -> ("es", "ES").
fn split_locale(raw: &str) -> (String, String) {
    let base = raw.split(['.', '@', ':']).next().unwrap_or_default();
    let mut parts = base.split(['_', '-']);
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    let region = parts.next().unwrap_or_default().to_ascii_uppercase();
    (language, region)
}

/// The preferred language: ES's, else the environment's.
pub fn preferred_language(es_language: &str) -> String {
    if !es_language.trim().is_empty() {
        return es_language.trim().to_owned();
    }
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .unwrap_or_else(|| FALLBACK.to_owned())
}

impl Translator {
    /// The translator for a language such as `es_ES` or `ca`.
    pub fn new(language: &str) -> Self {
        let (code, region) = split_locale(language);
        let (code, source, default_locale) = LANGUAGES
            .iter()
            .copied()
            .find(|(known, _, _)| *known == code)
            .unwrap_or(LANGUAGES[0]);
        let locale = chrono::Locale::try_from(format!("{code}_{region}").as_str())
            .or_else(|_| chrono::Locale::try_from(default_locale))
            .unwrap_or(chrono::Locale::POSIX);
        Self {
            bundle: bundle(code, source),
            fallback: bundle(FALLBACK, LANGUAGES[0].1),
            locale,
        }
    }

    /// The text of a message id (the id itself when no language has it).
    pub fn text(&self, id: &str) -> String {
        self.format(id, None)
    }

    /// The text of a message with one numeric argument (`{ $name }`).
    pub fn text_with(&self, id: &str, name: &str, value: i64) -> String {
        self.text_args(id, &[(name, value)])
    }

    /// The text of a message with numeric arguments.
    pub fn text_args(&self, id: &str, values: &[(&str, i64)]) -> String {
        let mut args = FluentArgs::new();
        for (name, value) in values {
            args.set(*name, *value);
        }
        self.format(id, Some(&args))
    }

    fn format(&self, id: &str, args: Option<&FluentArgs>) -> String {
        for bundle in [&self.bundle, &self.fallback] {
            if let Some(pattern) = bundle.get_message(id).and_then(|message| message.value()) {
                let mut errors = Vec::new();
                return bundle
                    .format_pattern(pattern, args, &mut errors)
                    .into_owned();
            }
        }
        id.to_owned()
    }

    /// The locale the clock formats day and month names in.
    pub fn locale(&self) -> chrono::Locale {
        self.locale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_language_and_locale() {
        let tr = Translator::new("ca_ES.UTF-8");
        assert_eq!(tr.text("menu-quit"), "Surt del joc");
        assert_eq!(tr.locale(), chrono::Locale::ca_ES);
        assert_eq!(Translator::new("pt_BR").locale(), chrono::Locale::pt_BR);
    }

    #[test]
    fn unknown_language_falls_back_to_english() {
        let tr = Translator::new("tlh");
        assert_eq!(tr.text("menu-continue"), "Continue");
        assert_eq!(tr.text("no-such-id"), "no-such-id");
    }

    #[test]
    fn every_language_has_every_message() {
        let english = bundle(FALLBACK, LANGUAGES[0].1);
        for (code, source, _) in LANGUAGES {
            let translated = bundle(code, source);
            for id in [
                "menu-continue",
                "menu-quit",
                "clock-date-format",
                "clock-time-format",
            ] {
                assert!(english.has_message(id), "{id}");
                assert!(translated.has_message(id), "{code}: {id}");
            }
        }
    }
}
