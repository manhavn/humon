//! Saved connections, kept in the app's config directory.
//!
//! The password is split out of the URI and stored separately in
//! `secrets.json`, so moving secrets to the Keychain later only touches
//! `load_secrets`/`save_secrets`. Until then both files are plain JSON that only
//! the current user can read (0600).

use std::collections::HashMap;
use std::path::PathBuf;

use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use crate::files;

const CONNECTIONS_FILE: &str = "connections.json";
const SECRETS_FILE: &str = "secrets.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    /// Connection string with the password removed; safe to show.
    pub uri: String,
    /// Label color as `#rrggbb`; marks the connection and its tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Saved connections in one directory; `Store::default()` is the app's config dir.
pub struct Store {
    dir: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::new(files::config_dir())
    }
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn list(&self) -> Result<Vec<SavedConnection>, String> {
        files::read_json(&self.dir.join(CONNECTIONS_FILE))
    }

    fn write_list(&self, list: &[SavedConnection]) -> Result<(), String> {
        files::write_json(&self.dir.join(CONNECTIONS_FILE), &list)
    }

    fn secrets(&self) -> Result<HashMap<String, String>, String> {
        files::read_json(&self.dir.join(SECRETS_FILE))
    }

    fn write_secrets(&self, secrets: &HashMap<String, String>) -> Result<(), String> {
        files::write_json(&self.dir.join(SECRETS_FILE), secrets)
    }

    /// The full connection string, password included, for connecting.
    pub fn resolve_uri(&self, conn: &SavedConnection) -> Result<String, String> {
        Ok(match self.secrets()?.get(&conn.id) {
            Some(password) => join_password(&conn.uri, password),
            None => conn.uri.clone(),
        })
    }

    pub fn save(&self, name: &str, uri: &str, color: Option<String>) -> Result<SavedConnection, String> {
        let (uri, password) = split_password(uri.trim());
        let name = match name.trim() {
            "" => display_host(&uri),
            n => n.to_string(),
        };
        let conn = SavedConnection { id: ObjectId::new().to_hex(), name, uri, color };
        // Read both files first, so an unreadable one stops us before anything is written.
        let mut all = self.list()?;
        let mut secrets = self.secrets()?;
        all.push(conn.clone());
        if let Some(password) = password {
            secrets.insert(conn.id.clone(), password);
            self.write_secrets(&secrets)?;
        }
        self.write_list(&all)?;
        Ok(conn)
    }

    /// Sets or clears a connection's color; an unknown id changes nothing.
    pub fn set_color(&self, id: &str, color: Option<String>) -> Result<(), String> {
        let mut all = self.list()?;
        let Some(conn) = all.iter_mut().find(|c| c.id == id) else { return Ok(()) };
        conn.color = color;
        self.write_list(&all)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut all = self.list()?;
        all.retain(|c| c.id != id);
        self.write_list(&all)?;
        let mut secrets = self.secrets()?;
        if secrets.remove(id).is_some() {
            self.write_secrets(&secrets)?;
        }
        Ok(())
    }
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
    use crate::files::test_dir;

    #[test]
    fn save_then_list() {
        let store = Store::new(test_dir("store-save"));
        let a = store.save("Local", "mongodb://localhost:27017", None).unwrap();
        let b = store.save("", "mongodb://db.example.com:27017/app", Some("#e5484d".into())).unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].id.as_str(), all[0].name.as_str()), (a.id.as_str(), "Local"));
        assert_eq!(all[1].name, "db.example.com:27017", "an empty name defaults to the host");
        assert_eq!(all[1].color.as_deref(), Some("#e5484d"));
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn password_is_kept_apart_and_restored_on_connect() {
        let dir = test_dir("store-password");
        let store = Store::new(dir.clone());
        let saved = store.save("Atlas", "mongodb+srv://ali:s3cret@c.mongodb.net/?w=1", None).unwrap();
        assert_eq!(saved.uri, "mongodb+srv://ali@c.mongodb.net/?w=1");
        let listed = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(!listed.contains("s3cret"), "connections.json must not hold the password");
        assert_eq!(store.resolve_uri(&saved).unwrap(), "mongodb+srv://ali:s3cret@c.mongodb.net/?w=1");
    }

    #[test]
    fn delete_removes_connection_and_its_password() {
        let dir = test_dir("store-delete");
        let store = Store::new(dir.clone());
        let keep = store.save("keep", "mongodb://a:1@h1", None).unwrap();
        let gone = store.save("gone", "mongodb://b:2@h2", None).unwrap();
        store.delete(&gone.id).unwrap();
        assert_eq!(store.list().unwrap().iter().map(|c| c.id.clone()).collect::<Vec<_>>(), [keep.id.clone()]);
        let secrets = std::fs::read_to_string(dir.join("secrets.json")).unwrap();
        assert!(!secrets.contains(&gone.id) && secrets.contains(&keep.id));
    }

    #[test]
    fn set_color_updates_and_ignores_unknown_ids() {
        let store = Store::new(test_dir("store-color"));
        let c = store.save("x", "mongodb://h", None).unwrap();
        store.set_color(&c.id, Some("#0090ff".into())).unwrap();
        assert_eq!(store.list().unwrap()[0].color.as_deref(), Some("#0090ff"));
        store.set_color("no-such-id", Some("#ffffff".into())).unwrap();
        store.set_color(&c.id, None).unwrap();
        assert_eq!(store.list().unwrap()[0].color, None);
    }

    #[test]
    fn corrupt_connections_file_is_reported_not_overwritten() {
        let dir = test_dir("store-corrupt");
        std::fs::write(dir.join("connections.json"), "{ broken").unwrap();
        let store = Store::new(dir.clone());
        assert!(store.list().is_err());
        assert!(store.save("x", "mongodb://h", None).is_err(), "saving must not wipe the unreadable file");
        assert_eq!(std::fs::read_to_string(dir.join("connections.json")).unwrap(), "{ broken");
    }

    #[test]
    fn display_host_strips_credentials_and_options() {
        assert_eq!(display_host("mongodb+srv://ali@cluster0.ab.mongodb.net/?x=1"), "cluster0.ab.mongodb.net");
        assert_eq!(display_host("mongodb://localhost:27017"), "localhost:27017");
        assert_eq!(display_host("mongodb://u:p@h1:1,h2:2/db"), "h1:1,h2:2");
    }

    #[test]
    fn uris_without_a_password_are_untouched() {
        for uri in ["mongodb://localhost:27017", "mongodb://ali@h1,h2/db?x=1", "mongodb://h/?a=b@c"] {
            assert_eq!(split_password(uri), (uri.to_string(), None));
        }
    }

    #[test]
    fn color_is_optional_in_saved_files() {
        let old: SavedConnection = serde_json::from_str(r#"{"id":"a","name":"n","uri":"mongodb://h"}"#).unwrap();
        assert_eq!(old.color, None);
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("color"), "{json}");
    }

    #[test]
    fn password_round_trip() {
        let uri = "mongodb+srv://ali:p%40ss:w@cluster0.ab.mongodb.net/?retryWrites=true";
        let (without, pw) = split_password(uri);
        assert_eq!(without, "mongodb+srv://ali@cluster0.ab.mongodb.net/?retryWrites=true");
        assert_eq!(join_password(&without, pw.as_deref().unwrap()), uri);
    }
}
