//! The open tabs, kept between launches: each collection tab's query and
//! view, each shell's script, and which tab was active.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::files;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SavedTab {
    Collection {
        conn_id: String,
        db: String,
        coll: String,
        /// A view: no edits, no indexes.
        #[serde(default)]
        view: bool,
        #[serde(default)]
        filter: String,
        #[serde(default)]
        sort: String,
        #[serde(default)]
        projection: String,
        #[serde(default = "default_page_size")]
        page_size: i64,
        /// "tree", "table", "json" or "schema".
        #[serde(default)]
        mode: String,
    },
    Shell {
        conn_id: String,
        db: String,
        #[serde(default)]
        code: String,
    },
}

fn default_page_size() -> i64 {
    20
}

impl SavedTab {
    pub fn conn_id(&self) -> &str {
        match self {
            SavedTab::Collection { conn_id, .. } | SavedTab::Shell { conn_id, .. } => conn_id,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs` of the active one.
    #[serde(default)]
    pub active: Option<usize>,
}

impl Session {
    pub fn default_path() -> PathBuf {
        files::config_dir().join("session.json")
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        files::read_json(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        files::write_json(path, self)
    }
}

/// The connections `tabs` need, each once, in tab order.
pub fn connections<'a>(tabs: impl IntoIterator<Item = &'a SavedTab>) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for tab in tabs {
        if !ids.iter().any(|id| id == tab.conn_id()) {
            ids.push(tab.conn_id().to_string());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection(conn: &str, coll: &str) -> SavedTab {
        SavedTab::Collection {
            conn_id: conn.into(),
            db: "shop".into(),
            coll: coll.into(),
            view: false,
            filter: "{ age: { $gt: 30 } }".into(),
            sort: String::new(),
            projection: String::new(),
            page_size: 50,
            mode: "table".into(),
        }
    }

    fn shell(conn: &str) -> SavedTab {
        SavedTab::Shell {
            conn_id: conn.into(),
            db: "shop".into(),
            code: "db.users.find()".into(),
        }
    }

    #[test]
    fn round_trips_through_its_file() {
        let path = crate::files::test_dir("session").join("session.json");
        assert_eq!(
            Session::load_from(&path).unwrap(),
            Session::default(),
            "no file: nothing to restore"
        );
        let session = Session {
            tabs: vec![collection("a", "users"), shell("b")],
            active: Some(1),
        };
        session.save_to(&path).unwrap();
        assert_eq!(Session::load_from(&path).unwrap(), session);
        std::fs::write(&path, "{ broken").unwrap();
        assert!(Session::load_from(&path).is_err());
    }

    #[test]
    fn connections_to_open_once_each_in_tab_order() {
        let session = Session {
            tabs: vec![collection("b", "x"), shell("a"), collection("b", "y")],
            active: None,
        };
        assert_eq!(connections(&session.tabs), ["b", "a"]);
    }

    #[test]
    fn older_files_without_newer_fields_still_load() {
        let json = r#"{ "tabs": [ { "type": "shell", "conn_id": "a", "db": "shop" } ] }"#;
        let session: Session = serde_json::from_str(json).unwrap();
        assert_eq!(
            session.tabs,
            [SavedTab::Shell {
                conn_id: "a".into(),
                db: "shop".into(),
                code: String::new()
            }]
        );
        assert_eq!(session.active, None);
    }
}
