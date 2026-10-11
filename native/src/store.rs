//! Saved connections, in the same files the Tauri app uses so both see them.
//!
//! The password is split out of the URI and stored separately in
//! `secrets.json`, so moving secrets to the Keychain later only touches
//! `load_secrets`/`save_secrets`. Until then both files are plain JSON that only
//! the current user can read (0600).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

const CONNECTIONS_FILE: &str = "connections.json";
const SECRETS_FILE: &str = "secrets.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    /// Connection string with the password removed; safe to show.
    pub uri: String,
}

fn config_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let dir = PathBuf::from(home).join("Library/Application Support/dev.muzaffer.humon");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Writes via a temp file and rename so a crash never leaves a half-written file.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
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

pub fn list() -> Result<Vec<SavedConnection>, String> {
    read_json(&config_dir()?.join(CONNECTIONS_FILE))
}

fn load_secrets() -> Result<HashMap<String, String>, String> {
    read_json(&config_dir()?.join(SECRETS_FILE))
}

fn save_secrets(secrets: &HashMap<String, String>) -> Result<(), String> {
    write_json(&config_dir()?.join(SECRETS_FILE), secrets)
}

/// The full connection string, password included, for connecting.
pub fn resolve_uri(conn: &SavedConnection) -> Result<String, String> {
    Ok(match load_secrets()?.get(&conn.id) {
        Some(password) => join_password(&conn.uri, password),
        None => conn.uri.clone(),
    })
}

pub fn save(name: &str, uri: &str) -> Result<SavedConnection, String> {
    let (uri, password) = split_password(uri.trim());
    let id = ObjectId::new().to_hex();
    let name = match name.trim() {
        "" => display_host(&uri),
        n => n.to_string(),
    };
    let conn = SavedConnection { id: id.clone(), name, uri };
    let mut all = list()?;
    all.push(conn.clone());
    if let Some(password) = password {
        let mut secrets = load_secrets()?;
        secrets.insert(id, password);
        save_secrets(&secrets)?;
    }
    write_json(&config_dir()?.join(CONNECTIONS_FILE), &all)?;
    Ok(conn)
}

pub fn delete(id: &str) -> Result<(), String> {
    let mut all = list()?;
    all.retain(|c| c.id != id);
    write_json(&config_dir()?.join(CONNECTIONS_FILE), &all)?;
    let mut secrets = load_secrets()?;
    if secrets.remove(id).is_some() {
        save_secrets(&secrets)?;
    }
    Ok(())
}

/// Byte range of the `user:password` part of a connection string, if any.
fn userinfo_range(uri: &str) -> Option<(usize, usize)> {
    let start = uri.find("://")? + 3;
    let rest = &uri[start..];
    let authority = &rest[..rest.find(['/', '?']).unwrap_or(rest.len())];
    let at = authority.rfind('@')?;
    Some((start, start + at))
}

fn split_password(uri: &str) -> (String, Option<String>) {
    if let Some((start, end)) = userinfo_range(uri)
        && let Some(colon) = uri[start..end].find(':')
    {
        let colon = start + colon;
        let without = format!("{}{}", &uri[..colon], &uri[end..]);
        return (without, Some(uri[colon + 1..end].to_string()));
    }
    (uri.to_string(), None)
}

fn join_password(uri: &str, password: &str) -> String {
    match userinfo_range(uri) {
        Some((_, end)) => format!("{}:{}{}", &uri[..end], password, &uri[end..]),
        None => uri.to_string(),
    }
}

/// Host part of a URI, used as the default name for a new connection.
fn display_host(uri: &str) -> String {
    let start = uri.find("://").map_or(0, |i| i + 3);
    let rest = &uri[start..];
    let rest = &rest[rest.rfind('@').map_or(0, |i| i + 1)..];
    rest[..rest.find(['/', '?']).unwrap_or(rest.len())].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let uri = "mongodb+srv://ali:p%40ss:w@cluster0.ab.mongodb.net/?retryWrites=true";
        let (without, pw) = split_password(uri);
        assert_eq!(without, "mongodb+srv://ali@cluster0.ab.mongodb.net/?retryWrites=true");
        assert_eq!(join_password(&without, pw.as_deref().unwrap()), uri);
    }
}
