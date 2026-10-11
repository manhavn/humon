//! Saved connections, kept in the app's config directory.
//!
//! The password is split out of the URI and stored separately in
//! `secrets.json`, so moving secrets to the OS keychain later only touches
//! `load_secrets`/`save_secrets`. Until then both files are plain JSON that only
//! the current user can read (0600).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const CONNECTIONS_FILE: &str = "connections.json";
const SECRETS_FILE: &str = "secrets.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    /// Connection string with the password removed; safe to show in the UI.
    pub uri: String,
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
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
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn load_connections(app: &AppHandle) -> Result<Vec<SavedConnection>, String> {
    read_json(&config_dir(app)?.join(CONNECTIONS_FILE))
}

fn save_connections(app: &AppHandle, list: &[SavedConnection]) -> Result<(), String> {
    write_json(&config_dir(app)?.join(CONNECTIONS_FILE), &list)
}

fn load_secrets(app: &AppHandle) -> Result<HashMap<String, String>, String> {
    read_json(&config_dir(app)?.join(SECRETS_FILE))
}

fn save_secrets(app: &AppHandle, secrets: &HashMap<String, String>) -> Result<(), String> {
    write_json(&config_dir(app)?.join(SECRETS_FILE), secrets)
}

/// Returns the full connection string, password included, for connecting.
pub fn resolve_uri(app: &AppHandle, id: &str) -> Result<String, String> {
    let conn = load_connections(app)?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or("kayıtlı bağlantı bulunamadı")?;
    Ok(match load_secrets(app)?.get(id) {
        Some(password) => join_password(&conn.uri, password),
        None => conn.uri,
    })
}

#[tauri::command]
pub fn list_saved(app: AppHandle) -> Result<Vec<SavedConnection>, String> {
    load_connections(&app)
}

/// Creates a connection, or updates it when `id` is given. On update a URI
/// without a password keeps the stored one, so the UI never needs to hold it.
#[tauri::command]
pub fn save_connection(
    app: AppHandle,
    id: Option<String>,
    name: String,
    uri: String,
) -> Result<SavedConnection, String> {
    let (uri, password) = split_password(uri.trim());
    let mut list = load_connections(&app)?;
    let id = id.unwrap_or_else(|| ObjectId::new().to_hex());
    let name = match name.trim() {
        "" => display_host(&uri),
        n => n.to_string(),
    };
    let conn = SavedConnection {
        id: id.clone(),
        name,
        uri,
    };
    match list.iter_mut().find(|c| c.id == id) {
        Some(existing) => *existing = conn.clone(),
        None => list.push(conn.clone()),
    }

    let mut secrets = load_secrets(&app)?;
    if let Some(password) = password {
        secrets.insert(id, password);
        save_secrets(&app, &secrets)?;
    }
    save_connections(&app, &list)?;
    Ok(conn)
}

#[tauri::command]
pub fn delete_saved(app: AppHandle, id: String) -> Result<(), String> {
    let mut list = load_connections(&app)?;
    list.retain(|c| c.id != id);
    save_connections(&app, &list)?;
    let mut secrets = load_secrets(&app)?;
    if secrets.remove(&id).is_some() {
        save_secrets(&app, &secrets)?;
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
        assert_eq!(
            without,
            "mongodb+srv://ali@cluster0.ab.mongodb.net/?retryWrites=true"
        );
        assert_eq!(pw.as_deref(), Some("p%40ss:w"));
        assert_eq!(join_password(&without, pw.as_deref().unwrap()), uri);
    }

    #[test]
    fn uris_without_password_are_untouched() {
        for uri in [
            "mongodb://localhost:27017",
            "mongodb://ali@h1,h2/db?x=1",
            "mongodb://h/?a=b@c",
        ] {
            assert_eq!(split_password(uri), (uri.to_string(), None));
        }
    }

    #[test]
    fn default_name_is_host() {
        assert_eq!(
            display_host("mongodb+srv://ali@cluster0.ab.mongodb.net/?x=1"),
            "cluster0.ab.mongodb.net"
        );
        assert_eq!(display_host("mongodb://localhost:27017"), "localhost:27017");
    }
}
