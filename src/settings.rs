//! App settings (language, update checks, schema sample size), stored next to the saved connections.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::files;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Settings {
    /// Locale code such as "en" or "tr"; None follows the system language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Whether to ask GitHub for a newer release on start; None means yes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_updates: Option<bool>,
    /// Documents to sample for the schema behind query completion; None means the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_sample: Option<u32>,
    /// Search the whole collection for fields the sample missed; None means yes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discover_schema: Option<bool>,
    /// The first-launch welcome (the settings page) has been shown and closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcomed: Option<bool>,
}

/// Above this a sample stops being a sample; it only makes the server work.
pub const MAX_SCHEMA_SAMPLE: u32 = 100_000;

impl Settings {
    pub fn check_updates(&self) -> bool {
        self.check_updates.unwrap_or(true)
    }

    /// Whether to open on the welcome page. Saved connections mean someone
    /// used humon before the welcome existed, so they don't see it.
    pub fn is_first_launch(&self, has_connections: bool) -> bool {
        self.welcomed != Some(true) && !has_connections
    }

    pub fn sampling(&self) -> crate::schema::Sampling {
        crate::schema::Sampling {
            size: self.schema_sample(),
            discover: self.discover_schema(),
        }
    }

    pub fn discover_schema(&self) -> bool {
        self.discover_schema.unwrap_or(true)
    }

    pub fn schema_sample(&self) -> u32 {
        match self.schema_sample {
            Some(0) | None => crate::schema::DEFAULT_SAMPLE,
            Some(n) => n.min(MAX_SCHEMA_SAMPLE),
        }
    }

    pub fn default_path() -> PathBuf {
        files::config_dir().join("settings.json")
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        files::read_json(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        files::write_json(path, self)
    }
}

/// Every language with a file in `locales/`, sorted by code.
pub fn languages() -> Vec<String> {
    let mut codes: Vec<String> = rust_i18n::available_locales!()
        .into_iter()
        .map(|c| c.to_string())
        .collect();
    codes.sort();
    codes
}

/// The best available locale for a system locale like "tr-TR": exact match,
/// then the language part, then English.
pub fn resolve(system: &str, available: &[String]) -> String {
    let lang = system.split(['-', '_']).next().unwrap_or(system).to_lowercase();
    available
        .iter()
        .find(|c| c.eq_ignore_ascii_case(system))
        .or_else(|| available.iter().find(|c| c.to_lowercase() == lang))
        .cloned()
        .unwrap_or_else(|| "en".into())
}

/// The language to start in: the saved choice if it still exists, else the
/// system language.
pub fn pick_language(saved: Option<&str>, system: &str, available: &[String]) -> String {
    match saved {
        Some(code) if available.iter().any(|c| c == code) => code.to_string(),
        _ => resolve(system, available),
    }
}

pub fn initial_language(saved: &Settings) -> String {
    pick_language(
        saved.language.as_deref(),
        &sys_locale::get_locale().unwrap_or_default(),
        &languages(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_checks_are_on_unless_turned_off() {
        assert!(Settings::default().check_updates());
        let off: Settings = serde_json::from_str(r#"{"check_updates": false}"#).unwrap();
        assert!(!off.check_updates());
        let old: Settings = serde_json::from_str(r#"{"language": "tr"}"#).unwrap();
        assert!(
            old.check_updates(),
            "files from before the setting existed keep checking"
        );
    }

    #[test]
    fn welcome_shows_only_on_a_fresh_install() {
        assert!(Settings::default().is_first_launch(false));
        assert!(
            !Settings::default().is_first_launch(true),
            "people from before the welcome already have connections"
        );
        let seen = Settings {
            welcomed: Some(true),
            ..Default::default()
        };
        assert!(
            !seen.is_first_launch(false),
            "shown once, even with no connection saved"
        );
    }

    #[test]
    fn schema_discovery_is_on_unless_turned_off() {
        assert!(Settings::default().discover_schema());
        let off = Settings {
            discover_schema: Some(false),
            ..Default::default()
        };
        assert!(!off.discover_schema());
    }

    #[test]
    fn schema_sample_defaults_and_stays_in_range() {
        use crate::schema::DEFAULT_SAMPLE;
        assert_eq!(Settings::default().schema_sample(), DEFAULT_SAMPLE);
        let pick = |n| {
            Settings {
                schema_sample: Some(n),
                ..Default::default()
            }
            .schema_sample()
        };
        assert_eq!(pick(5000), 5000);
        assert_eq!(pick(0), DEFAULT_SAMPLE, "zero would sample nothing");
        assert_eq!(
            pick(10_000_000),
            MAX_SCHEMA_SAMPLE,
            "a hand-edited huge value is capped"
        );
    }

    #[test]
    fn settings_round_trip_and_default() {
        let path = crate::files::test_dir("settings").join("settings.json");
        assert_eq!(Settings::load_from(&path).unwrap().language, None);
        Settings {
            language: Some("tr".into()),
            check_updates: Some(false),
            schema_sample: Some(500),
            discover_schema: Some(false),
            welcomed: Some(true),
        }
        .save_to(&path)
        .unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(loaded.language.as_deref(), Some("tr"));
        assert!(!loaded.check_updates(), "every setting survives a save");
        assert_eq!(loaded.schema_sample(), 500);
        assert!(!loaded.is_first_launch(false));
        assert!(!loaded.discover_schema());
        std::fs::write(&path, "[").unwrap();
        assert!(Settings::load_from(&path).is_err());
    }

    #[test]
    fn initial_language_prefers_saved_choice_when_available() {
        let available = vec!["en".to_string(), "tr".to_string()];
        assert_eq!(pick_language(Some("tr"), "en-US", &available), "tr");
        assert_eq!(
            pick_language(Some("xx"), "tr-TR", &available),
            "tr",
            "unknown saved code falls back to system"
        );
        assert_eq!(pick_language(None, "de-DE", &available), "en");
    }

    #[test]
    fn resolves_system_locales() {
        let available = vec!["en".to_string(), "tr".to_string()];
        assert_eq!(resolve("tr-TR", &available), "tr");
        assert_eq!(resolve("en_US", &available), "en");
        assert_eq!(resolve("de-DE", &available), "en");
        assert_eq!(resolve("", &available), "en");
    }

    /// Keys used in the code, as string literals in a known namespace.
    fn keys_used_in_code() -> Vec<String> {
        const NAMESPACES: &[&str] = &[
            "app.",
            "sidebar.",
            "form.",
            "tab.",
            "card.",
            "tree.",
            "status.",
            "kind.",
            "errors.",
            "settings.",
            "query.",
            "history.",
            "manage.",
            "export.",
            "console.",
            "palette.",
            "update.",
            "complete.",
            "check.",
            "schema.",
        ];
        fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        rust_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut keys = Vec::new();
        for path in files {
            let code = std::fs::read_to_string(path).unwrap();
            for literal in code.split('"').skip(1).step_by(2) {
                let is_key = NAMESPACES.iter().any(|ns| literal.starts_with(ns))
                    && !literal.ends_with('.')
                    && !literal.ends_with(".json")
                    && literal.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c == '.');
                if is_key && !keys.contains(&literal.to_string()) {
                    keys.push(literal.to_string());
                }
            }
        }
        keys
    }

    #[test]
    fn every_key_used_in_code_exists() {
        // A mistyped key renders as the raw key at runtime, with no error.
        let en = include_str!("../locales/en.yml");
        let used = keys_used_in_code();
        assert!(used.len() > 50, "scanner found only {} keys", used.len());
        let missing: Vec<_> = used
            .iter()
            .filter(|k| !en.lines().any(|l| l.starts_with(&format!("{k}:"))))
            .collect();
        assert!(missing.is_empty(), "missing from en.yml: {missing:?}");
    }

    #[test]
    fn every_language_has_every_key() {
        // A contributed locale missing keys would silently fall back to English.
        let en = include_str!("../locales/en.yml");
        let keys: Vec<&str> = en
            .lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with(' ') && l.contains(':'))
            .map(|l| l.split(':').next().unwrap())
            .collect();
        for code in languages() {
            let text = std::fs::read_to_string(format!("{}/locales/{code}.yml", env!("CARGO_MANIFEST_DIR"))).unwrap();
            for key in &keys {
                assert!(
                    text.lines().any(|l| l.starts_with(&format!("{key}:"))),
                    "{code}.yml is missing `{key}`"
                );
            }
        }
    }

    #[test]
    fn every_language_keeps_the_placeholders() {
        // A translated or dropped %{name} shows up raw, or loses the value.
        fn placeholders(text: &str) -> Vec<(String, Vec<String>)> {
            text.lines()
                .filter(|l| !l.starts_with('#') && !l.starts_with(' ') && l.contains(':'))
                .map(|l| {
                    let (key, value) = l.split_once(':').unwrap();
                    let mut names: Vec<String> = value
                        .split("%{")
                        .skip(1)
                        .filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string()))
                        .collect();
                    names.sort();
                    (key.to_string(), names)
                })
                .collect()
        }
        let en = placeholders(include_str!("../locales/en.yml"));
        for code in languages() {
            let text = std::fs::read_to_string(format!("{}/locales/{code}.yml", env!("CARGO_MANIFEST_DIR"))).unwrap();
            let theirs = placeholders(&text);
            for (key, names) in &en {
                if let Some((_, other)) = theirs.iter().find(|(k, _)| k == key) {
                    assert_eq!(other, names, "{code}.yml `{key}`");
                }
            }
        }
    }
}
