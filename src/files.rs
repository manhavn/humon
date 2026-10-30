//! The app's config directory and the JSON files in it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// `~/Library/Application Support/dev.muzaffer.humon` on macOS,
/// `~/.config/humon` on Linux.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join("Library/Application Support/dev.muzaffer.humon")
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("humon")
        } else {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".config/humon")
        }
    }
}

/// Reads JSON; a missing file is the default value, a corrupt one an error.
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes JSON readable only by the user (0600), through a temp file and a
/// rename so a crash never leaves a half-written file.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// A fresh, empty directory for one test.
#[cfg(test)]
pub fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("humon-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
    struct Sample {
        name: String,
        n: u32,
    }

    #[test]
    fn json_round_trip() {
        let dir = test_dir("files-round-trip");
        let path = dir.join("nested/sample.json");
        let value = Sample { name: "a".into(), n: 3 };
        write_json(&path, &value).unwrap();
        assert_eq!(read_json::<Sample>(&path).unwrap(), value);
    }

    #[test]
    fn missing_file_reads_as_default() {
        let dir = test_dir("files-missing");
        assert_eq!(read_json::<Sample>(&dir.join("none.json")).unwrap(), Sample::default());
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_default() {
        // Silently treating it as empty would overwrite the user's data on the next save.
        let dir = test_dir("files-corrupt");
        let path = dir.join("bad.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_json::<Sample>(&path).is_err());
    }

    #[test]
    fn written_files_are_private_and_leave_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = test_dir("files-private");
        let path = dir.join("secret.json");
        write_json(&path, &Sample::default()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["secret.json"]);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn config_dir_is_under_application_support() {
        assert!(config_dir().ends_with("Library/Application Support/dev.muzaffer.humon"));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn config_dir_is_under_config() {
        assert!(config_dir().ends_with(".config/humon") || config_dir().ends_with("humon"));
    }
}
