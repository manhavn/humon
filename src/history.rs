//! Recent and favorite queries per collection, kept in `history.json` next
//! to the saved connections.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::files;

const MAX_RECENT: usize = 20;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Default, Debug)]
pub struct Query {
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub projection: String,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.filter.trim().is_empty() && self.sort.trim().is_empty() && self.projection.trim().is_empty()
    }

    /// One line for lists: the filter, plus sort and projection when set.
    pub fn summary(&self) -> String {
        let mut parts = vec![if self.filter.trim().is_empty() {
            "{}".to_string()
        } else {
            self.filter.trim().to_string()
        }];
        if !self.sort.trim().is_empty() {
            parts.push(format!("sort {}", self.sort.trim()));
        }
        if !self.projection.trim().is_empty() {
            parts.push(format!("project {}", self.projection.trim()));
        }
        parts.join(" · ")
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Entry {
    #[serde(default)]
    recent: Vec<Query>,
    #[serde(default)]
    favorites: Vec<Query>,
}

/// All collections' queries, keyed by "connection id/db/collection".
/// `History::default()` is detached: usable in memory, never written.
#[derive(Serialize, Deserialize, Default)]
pub struct History {
    #[serde(flatten)]
    entries: BTreeMap<String, Entry>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

pub fn key(conn_id: &str, db: &str, coll: &str) -> String {
    format!("{conn_id}/{db}/{coll}")
}

impl History {
    pub fn default_path() -> PathBuf {
        files::config_dir().join("history.json")
    }

    /// Loads history that `save` writes back to the same file. A corrupt
    /// file is an error, so the caller can keep it instead of overwriting it.
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let mut history: Self = files::read_json(&path)?;
        history.path = Some(path);
        Ok(history)
    }

    pub fn save(&self) -> Result<(), String> {
        match &self.path {
            Some(path) => files::write_json(path, self),
            None => Ok(()),
        }
    }

    pub fn recent(&self, key: &str) -> &[Query] {
        self.entries.get(key).map_or(&[], |e| &e.recent)
    }

    pub fn favorites(&self, key: &str) -> &[Query] {
        self.entries.get(key).map_or(&[], |e| &e.favorites)
    }

    /// Puts `query` at the top of the recent list, without duplicates.
    pub fn record(&mut self, key: &str, query: Query) {
        if query.is_empty() {
            return;
        }
        let recent = &mut self.entries.entry(key.to_string()).or_default().recent;
        recent.retain(|q| *q != query);
        recent.insert(0, query);
        recent.truncate(MAX_RECENT);
    }

    /// Adds a favorite; returns false if it was already there.
    pub fn add_favorite(&mut self, key: &str, query: Query) -> bool {
        let favorites = &mut self.entries.entry(key.to_string()).or_default().favorites;
        if query.is_empty() || favorites.contains(&query) {
            return false;
        }
        favorites.push(query);
        true
    }

    pub fn remove_favorite(&mut self, key: &str, index: usize) {
        if let Some(entry) = self.entries.get_mut(key)
            && index < entry.favorites.len()
        {
            entry.favorites.remove(index);
        }
    }

    /// Drops everything stored for collections of a connection, db or collection.
    pub fn forget(&mut self, prefix: &str) {
        self.entries
            .retain(|k, _| !(k == prefix || k.starts_with(&format!("{prefix}/"))));
    }

    /// Moves a collection's entry after a rename.
    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(entry) = self.entries.remove(old) {
            self.entries.insert(new.to_string(), entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(filter: &str) -> Query {
        Query {
            filter: filter.into(),
            ..Default::default()
        }
    }

    #[test]
    fn recent_is_deduplicated_newest_first_and_capped() {
        let mut h = History::default();
        let k = key("c", "db", "users");
        h.record(&k, q("{ a: 1 }"));
        h.record(&k, q("{ b: 1 }"));
        h.record(&k, q("{ a: 1 }"));
        h.record(&k, Query::default());
        assert_eq!(h.recent(&k), &[q("{ a: 1 }"), q("{ b: 1 }")]);
        for i in 0..30 {
            h.record(&k, q(&format!("{{ n: {i} }}")));
        }
        assert_eq!(h.recent(&k).len(), MAX_RECENT);
        assert_eq!(h.recent(&k)[0], q("{ n: 29 }"));
    }

    #[test]
    fn favorites_rename_and_forget() {
        let mut h = History::default();
        let k = key("c", "db", "users");
        assert!(h.add_favorite(&k, q("{ a: 1 }")));
        assert!(!h.add_favorite(&k, q("{ a: 1 }")));
        h.rename(&k, &key("c", "db", "people"));
        assert_eq!(h.favorites(&key("c", "db", "people")).len(), 1);
        h.forget("c/db");
        assert!(h.favorites(&key("c", "db", "people")).is_empty());
        h.add_favorite(&k, q("{ b: 1 }"));
        h.remove_favorite(&k, 0);
        assert!(h.favorites(&k).is_empty());
    }

    #[test]
    fn saved_history_opens_again() {
        let path = crate::files::test_dir("history-save").join("history.json");
        let mut h = History::open(path.clone()).unwrap();
        h.record(&key("c", "db", "users"), q("{ a: 1 }"));
        h.add_favorite(&key("c", "db", "users"), q("{ fav: 1 }"));
        h.save().unwrap();
        let again = History::open(path).unwrap();
        assert_eq!(again.recent(&key("c", "db", "users")), &[q("{ a: 1 }")]);
        assert_eq!(again.favorites(&key("c", "db", "users")), &[q("{ fav: 1 }")]);
    }

    #[test]
    fn missing_file_opens_empty() {
        let path = crate::files::test_dir("history-missing").join("history.json");
        assert!(History::open(path).unwrap().recent("x").is_empty());
    }

    #[test]
    fn corrupt_file_is_an_error_and_a_detached_history_never_writes() {
        let path = crate::files::test_dir("history-corrupt").join("history.json");
        std::fs::write(&path, "nope").unwrap();
        assert!(History::open(path.clone()).is_err());
        // What the app falls back to: usable in memory, but saving is a no-op.
        let mut h = History::default();
        h.record("k", q("{ a: 1 }"));
        h.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nope");
    }

    #[test]
    fn summary_shows_only_what_is_set() {
        let full = Query {
            filter: "{ a: 1 }".into(),
            sort: "{ _id: -1 }".into(),
            projection: String::new(),
        };
        assert_eq!(full.summary(), "{ a: 1 } · sort { _id: -1 }");
        assert_eq!(Query::default().summary(), "{}");
    }
}
