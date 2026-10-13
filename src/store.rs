//! Saved connections, kept in the app's config directory.
//!
//! The password is split out of the URI: the URI goes to `connections.json`,
//! the password to a [`Secrets`] store, the macOS Keychain in the app. Older
//! versions kept passwords in `secrets.json`; [`Store::migrate_secrets`] moves
//! them, and until it has, they are still read from there (never written).

use std::collections::HashMap;
use std::path::PathBuf;

use mongodb::bson::oid::ObjectId;
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::files;

/// Where passwords live, keyed by connection id. Errors never contain the password.
pub trait Secrets {
    fn get(&self, id: &str) -> Result<Option<String>, String>;
    fn set(&self, id: &str, password: &str) -> Result<(), String>;
    /// Removing a missing secret is not an error.
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// The macOS Keychain: one generic password per connection, under `service`.
pub struct Keychain {
    service: &'static str,
}

pub const KEYCHAIN_SERVICE: &str = "dev.muzaffer.humon";

impl Keychain {
    pub fn new(service: &'static str) -> Self {
        Self { service }
    }
}

/// `errSecItemNotFound`
const NOT_FOUND: i32 = -25300;

fn keychain_error(e: security_framework::base::Error) -> String {
    let detail = e.message().unwrap_or_else(|| format!("OSStatus {}", e.code()));
    t!("errors.keychain", error = detail).into()
}

impl Secrets for Keychain {
    fn get(&self, id: &str) -> Result<Option<String>, String> {
        match security_framework::passwords::get_generic_password(self.service, id) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| t!("errors.keychain", error = "not UTF-8").into()),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn set(&self, id: &str, password: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(self.service, id, password.as_bytes())
            .map_err(keychain_error)
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(self.service, id) {
            Err(e) if e.code() != NOT_FOUND => Err(keychain_error(e)),
            _ => Ok(()),
        }
    }
}

const CONNECTIONS_FILE: &str = "connections.json";
const SECRETS_FILE: &str = "secrets.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    /// Connection string with the password removed; safe to show.
    pub uri: String,
    /// Label color as `#rrggbb`; marks the connection and its tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Saved connections in one directory; `Store::default()` is the app's
/// config dir with passwords in the Keychain.
pub struct Store {
    dir: PathBuf,
    secrets: Box<dyn Secrets>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new(files::config_dir(), Box::new(Keychain::new(KEYCHAIN_SERVICE)))
    }
}

impl Store {
    pub fn new(dir: PathBuf, secrets: Box<dyn Secrets>) -> Self {
        Self { dir, secrets }
    }

    pub fn list(&self) -> Result<Vec<SavedConnection>, String> {
        files::read_json(&self.dir.join(CONNECTIONS_FILE))
    }

    fn write_list(&self, list: &[SavedConnection]) -> Result<(), String> {
        files::write_json(&self.dir.join(CONNECTIONS_FILE), &list)
    }

    /// Passwords an older version left in `secrets.json`, not yet moved.
    fn legacy(&self) -> Result<HashMap<String, String>, String> {
        files::read_json(&self.dir.join(SECRETS_FILE))
    }

    /// Writes what is left of `secrets.json`, removing the file once it is empty.
    fn write_legacy(&self, left: &HashMap<String, String>) -> Result<(), String> {
        let path = self.dir.join(SECRETS_FILE);
        if left.is_empty() {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        } else {
            files::write_json(&path, left)
        }
    }

    /// Moves passwords from `secrets.json` into the secret store, removing each
    /// from the file only once it is stored. Returns how many moved; on an
    /// error the rest stay in the file for the next try.
    pub fn migrate_secrets(&self) -> Result<usize, String> {
        if !self.dir.join(SECRETS_FILE).exists() {
            return Ok(0);
        }
        let mut left = self.legacy()?;
        let mut moved = 0;
        let mut failed = None;
        let ids: Vec<String> = left.keys().cloned().collect();
        for id in ids {
            match self.secrets.set(&id, &left[&id]) {
                Ok(()) => {
                    left.remove(&id);
                    moved += 1;
                }
                Err(e) => failed = Some(e),
            }
        }
        self.write_legacy(&left)?;
        match failed {
            Some(e) => Err(t!("errors.migrate_secrets", count = left.len(), error = e).into()),
            None => Ok(moved),
        }
    }

    /// The full connection string, password included, for connecting.
    pub fn resolve_uri(&self, conn: &SavedConnection) -> Result<String, String> {
        let password = match self.secrets.get(&conn.id)? {
            Some(password) => Some(password),
            None => self.legacy()?.remove(&conn.id),
        };
        Ok(match password {
            Some(password) => join_password(&conn.uri, &password),
            None => conn.uri.clone(),
        })
    }

    pub fn save(&self, name: &str, uri: &str, color: Option<String>) -> Result<SavedConnection, String> {
        let (uri, password) = split_password(uri.trim());
        let name = match name.trim() {
            "" => display_host(&uri),
            n => n.to_string(),
        };
        let conn = SavedConnection {
            id: ObjectId::new().to_hex(),
            name,
            uri,
            color,
        };
        // Read the list first, so an unreadable file stops us before anything is written.
        let mut all = self.list()?;
        all.push(conn.clone());
        if let Some(password) = &password {
            self.secrets.set(&conn.id, password)?;
        }
        if let Err(e) = self.write_list(&all) {
            if password.is_some() {
                let _ = self.secrets.delete(&conn.id);
            }
            return Err(e);
        }
        Ok(conn)
    }

    /// Sets or clears a connection's color; an unknown id changes nothing.
    pub fn set_color(&self, id: &str, color: Option<String>) -> Result<(), String> {
        let mut all = self.list()?;
        let Some(conn) = all.iter_mut().find(|c| c.id == id) else {
            return Ok(());
        };
        conn.color = color;
        self.write_list(&all)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut all = self.list()?;
        all.retain(|c| c.id != id);
        self.write_list(&all)?;
        self.secrets.delete(id)?;
        let mut left = self.legacy()?;
        if left.remove(id).is_some() {
            self.write_legacy(&left)?;
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
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    #[test]
    fn save_then_list() {
        let (store, _, _) = store("store-save");
        let a = store.save("Local", "mongodb://localhost:27017", None).unwrap();
        let b = store
            .save("", "mongodb://db.example.com:27017/app", Some("#e5484d".into()))
            .unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].id.as_str(), all[0].name.as_str()), (a.id.as_str(), "Local"));
        assert_eq!(
            all[1].name, "db.example.com:27017",
            "an empty name defaults to the host"
        );
        assert_eq!(all[1].color.as_deref(), Some("#e5484d"));
        assert_ne!(a.id, b.id);
    }

    /// Secrets in memory, failing for the ids in `fail` like a denied Keychain.
    #[derive(Clone, Default)]
    struct Memory {
        map: Rc<RefCell<HashMap<String, String>>>,
        fail: Rc<RefCell<HashSet<String>>>,
    }

    impl Secrets for Memory {
        fn get(&self, id: &str) -> Result<Option<String>, String> {
            if self.fail.borrow().contains(id) {
                return Err("denied".into());
            }
            Ok(self.map.borrow().get(id).cloned())
        }

        fn set(&self, id: &str, password: &str) -> Result<(), String> {
            if self.fail.borrow().contains(id) || self.fail.borrow().contains("*") {
                return Err("denied".into());
            }
            self.map.borrow_mut().insert(id.into(), password.into());
            Ok(())
        }

        fn delete(&self, id: &str) -> Result<(), String> {
            if self.fail.borrow().contains(id) {
                return Err("denied".into());
            }
            self.map.borrow_mut().remove(id);
            Ok(())
        }
    }

    fn store(name: &str) -> (Store, Memory, PathBuf) {
        let dir = test_dir(name);
        let secrets = Memory::default();
        (Store::new(dir.clone(), Box::new(secrets.clone())), secrets, dir)
    }

    fn legacy(dir: &std::path::Path, entries: &[(&str, &str)]) {
        let map: HashMap<&str, &str> = entries.iter().copied().collect();
        files::write_json(&dir.join("secrets.json"), &map).unwrap();
    }

    #[test]
    fn password_goes_to_the_secret_store_never_a_file() {
        let (store, secrets, dir) = store("store-password");
        let saved = store
            .save("Atlas", "mongodb+srv://ali:s3cret@c.mongodb.net/?w=1", None)
            .unwrap();
        assert_eq!(saved.uri, "mongodb+srv://ali@c.mongodb.net/?w=1");
        assert_eq!(secrets.map.borrow().get(&saved.id).map(String::as_str), Some("s3cret"));
        let listed = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(
            !listed.contains("s3cret"),
            "connections.json must not hold the password"
        );
        assert!(!dir.join("secrets.json").exists(), "no plain-text secrets file");
        assert_eq!(
            store.resolve_uri(&saved).unwrap(),
            "mongodb+srv://ali:s3cret@c.mongodb.net/?w=1",
            "restored for connecting, as after a restart"
        );
    }

    #[test]
    fn connections_without_a_password_skip_the_secret_store() {
        let (store, secrets, _) = store("store-no-password");
        secrets.fail.borrow_mut().insert("*".into());
        let saved = store.save("local", "mongodb://localhost:27017", None).unwrap();
        assert!(secrets.map.borrow().is_empty());
        assert_eq!(store.resolve_uri(&saved).unwrap(), "mongodb://localhost:27017");
    }

    #[test]
    fn a_failing_secret_store_saves_nothing_and_leaks_nothing() {
        let (store, secrets, dir) = store("store-denied");
        secrets.fail.borrow_mut().insert("*".into());
        let err = store.save("x", "mongodb://u:hunter2@h", None).unwrap_err();
        assert!(!err.contains("hunter2"), "{err}");
        assert!(store.list().unwrap().is_empty(), "no connection without its password");
        assert!(!dir.join("secrets.json").exists(), "no plain-text fallback");
    }

    #[test]
    fn delete_removes_connection_and_its_password() {
        let (store, secrets, _) = store("store-delete");
        let keep = store.save("keep", "mongodb://a:1@h1", None).unwrap();
        let gone = store.save("gone", "mongodb://b:2@h2", None).unwrap();
        store.delete(&gone.id).unwrap();
        assert_eq!(
            store.list().unwrap().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            [keep.id.as_str()]
        );
        assert!(!secrets.map.borrow().contains_key(&gone.id));
        assert!(secrets.map.borrow().contains_key(&keep.id));
    }

    #[test]
    fn a_denied_read_is_an_error_without_the_secret() {
        let (store, secrets, _) = store("store-read-denied");
        let saved = store.save("x", "mongodb://u:hunter2@h", None).unwrap();
        secrets.fail.borrow_mut().insert(saved.id.clone());
        let err = store.resolve_uri(&saved).unwrap_err();
        assert!(!err.contains("hunter2"), "{err}");
    }

    #[test]
    fn legacy_secrets_move_to_the_secret_store() {
        let (store, secrets, dir) = store("store-migrate");
        legacy(&dir, &[("a", "pa"), ("b", "pb")]);
        assert_eq!(store.migrate_secrets().unwrap(), 2);
        assert_eq!(secrets.map.borrow().get("a").map(String::as_str), Some("pa"));
        assert_eq!(secrets.map.borrow().get("b").map(String::as_str), Some("pb"));
        assert!(!dir.join("secrets.json").exists(), "the plain-text file is gone");
        assert_eq!(store.migrate_secrets().unwrap(), 0, "nothing left to do");
    }

    #[test]
    fn a_failed_migration_keeps_what_did_not_move_and_resumes() {
        let (store, secrets, dir) = store("store-migrate-partial");
        legacy(&dir, &[("a", "pa"), ("b", "pb")]);
        secrets.fail.borrow_mut().insert("b".into());
        let err = store.migrate_secrets().unwrap_err();
        assert!(!err.contains("pb"), "{err}");
        let left: HashMap<String, String> = files::read_json(&dir.join("secrets.json")).unwrap();
        assert_eq!(left.keys().collect::<Vec<_>>(), ["b"], "only the moved one is removed");
        // Until it moves, the connection still works.
        let b = SavedConnection {
            id: "b".into(),
            name: "b".into(),
            uri: "mongodb://u@h".into(),
            color: None,
        };
        secrets.fail.borrow_mut().clear();
        assert_eq!(store.resolve_uri(&b).unwrap(), "mongodb://u:pb@h");
        assert_eq!(store.migrate_secrets().unwrap(), 1);
        assert!(!dir.join("secrets.json").exists());
    }

    #[test]
    fn deleting_an_unmigrated_connection_drops_its_legacy_password() {
        let (store, _, dir) = store("store-delete-legacy");
        legacy(&dir, &[("a", "pa"), ("b", "pb")]);
        store.delete("a").unwrap();
        let left: HashMap<String, String> = files::read_json(&dir.join("secrets.json")).unwrap();
        assert_eq!(left.keys().collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn set_color_updates_and_ignores_unknown_ids() {
        let (store, _, _) = store("store-color");
        let c = store.save("x", "mongodb://h", None).unwrap();
        store.set_color(&c.id, Some("#0090ff".into())).unwrap();
        assert_eq!(store.list().unwrap()[0].color.as_deref(), Some("#0090ff"));
        store.set_color("no-such-id", Some("#ffffff".into())).unwrap();
        store.set_color(&c.id, None).unwrap();
        assert_eq!(store.list().unwrap()[0].color, None);
    }

    #[test]
    fn corrupt_connections_file_is_reported_not_overwritten() {
        let (store, _, dir) = store("store-corrupt");
        std::fs::write(dir.join("connections.json"), "{ broken").unwrap();
        assert!(store.list().is_err());
        assert!(
            store.save("x", "mongodb://h", None).is_err(),
            "saving must not wipe the unreadable file"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("connections.json")).unwrap(),
            "{ broken"
        );
    }

    #[test]
    fn display_host_strips_credentials_and_options() {
        assert_eq!(
            display_host("mongodb+srv://ali@cluster0.ab.mongodb.net/?x=1"),
            "cluster0.ab.mongodb.net"
        );
        assert_eq!(display_host("mongodb://localhost:27017"), "localhost:27017");
        assert_eq!(display_host("mongodb://u:p@h1:1,h2:2/db"), "h1:1,h2:2");
    }

    #[test]
    fn uris_without_a_password_are_untouched() {
        for uri in [
            "mongodb://localhost:27017",
            "mongodb://ali@h1,h2/db?x=1",
            "mongodb://h/?a=b@c",
        ] {
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

    /// Against the real Keychain, under a test service name. Not run in CI,
    /// whose runners may have no unlocked keychain.
    #[test]
    #[ignore = "uses the real macOS Keychain"]
    fn keychain_round_trip() {
        let keychain = Keychain::new("dev.muzaffer.humon.test");
        let id = format!("test-{}", std::process::id());
        assert_eq!(keychain.get(&id).unwrap(), None);
        keychain.set(&id, "p@ss:wörd").unwrap();
        assert_eq!(keychain.get(&id).unwrap().as_deref(), Some("p@ss:wörd"));
        keychain.set(&id, "changed").unwrap();
        assert_eq!(keychain.get(&id).unwrap().as_deref(), Some("changed"), "set replaces");
        keychain.delete(&id).unwrap();
        assert_eq!(keychain.get(&id).unwrap(), None);
        keychain.delete(&id).unwrap();
    }
}
