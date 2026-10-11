//! App settings (currently the UI language), stored next to the saved connections.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct Settings {
    /// Locale code such as "en" or "tr"; None follows the system language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

fn path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/dev.muzaffer.humon/settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path().ok_or("HOME is not set")?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())
    }
}

/// Every language with a file in `locales/`, sorted by code.
pub fn languages() -> Vec<String> {
    let mut codes: Vec<String> = rust_i18n::available_locales!().into_iter().map(|c| c.to_string()).collect();
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

/// The language to start in: the saved choice, else the system language.
pub fn initial_language(settings: &Settings) -> String {
    let available = languages();
    if let Some(code) = settings.language.as_ref().filter(|c| available.contains(c)) {
        return code.clone();
    }
    resolve(&sys_locale::get_locale().unwrap_or_default(), &available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_system_locales() {
        let available = vec!["en".to_string(), "tr".to_string()];
        assert_eq!(resolve("tr-TR", &available), "tr");
        assert_eq!(resolve("en_US", &available), "en");
        assert_eq!(resolve("de-DE", &available), "en");
        assert_eq!(resolve("", &available), "en");
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
                assert!(text.lines().any(|l| l.starts_with(&format!("{key}:"))), "{code}.yml is missing `{key}`");
            }
        }
    }
}
