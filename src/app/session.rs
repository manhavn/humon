//! Reopening the last launch's tabs, and keeping the session file current.

use super::*;
use crate::history::Query;
use crate::session::{SavedTab, Session};

impl Humon {
    /// The open tabs in tab order, then those still waiting for their connection.
    fn session(&self, cx: &App) -> Session {
        let ids = self.tab_ids();
        let open: Vec<SavedTab> = ids
            .iter()
            .filter_map(|id| {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == *id) {
                    Some(tab.saved(cx))
                } else {
                    self.consoles.iter().find(|c| c.id == *id).map(|c| c.saved(cx))
                }
            })
            .collect();
        let active = self.active_tab.and_then(|a| ids.iter().position(|id| *id == a));
        crate::session::merge(open, active, &self.pending, self.pending_active)
    }

    pub(crate) fn persist_session(&self, cx: &App) {
        if self.session_broken || !self.settings.restore_session() {
            return;
        }
        // Losing the session only costs the tabs next time; nothing to report.
        let session = self.session(cx);
        let session = if self.settings.session_scripts() {
            session
        } else {
            session.without_scripts()
        };
        let _ = session.save_to(&Session::default_path());
    }

    /// Connects every connection the saved tabs need.
    pub(crate) fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending.is_empty() {
            crate::session::restore_started();
            // Restoring has had its chance to crash; past this, it didn't.
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(15)).await;
                crate::session::restore_finished();
            })
            .detach();
        }
        let ids = crate::session::connections(self.pending.iter().map(|(_, tab)| tab));
        // Tabs of a connection that no longer exists can't come back.
        self.pending
            .retain(|(_, tab)| self.conns.iter().any(|c| c.saved.id == tab.conn_id()));
        for id in ids {
            let waiting = self
                .conns
                .iter()
                .any(|c| c.saved.id == id && c.client.is_none() && !c.connecting);
            if waiting {
                self.connect(&id, window, cx);
            }
        }
        cx.notify();
    }

    /// Opens the saved tabs of a connection that just came up, keeping
    /// whichever tab is active unless one of these was the saved active one.
    pub(crate) fn open_pending(&mut self, conn_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self
            .conns
            .iter()
            .find(|c| c.saved.id == conn_id)
            .and_then(|c| c.client.clone())
        else {
            return;
        };
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|(_, tab)| tab.conn_id() == conn_id);
        self.pending = rest;
        if mine.is_empty() {
            return;
        }
        let before = self.active_tab;
        let mut active = before;
        let mut first = None;
        for (place, tab) in mine {
            let opened = match tab {
                SavedTab::Collection {
                    db,
                    coll,
                    view,
                    timeseries,
                    filter,
                    sort,
                    projection,
                    page_size,
                    mode,
                    ..
                } => {
                    let kind = match (view, timeseries) {
                        (true, _) => CollKind::View,
                        (_, true) => CollKind::Timeseries,
                        _ => CollKind::Collection,
                    };
                    let query = Query {
                        filter,
                        sort,
                        projection,
                    };
                    let target = CollectionRef::new(client.clone(), &db, &coll);
                    let saved = crate::tab::SavedView { query, page_size, mode };
                    Some(self.open_saved_collection(conn_id, target, kind, saved, window, cx))
                }
                SavedTab::Unknown => None,
                SavedTab::Shell { db, code, .. } => {
                    let id = self.open_console(conn_id, &db, window, cx);
                    if let Some(id) = id
                        && !code.is_empty()
                    {
                        self.set_console_input(id, code, window, cx);
                    }
                    id
                }
            };
            first = first.or(opened);
            if self.pending_active == Some(place) {
                self.pending_active = None;
                active = opened;
            }
        }
        // Something is active: the saved active tab, else what was, else the first restored.
        self.active_tab = active.or(first);
        self.session_dirty = true;
        cx.notify();
    }
}
