//! Connecting, disconnecting and removing saved connections, and loading
//! their databases and collections.

use super::*;

impl Humon {
    pub(crate) fn save_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.form_name.read(cx).value().to_string();
        let uri = self.form_uri.read(cx).value().to_string();
        if uri.trim().is_empty() {
            self.form_error = Some(t!("form.uri_required").into());
            return;
        }
        match self.store.save(&name, &uri, self.form_color.take()) {
            Ok(saved) => {
                self.form_error = None;
                let id = saved.id.clone();
                self.conns.push(ConnNode {
                    saved,
                    client: None,
                    connecting: false,
                    open: false,
                    error: None,
                    dbs: vec![],
                });
                self.adding = false;
                self.connect(&id, window, cx);
            }
            Err(e) => self.form_error = Some(e),
        }
    }

    pub(crate) fn connect(&mut self, saved_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let uri = self.store.resolve_uri(&self.conns[ix].saved);
        let conn = &mut self.conns[ix];
        conn.connecting = true;
        conn.error = None;
        let id = saved_id.to_string();
        self.run(
            async move {
                let client = mongo::open(uri?).await?;
                let dbs = mongo::databases(client.clone()).await?;
                Ok::<_, String>((client, dbs))
            },
            window,
            cx,
            move |this, res, _, _| {
                let Some(ix) = this.conn_ix(&id) else { return };
                let conn = &mut this.conns[ix];
                conn.connecting = false;
                match res {
                    Ok((client, dbs)) => {
                        conn.client = Some(client);
                        conn.open = true;
                        conn.dbs = dbs
                            .into_iter()
                            .map(|info| DbNode {
                                info,
                                open: false,
                                loading: false,
                                collections: None,
                            })
                            .collect();
                    }
                    Err(e) => conn.error = Some(e),
                }
            },
        );
    }

    pub(crate) fn toggle_conn(&mut self, saved_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let conn = &mut self.conns[ix];
        if conn.connecting {
            return;
        }
        if conn.client.is_none() {
            self.connect(saved_id, window, cx);
        } else {
            conn.open = !conn.open;
        }
    }

    pub(crate) fn disconnect(&mut self, saved_id: &str) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        self.tabs.retain(|t| t.conn_id != saved_id);
        self.consoles.retain(|c| c.conn_id != saved_id);
        if self.active_tab.is_some_and(|id| !self.tab_ids().contains(&id)) {
            self.active_tab = self.tab_ids().last().copied();
        }
        let conn = &mut self.conns[ix];
        conn.client = None;
        conn.open = false;
        conn.dbs.clear();
    }

    pub(crate) fn remove_conn(&mut self, saved_id: &str) {
        if self.confirm_delete.as_deref() != Some(saved_id) {
            self.confirm_delete = Some(saved_id.to_string());
            return;
        }
        self.confirm_delete = None;
        self.disconnect(saved_id);
        match self.store.delete(saved_id) {
            Ok(()) => self.conns.retain(|c| c.saved.id != saved_id),
            Err(e) => {
                if let Some(ix) = self.conn_ix(saved_id) {
                    self.conns[ix].error = Some(e);
                }
            }
        }
    }

    pub fn refresh_dbs(&mut self, saved_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let Some(client) = self.conns[ix].client.clone() else {
            return;
        };
        let id = saved_id.to_string();
        self.run(mongo::databases(client), window, cx, move |this, res, _, _| {
            let Some(ix) = this.conn_ix(&id) else { return };
            let conn = &mut this.conns[ix];
            match res {
                Ok(dbs) => {
                    conn.dbs = dbs
                        .into_iter()
                        .map(|info| DbNode {
                            info,
                            open: false,
                            loading: false,
                            collections: None,
                        })
                        .collect()
                }
                Err(e) => conn.error = Some(e),
            }
        });
    }

    pub(crate) fn toggle_db(&mut self, saved_id: &str, db_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let db = &mut self.conns[ix].dbs[db_ix];
        db.open = !db.open;
        if db.open {
            let name = db.info.name.clone();
            self.load_collections(saved_id, &name, window, cx);
        }
    }

    /// Fetches a database's collections once; later calls are no-ops.
    pub(crate) fn load_collections(
        &mut self,
        saved_id: &str,
        db_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let conn = &mut self.conns[ix];
        let Some(client) = conn.client.clone() else { return };
        let Some(db) = conn.dbs.iter_mut().find(|d| d.info.name == db_name) else {
            return;
        };
        if db.loading || db.collections.is_some() {
            return;
        }
        db.loading = true;
        let (id, name) = (saved_id.to_string(), db_name.to_string());
        self.run(
            mongo::collections(client, name.clone()),
            window,
            cx,
            move |this, res, _, _| {
                let Some(ix) = this.conn_ix(&id) else { return };
                let conn = &mut this.conns[ix];
                let Some(db) = conn.dbs.iter_mut().find(|d| d.info.name == name) else {
                    return;
                };
                db.loading = false;
                match res {
                    Ok(cols) => db.collections = Some(cols),
                    Err(e) => {
                        db.open = false;
                        conn.error = Some(e);
                    }
                }
            },
        );
    }

    /// Forgets a database's collections and fetches them again.
    pub fn reload_collections(&mut self, saved_id: &str, db_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(db) = self
            .conns
            .iter_mut()
            .find(|c| c.saved.id == saved_id)
            .and_then(|c| c.dbs.iter_mut().find(|d| d.info.name == db_name))
        {
            db.collections = None;
            db.loading = false;
        }
        self.load_collections(saved_id, db_name, window, cx);
    }

    /// Searching needs every database's collections, so load the missing ones.
    pub(crate) fn on_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query = self.search.read(cx).value().trim().to_lowercase();
        if self.query.is_empty() {
            return;
        }
        let missing: Vec<(String, String)> = self
            .conns
            .iter()
            .filter(|c| c.client.is_some())
            .flat_map(|c| {
                c.dbs
                    .iter()
                    .filter(|d| d.collections.is_none())
                    .map(|d| (c.saved.id.clone(), d.info.name.clone()))
            })
            .collect();
        for (id, db) in missing {
            self.load_collections(&id, &db, window, cx);
        }
    }

    pub(crate) fn set_color(&mut self, saved_id: &str, color: Option<String>) {
        self.color_picker = None;
        let Some(ix) = self.conn_ix(saved_id) else { return };
        match self.store.set_color(saved_id, color.clone()) {
            Ok(()) => self.conns[ix].saved.color = color,
            Err(e) => self.conns[ix].error = Some(e),
        }
    }
}
