//! Recent and favorite queries per collection, kept in `history.json` next
//! to the saved connections.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
        let mut parts = vec![if self.filter.trim().is_empty() { "{}".to_string() } else { self.filter.trim().to_string() }];
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
#[derive(Serialize, Deserialize, Default)]
pub struct History {
    #[serde(flatten)]
    entries: BTreeMap<String, Entry>,
}

pub fn key(conn_id: &str, db: &str, coll: &str) -> String {
    format!("{conn_id}/{db}/{coll}")
}

fn path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/dev.muzaffer.humon/history.json"))
}

impl History {
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
        fs::write(path, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
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
        self.entries.retain(|k, _| !(k == prefix || k.starts_with(&format!("{prefix}/"))));
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
        Query { filter: filter.into(), ..Default::default() }
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
    fn summary_shows_only_what_is_set() {
        let full = Query { filter: "{ a: 1 }".into(), sort: "{ _id: -1 }".into(), projection: String::new() };
        assert_eq!(full.summary(), "{ a: 1 } · sort { _id: -1 }");
        assert_eq!(Query::default().summary(), "{}");
    }
}
