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
        timeseries: bool,
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
    /// A tab type from a newer version: skipped, so the rest still restore.
    #[serde(other)]
    Unknown,
}

fn default_page_size() -> i64 {
    crate::tab::DEFAULT_PAGE_SIZE
}

impl SavedTab {
    pub fn conn_id(&self) -> &str {
        match self {
            SavedTab::Collection { conn_id, .. } | SavedTab::Shell { conn_id, .. } => conn_id,
            SavedTab::Unknown => "",
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

    /// The same tabs with every shell emptied, for when scripts may hold
    /// passwords and shouldn't be written down.
    pub fn without_scripts(mut self) -> Session {
        for tab in &mut self.tabs {
            if let SavedTab::Shell { code, .. } = tab {
                code.clear();
            }
        }
        self
    }
}

/// Marks the start of reopening tabs; removed once it has gone well.
fn restoring_marker(dir: &Path) -> PathBuf {
    dir.join("session.restoring")
}

/// Whether the last launch closed while reopening tabs (the marker is still
/// there). Clears it, so one bad session is skipped once, not forever.
pub fn restore_crashed_in(dir: &Path) -> bool {
    let marker = restoring_marker(dir);
    let crashed = marker.exists();
    let _ = std::fs::remove_file(marker);
    crashed
}

pub fn restore_started_in(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(restoring_marker(dir), b"");
}

pub fn restore_finished_in(dir: &Path) {
    let _ = std::fs::remove_file(restoring_marker(dir));
}

pub fn restore_crashed() -> bool {
    restore_crashed_in(&files::config_dir())
}

pub fn restore_started() {
    restore_started_in(&files::config_dir());
}

pub fn restore_finished() {
    restore_finished_in(&files::config_dir());
}

/// The session to save: the open tabs, then those still waiting for their
/// connection in their saved order. The active tab is the open one, or, while
/// none is, the saved active tab if it is still waiting.
pub fn merge(
    open: Vec<SavedTab>,
    open_active: Option<usize>,
    pending: &[(usize, SavedTab)],
    pending_active: Option<usize>,
) -> Session {
    let mut waiting: Vec<&(usize, SavedTab)> = pending.iter().collect();
    waiting.sort_by_key(|(place, _)| *place);
    let active = open_active.or_else(|| {
        let place = pending_active?;
        waiting.iter().position(|(p, _)| *p == place).map(|i| open.len() + i)
    });
    let mut tabs = open;
    tabs.extend(waiting.into_iter().map(|(_, tab)| tab.clone()));
    Session { tabs, active }
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
            timeseries: false,
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

    #[test]
    fn waiting_tabs_are_kept_with_the_active_one() {
        let pending = [(0, shell("b")), (2, collection("b", "y"))];
        let merged = merge(vec![collection("a", "x")], None, &pending, Some(2));
        assert_eq!(merged.tabs, [collection("a", "x"), shell("b"), collection("b", "y")]);
        assert_eq!(merged.active, Some(2), "the saved active tab is still waiting");
        let merged = merge(vec![collection("a", "x")], Some(0), &pending, Some(2));
        assert_eq!(merged.active, Some(0), "an open active tab wins");
    }

    #[test]
    fn older_collection_entries_and_unknown_tab_types_load() {
        let json = r#"{ "tabs": [
            { "type": "collection", "conn_id": "a", "db": "shop", "coll": "users" },
            { "type": "chart", "conn_id": "a" }
        ] }"#;
        let session: Session = serde_json::from_str(json).unwrap();
        assert!(matches!(
            &session.tabs[0],
            SavedTab::Collection { page_size: 20, view: false, mode, .. } if mode.is_empty()
        ));
        assert_eq!(
            session.tabs[1],
            SavedTab::Unknown,
            "a tab type from a newer version is skipped, not fatal"
        );
    }

    #[test]
    fn a_restore_that_never_finished_is_skipped_once() {
        let dir = crate::files::test_dir("session-restore");
        assert!(!restore_crashed_in(&dir), "a clean start");
        restore_started_in(&dir);
        restore_finished_in(&dir);
        assert!(!restore_crashed_in(&dir), "a restore that finished");
        restore_started_in(&dir);
        assert!(restore_crashed_in(&dir), "closed while restoring");
        assert!(!restore_crashed_in(&dir), "skipped once, then back to normal");
    }

    #[test]
    fn scripts_can_be_left_out() {
        let session = Session {
            tabs: vec![collection("a", "users"), shell("a")],
            active: Some(1),
        };
        let kept = session.without_scripts();
        assert_eq!(
            kept.tabs[0],
            collection("a", "users"),
            "collection tabs keep their query"
        );
        assert_eq!(
            kept.tabs[1],
            SavedTab::Shell {
                conn_id: "a".into(),
                db: "shop".into(),
                code: String::new()
            },
            "the shell tab stays, empty"
        );
        assert_eq!(kept.active, Some(1));
    }
}
