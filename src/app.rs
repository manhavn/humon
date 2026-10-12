//! Root view: sidebar with saved connections and the db/collection tree, the
//! tab bar, and the active collection tab (see tab.rs).

use std::future::Future;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::Client;

use crate::mongo::{self, CollKind, CollectionInfo, DatabaseInfo, RT};
use crate::store::{SavedConnection, Store};
use crate::history::History;
use crate::manage::{EditForm, SidebarEdit};
use crate::settings::{self, Settings};
use crate::tab::Tab;
use rust_i18n::t;

pub struct DbNode {
    pub info: DatabaseInfo,
    pub open: bool,
    pub loading: bool,
    pub collections: Option<Vec<CollectionInfo>>,
}

pub struct ConnNode {
    pub saved: SavedConnection,
    pub client: Option<Client>,
    pub connecting: bool,
    pub open: bool,
    pub error: Option<String>,
    pub dbs: Vec<DbNode>,
}

pub struct Humon {
    pub conns: Vec<ConnNode>,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    next_tab_id: usize,
    adding: bool,
    form_name: Entity<InputState>,
    form_uri: Entity<InputState>,
    form_color: Option<String>,
    form_error: Option<String>,
    /// Saved id awaiting a second click to confirm deletion.
    confirm_delete: Option<String>,
    /// Saved id whose color palette is open.
    color_picker: Option<String>,
    search: Entity<InputState>,
    /// Lowercased collection search; empty shows the normal tree.
    query: String,
    pub load_error: Option<String>,
    pub history: History,
    store: Store,
    pub form: EditForm,
    show_settings: bool,
    _subscriptions: Vec<Subscription>,
}

/// The app logo, embedded so the binary needs no files next to it.
fn logo() -> Arc<Image> {
    static LOGO: std::sync::LazyLock<Arc<Image>> = std::sync::LazyLock::new(|| {
        Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../assets/logo-256.png").to_vec()))
    });
    LOGO.clone()
}

/// Connection label colors, stored as `#rrggbb`.
const PALETTE: [&str; 8] = ["#e5484d", "#f76b15", "#ffc53d", "#30a46c", "#12a594", "#0090ff", "#8e4ec6", "#d6409f"];

fn parse_color(hex: &str) -> Option<Hsla> {
    u32::from_str_radix(hex.strip_prefix('#')?, 16).ok().map(|v| rgb(v).into())
}

/// Which of a database's collections the tree shows for a search; `None`
/// hides the database. An empty query or a matching database name shows all.
pub fn visible_collections<'a>(db_name: &str, cols: Option<&'a [CollectionInfo]>, query: &str) -> Option<Vec<&'a CollectionInfo>> {
    let cols = cols.unwrap_or(&[]);
    let q = query.to_lowercase();
    if q.is_empty() || db_name.to_lowercase().contains(&q) {
        return Some(cols.iter().collect());
    }
    let matching: Vec<_> = cols.iter().filter(|c| c.name.to_lowercase().contains(&q)).collect();
    (!matching.is_empty()).then_some(matching)
}

impl ConnNode {
    pub fn color(&self) -> Option<Hsla> {
        self.saved.color.as_deref().and_then(parse_color)
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut unit = "KB";
    for u in ["MB", "GB", "TB"] {
        if v < 1024.0 {
            break;
        }
        v /= 1024.0;
        unit = u;
    }
    if v < 10.0 { format!("{v:.1} {unit}") } else { format!("{v:.0} {unit}") }
}

impl Humon {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form_name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("form.name")));
        let form_uri = cx.new(|cx| InputState::new(window, cx).placeholder(t!("form.uri")));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("sidebar.search")));
        let search_sub = cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::Change = ev {
                this.on_search(window, cx);
            }
        });
        let (form, form_subs) = EditForm::new(window, cx);
        let store = Store::default();
        // An unreadable history file is reported and left alone, never overwritten.
        let (history, history_error) = match History::open(History::default_path()) {
            Ok(h) => (h, None),
            Err(e) => (History::default(), Some(e)),
        };
        let (saved, load_error) = match store.list() {
            Ok(list) => (list, None),
            Err(e) => (vec![], Some(e)),
        };
        let load_error = load_error.or(history_error);
        let adding = saved.is_empty();
        Self {
            conns: saved
                .into_iter()
                .map(|saved| ConnNode { saved, client: None, connecting: false, open: false, error: None, dbs: vec![] })
                .collect(),
            tabs: vec![],
            active_tab: None,
            next_tab_id: 1,
            adding,
            form_name,
            form_uri,
            form_color: None,
            form_error: None,
            confirm_delete: None,
            color_picker: None,
            search,
            query: String::new(),
            load_error,
            history,
            store,
            form,
            show_settings: false,
            _subscriptions: std::iter::once(search_sub).chain(form_subs).collect(),
        }
    }

    /// Runs a driver future on tokio and applies the result back on the UI thread.
    pub fn run<T: Send + 'static>(
        &self,
        fut: impl Future<Output = T> + Send + 'static,
        window: &Window,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let handle = RT.spawn(fut);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(value) = handle.await else { return };
            this.update_in(cx, |this, window, cx| {
                apply(this, value, window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn conn_ix(&self, saved_id: &str) -> Option<usize> {
        self.conns.iter().position(|c| c.saved.id == saved_id)
    }

    fn save_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                self.conns.push(ConnNode { saved, client: None, connecting: false, open: false, error: None, dbs: vec![] });
                self.adding = false;
                self.connect(&id, window, cx);
            }
            Err(e) => self.form_error = Some(e),
        }
    }

    fn connect(&mut self, saved_id: &str, window: &mut Window, cx: &mut Context<Self>) {
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
                            .map(|info| DbNode { info, open: false, loading: false, collections: None })
                            .collect();
                    }
                    Err(e) => conn.error = Some(e),
                }
            },
        );
    }

    fn toggle_conn(&mut self, saved_id: &str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn disconnect(&mut self, saved_id: &str) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        self.tabs.retain(|t| t.conn_id != saved_id);
        if self.active_tab.is_some_and(|id| !self.tabs.iter().any(|t| t.id == id)) {
            self.active_tab = self.tabs.last().map(|t| t.id);
        }
        let conn = &mut self.conns[ix];
        conn.client = None;
        conn.open = false;
        conn.dbs.clear();
    }

    fn remove_conn(&mut self, saved_id: &str) {
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
        let Some(client) = self.conns[ix].client.clone() else { return };
        let id = saved_id.to_string();
        self.run(mongo::databases(client), window, cx, move |this, res, _, _| {
            let Some(ix) = this.conn_ix(&id) else { return };
            let conn = &mut this.conns[ix];
            match res {
                Ok(dbs) => {
                    conn.dbs = dbs
                        .into_iter()
                        .map(|info| DbNode { info, open: false, loading: false, collections: None })
                        .collect()
                }
                Err(e) => conn.error = Some(e),
            }
        });
    }

    fn toggle_db(&mut self, saved_id: &str, db_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let db = &mut self.conns[ix].dbs[db_ix];
        db.open = !db.open;
        if db.open {
            let name = db.info.name.clone();
            self.load_collections(saved_id, &name, window, cx);
        }
    }

    /// Fetches a database's collections once; later calls are no-ops.
    fn load_collections(&mut self, saved_id: &str, db_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let conn = &mut self.conns[ix];
        let Some(client) = conn.client.clone() else { return };
        let Some(db) = conn.dbs.iter_mut().find(|d| d.info.name == db_name) else { return };
        if db.loading || db.collections.is_some() {
            return;
        }
        db.loading = true;
        let (id, name) = (saved_id.to_string(), db_name.to_string());
        self.run(mongo::collections(client, name.clone()), window, cx, move |this, res, _, _| {
            let Some(ix) = this.conn_ix(&id) else { return };
            let conn = &mut this.conns[ix];
            let Some(db) = conn.dbs.iter_mut().find(|d| d.info.name == name) else { return };
            db.loading = false;
            match res {
                Ok(cols) => db.collections = Some(cols),
                Err(e) => {
                    db.open = false;
                    conn.error = Some(e);
                }
            }
        });
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

    pub fn close_tabs_where(&mut self, pred: impl Fn(&Tab) -> bool) {
        let ids: Vec<usize> = self.tabs.iter().filter(|t| pred(t)).map(|t| t.id).collect();
        for id in ids {
            self.close_tab(id);
        }
    }

    /// Searching needs every database's collections, so load the missing ones.
    fn on_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn set_language(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        rust_i18n::set_locale(code);
        let saved = Settings { language: Some(code.to_string()) }.save_to(&Settings::default_path());
        if let Err(e) = saved {
            self.load_error = Some(e);
        }
        // Placeholders are held by the inputs, so they need re-setting; everything
        // else is read through t! on the next render.
        for (input, key) in [(&self.form_name, "form.name"), (&self.form_uri, "form.uri"), (&self.search, "sidebar.search")] {
            input.update(cx, |s, cx| s.set_placeholder(t!(key), window, cx));
        }
        self.relabel_tabs(window, cx);
    }

    fn set_color(&mut self, saved_id: &str, color: Option<String>) {
        self.color_picker = None;
        let Some(ix) = self.conn_ix(saved_id) else { return };
        match self.store.set_color(saved_id, color.clone()) {
            Ok(()) => self.conns[ix].saved.color = color,
            Err(e) => self.conns[ix].error = Some(e),
        }
    }

    /// Focuses the collection's tab or opens one; `new_tab` always opens another.
    #[allow(clippy::too_many_arguments)]
    fn open_collection(&mut self, saved_id: &str, db: &str, coll: &str, kind: CollKind, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !new_tab
            && let Some(tab) = self.tabs.iter().find(|t| t.conn_id == saved_id && t.db == db && t.coll == coll)
        {
            self.active_tab = Some(tab.id);
            return;
        }
        let Some(ix) = self.conn_ix(saved_id) else { return };
        let Some(client) = self.conns[ix].client.clone() else { return };
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let tab = Tab::new(id, saved_id, client, db, coll, kind, window, cx);
        self.tabs.push(tab);
        self.active_tab = Some(id);
        self.run_query(id, window, cx);
    }

    fn close_tab(&mut self, id: usize) {
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else { return };
        self.tabs.remove(ix);
        if self.active_tab == Some(id) {
            self.active_tab = self.tabs.get(ix).or(self.tabs.last()).map(|t| t.id);
        }
    }

    /// A row of color swatches plus "none"; `on_pick` receives the chosen color.
    fn render_palette(
        &self,
        key: &str,
        selected: Option<&str>,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, Option<String>) + Clone + 'static,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, fg, muted) = (theme.border, theme.foreground, theme.muted_foreground);
        let mut row = div().flex().items_center().gap_1();
        for hex in PALETTE {
            let on_pick = on_pick.clone();
            let chosen = selected == Some(hex);
            row = row.child(
                div()
                    .id(SharedString::from(format!("swatch-{key}-{hex}")))
                    .size(px(16.))
                    .rounded_full()
                    .bg(parse_color(hex).unwrap_or(border))
                    .border_2()
                    .border_color(if chosen { fg } else { gpui_kit::transparent_black() })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        on_pick(this, Some(hex.to_string()));
                        cx.notify();
                    })),
            );
        }
        row.child(
            div()
                .id(SharedString::from(format!("swatch-{key}-none")))
                .px_1()
                .text_xs()
                .text_color(muted)
                .cursor_pointer()
                .child(t!("form.no_color").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    on_pick(this, None);
                    cx.notify();
                })),
        )
    }

    fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, danger, muted) = (theme.border, theme.danger, theme.muted_foreground);
        let palette = self.render_palette("form", self.form_color.as_deref(), cx, |this, color| this.form_color = color);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(border)
            .child(Input::new(&self.form_name))
            .child(Input::new(&self.form_uri))
            .child(div().flex().items_center().gap_2().child(div().text_xs().text_color(muted).child(t!("form.color").to_string())).child(palette))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .when(!self.conns.is_empty(), |d| {
                        d.child(Button::new("form-cancel").label(t!("form.cancel")).on_click(cx.listener(|this, _, _, cx| {
                            this.adding = false;
                            cx.notify();
                        })))
                    })
                    .child(Button::new("form-save").primary().label(t!("form.save")).on_click(cx.listener(|this, _, window, cx| {
                        this.save_connection(window, cx);
                        cx.notify();
                    }))),
            )
            .when_some(self.form_error.clone(), |d, e| d.child(div().text_xs().text_color(danger).child(e)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, hover, active_bg, accent) = (theme.muted_foreground, theme.danger, theme.list_hover, theme.list_active, theme.primary);
        let (sidebar_bg, border, warning) = (theme.sidebar, theme.border, theme.warning);
        let active = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id));
        let q = self.query.as_str();
        let searching = !q.is_empty();

        let mut list = div()
            .id("sidebar")
            .w(px(290.))
            .h_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .bg(sidebar_bg)
            .border_r_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("sidebar.connections").to_string())
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                div().id("settings").px_2().rounded_sm().cursor_pointer().hover(|s| s.bg(hover)).child("⚙")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_settings = !this.show_settings;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div().id("add-conn").px_2().rounded_sm().cursor_pointer().hover(|s| s.bg(hover)).child("+")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.adding = !this.adding;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(self.adding, |d| d.child(self.render_form(cx)))
            .when(!self.conns.is_empty(), |d| d.child(div().px_3().pb_2().child(Input::new(&self.search).cleanable(true))))
            .when_some(self.load_error.clone(), |d, e| d.child(div().px_3().text_xs().text_color(danger).child(e)));

        let mut any_match = false;
        for conn in &self.conns {
            let id = conn.saved.id.clone();
            let connected = conn.client.is_some();
            let status = if conn.connecting { warning } else if connected { accent } else { border };
            let confirm = self.confirm_delete.as_deref() == Some(id.as_str());
            let picking = self.color_picker.as_deref() == Some(id.as_str());
            let group: SharedString = format!("conn-{id}").into();
            let row = div()
                .id(SharedString::from(format!("conn-{id}")))
                .group(group.clone())
                .flex()
                .items_center()
                .gap_2()
                .pr_3()
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .on_click(cx.listener({
                    let id = id.clone();
                    move |this, _, window, cx| {
                        this.toggle_conn(&id, window, cx);
                        cx.notify();
                    }
                }))
                // Label color as a bar on the left edge; transparent when none is set.
                .child(div().w(px(3.)).h(px(26.)).bg(conn.color().unwrap_or(gpui_kit::transparent_black())))
                .child(div().text_color(muted).child(if conn.open || (searching && connected) { "▾" } else { "▸" }))
                .child(div().size(px(7.)).rounded_full().bg(status))
                .child(div().flex_1().min_w_0().overflow_hidden().font_weight(FontWeight::SEMIBOLD).child(conn.saved.name.clone()))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .text_sm()
                        .text_color(muted)
                        .when(!confirm && !picking, |d| d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)))
                        .child(
                            icon_button(format!("color-{id}"), "●", hover, cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.color_picker = if this.color_picker.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) };
                                    cx.notify();
                                }
                            }))
                            .text_color(conn.color().unwrap_or(muted)),
                        )
                        .when(connected, |d| {
                            d.child(icon_button(format!("newdb-{id}"), "+", hover, cx.listener({
                                let id = id.clone();
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.start_sidebar_edit(SidebarEdit::NewDb { conn: id.clone() }, window, cx);
                                    cx.notify();
                                }
                            })))
                            .child(icon_button(format!("refresh-{id}"), "↻", hover, cx.listener({
                                let id = id.clone();
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.refresh_dbs(&id, window, cx);
                                }
                            })))
                            .child(icon_button(format!("eject-{id}"), "⏏", hover, cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.disconnect(&id);
                                    cx.notify();
                                }
                            })))
                        })
                        .child(
                            icon_button(format!("del-{id}"), if confirm { t!("sidebar.confirm_delete").to_string() } else { "×".to_string() }, hover, cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.remove_conn(&id);
                                    cx.notify();
                                }
                            }))
                            .when(confirm, |d| d.text_color(danger)),
                        ),
                );
            list = list.child(row);
            if self.form.is(&SidebarEdit::NewDb { conn: id.clone() }) {
                list = list.child(self.render_edit_form(px(32.), cx));
            }
            if picking {
                let sid = id.clone();
                let palette = self.render_palette(&id, conn.saved.color.as_deref(), cx, move |this, color| this.set_color(&sid, color));
                list = list.child(div().pl_8().py_1().child(palette));
            }
            if conn.connecting {
                list = list.child(div().pl_8().text_xs().text_color(muted).child(t!("sidebar.connecting").to_string()));
            }
            if let Some(e) = &conn.error {
                list = list.child(div().pl_8().pr_3().text_xs().text_color(danger).child(e.clone()));
            }
            if !(conn.open || (searching && connected)) {
                continue;
            }
            for (di, db) in conn.dbs.iter().enumerate() {
                let Some(matching) = visible_collections(&db.info.name, db.collections.as_deref(), q) else { continue };
                any_match |= searching;
                let open = db.open || searching;
                let db_group: SharedString = format!("db-{id}-{di}").into();
                let (new_coll, drop_db) = (
                    SidebarEdit::NewColl { conn: id.clone(), db: db.info.name.clone() },
                    SidebarEdit::DropDb { conn: id.clone(), db: db.info.name.clone() },
                );
                let db_actions = div()
                    .flex()
                    .gap_1()
                    .text_color(muted)
                    .opacity(0.)
                    .group_hover(db_group.clone(), |s| s.opacity(1.))
                    .child(icon_button(format!("newcoll-{id}-{di}"), "+", hover, cx.listener({
                        let edit = new_coll.clone();
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.start_sidebar_edit(edit.clone(), window, cx);
                            cx.notify();
                        }
                    })))
                    .child(icon_button(format!("dropdb-{id}-{di}"), "×", hover, cx.listener({
                        let edit = drop_db.clone();
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.start_sidebar_edit(edit.clone(), window, cx);
                            cx.notify();
                        }
                    })));
                list = list.child(
                    div()
                        .id(SharedString::from(format!("db-{id}-{di}")))
                        .group(db_group)
                        .flex()
                        .gap_2()
                        .pl_8()
                        .pr_3()
                        .py_1()
                        .cursor_pointer()
                        .hover(|s| s.bg(hover))
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, window, cx| {
                                this.toggle_db(&id, di, window, cx);
                                cx.notify();
                            }
                        }))
                        .child(div().text_color(muted).child(if open { "▾" } else { "▸" }))
                        .child(div().flex_1().min_w_0().overflow_hidden().child(db.info.name.clone()))
                        .child(div().text_xs().text_color(muted).child(format_size(db.info.size_on_disk)))
                        .child(db_actions),
                );
                if self.form.is(&new_coll) || self.form.is(&drop_db) {
                    list = list.child(self.render_edit_form(px(56.), cx));
                }
                if !open {
                    continue;
                }
                if db.loading {
                    list = list.child(div().pl_16().text_xs().text_color(muted).child(t!("sidebar.loading").to_string()));
                } else if !searching && db.collections.as_ref().is_some_and(|c| c.is_empty()) {
                    list = list.child(div().pl_16().text_xs().text_color(muted).child(t!("sidebar.no_collections").to_string()));
                }
                for coll in matching {
                    let is_active = active.is_some_and(|t| t.conn_id == id && t.db == db.info.name && t.coll == coll.name);
                    let (sid, dbn, cn, kind) = (id.clone(), db.info.name.clone(), coll.name.clone(), coll.kind);
                    let open = cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        let m = ev.modifiers();
                        this.open_collection(&sid, &dbn, &cn, kind, m.platform || m.control, window, cx);
                        cx.notify();
                    });
                    let (sid, dbn, cn) = (id.clone(), db.info.name.clone(), coll.name.clone());
                    let open_new = cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.open_collection(&sid, &dbn, &cn, kind, true, window, cx);
                        cx.notify();
                    });
                    let coll_group: SharedString = format!("coll-{id}-{di}-{}", coll.name).into();
                    let (rename, drop_coll) = (
                        SidebarEdit::RenameColl { conn: id.clone(), db: db.info.name.clone(), coll: coll.name.clone() },
                        SidebarEdit::DropColl { conn: id.clone(), db: db.info.name.clone(), coll: coll.name.clone() },
                    );
                    let coll_actions = div()
                        .flex()
                        .gap_1()
                        .text_color(muted)
                        .opacity(0.)
                        .group_hover(coll_group.clone(), |s| s.opacity(1.))
                        .child(icon_button(format!("rename-{id}-{di}-{}", coll.name), "✎", hover, cx.listener({
                            let edit = rename.clone();
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.start_sidebar_edit(edit.clone(), window, cx);
                                cx.notify();
                            }
                        })))
                        .child(icon_button(format!("dropcoll-{id}-{di}-{}", coll.name), "×", hover, cx.listener({
                            let edit = drop_coll.clone();
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.start_sidebar_edit(edit.clone(), window, cx);
                                cx.notify();
                            }
                        })));
                    let show_form = self.form.is(&rename) || self.form.is(&drop_coll);
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("coll-{id}-{di}-{}", coll.name)))
                            .group(coll_group)
                            .flex()
                            .gap_2()
                            .pl_16()
                            .pr_3()
                            .py_1()
                            .cursor_pointer()
                            .when(is_active, |d| d.bg(active_bg))
                            .hover(|s| s.bg(hover))
                            .on_click(open)
                            .on_mouse_down(MouseButton::Middle, open_new)
                            .child(div().flex_1().min_w_0().overflow_hidden().child(coll.name.clone()))
                            .when(kind != CollKind::Collection, |d| {
                                d.child(div().text_xs().text_color(muted).child(if kind == CollKind::View { t!("kind.view") } else { t!("kind.timeseries") }.to_string()))
                            })
                            .child(coll_actions),
                    );
                    if show_form {
                        list = list.child(self.render_edit_form(px(80.), cx));
                    }
                }
            }
        }
        let still_loading = self.conns.iter().any(|c| c.dbs.iter().any(|d| d.loading));
        if searching && !any_match && !still_loading {
            list = list.child(div().px_3().py_2().text_xs().text_color(muted).child(t!("sidebar.no_match").to_string()));
        }
        list
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, bg, panel, border, accent, fg) = (theme.muted_foreground, theme.list_hover, theme.background, theme.sidebar, theme.border, theme.primary, theme.foreground);
        let mut bar = div().id("tabs").flex().flex_none().overflow_x_scroll().bg(panel).border_b_1().border_color(border);
        for tab in &self.tabs {
            let id = tab.id;
            let active = self.active_tab == Some(id);
            let conn = self.conns.iter().find(|c| c.saved.id == tab.conn_id);
            // The connection's color marks all its tabs; without one only the active tab is marked.
            let strip = conn.and_then(ConnNode::color).or(active.then_some(accent));
            let conn_name = conn.map(|c| c.saved.name.clone()).unwrap_or_default();
            let group: SharedString = format!("tab-{id}").into();
            bar = bar.child(
                div()
                    .id(("tab", id))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .max_w(px(240.))
                    .pl_3()
                    .pr_1()
                    .py_1()
                    .relative()
                    .border_r_1()
                    .border_color(border)
                    .cursor_pointer()
                    .text_color(if active { fg } else { muted })
                    .when(active, |d| d.bg(bg))
                    .when(!active, |d| d.hover(|s| s.bg(hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_tab = Some(id);
                        cx.notify();
                    }))
                    .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| {
                        this.close_tab(id);
                        cx.notify();
                    }))
                    .when_some(strip, |d, color| d.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(color)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::SEMIBOLD).overflow_hidden().whitespace_nowrap().child(tab.coll.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(format!("{conn_name} · {}", tab.db)),
                            ),
                    )
                    .child(
                        div()
                            .id(("close", id))
                            .px_1()
                            .rounded_sm()
                            .text_color(muted)
                            .when(!active, |d| d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)))
                            .hover(|s| s.bg(hover))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(id);
                                cx.notify();
                            })),
                    ),
            );
        }
        bar
    }
}

impl Humon {
    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, active_bg) = (theme.border, theme.muted_foreground, theme.list_hover, theme.list_active);
        let current = rust_i18n::locale().to_string();
        let mut languages = div().flex().flex_col().gap_1();
        for code in settings::languages() {
            let selected = code == current;
            let name = t!("_language_name", locale = &code).to_string();
            let pick = code.clone();
            languages = languages.child(
                div()
                    .id(SharedString::from(format!("lang-{code}")))
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .cursor_pointer()
                    .when(selected, |d| d.bg(active_bg))
                    .hover(|s| s.bg(hover))
                    .child(div().w(px(14.)).child(if selected { "✓" } else { "" }))
                    .child(div().flex_1().child(name))
                    .child(div().text_color(muted).child(code))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_language(&pick, window, cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .flex_1()
            .flex()
            .justify_center()
            .p_8()
            .child(
                div()
                    .w(px(460.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().text_xl().font_weight(FontWeight::SEMIBOLD).child(t!("settings.title").to_string()))
                            .child(Button::new("settings-close").label(t!("settings.close")).on_click(cx.listener(|this, _, _, cx| {
                                this.show_settings = false;
                                cx.notify();
                            }))),
                    )
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("settings.language").to_string()))
                    .child(languages)
                    .child(div().text_xs().text_color(muted).child(t!("settings.contribute").to_string())),
            )
    }
}

fn icon_button(
    id: String,
    label: impl Into<SharedString>,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div().id(SharedString::from(id)).px_1().rounded_sm().hover(move |s| s.bg(hover)).child(label.into()).on_click(on_click)
}

impl Render for Humon {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, muted) = (theme.background, theme.foreground, theme.muted_foreground);
        let content = match self.active_tab.and_then(|id| self.tabs.iter().position(|t| t.id == id)) {
            _ if self.show_settings => self.render_settings(cx).into_any_element(),
            Some(ix) => self.render_tab(ix, window, cx).into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_4()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(img(logo()).size(px(112.)))
                .child(if self.conns.is_empty() { t!("app.empty_first") } else { t!("app.empty_pick") }.to_string())
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(!self.tabs.is_empty(), |d| d.child(self.render_tabs(cx)))
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` macro, which shadows `#[test]`.
    use super::{format_size, parse_color, visible_collections};
    use crate::mongo::{CollKind, CollectionInfo};

    fn cols(names: &[&str]) -> Vec<CollectionInfo> {
        names.iter().map(|n| CollectionInfo { name: n.to_string(), kind: CollKind::Collection }).collect()
    }

    fn names(v: Option<Vec<&CollectionInfo>>) -> Option<Vec<&str>> {
        v.map(|v| v.into_iter().map(|c| c.name.as_str()).collect())
    }

    #[test]
    fn search_filters_collections() {
        let c = cols(&["users", "orders", "UserEvents"]);
        assert_eq!(names(visible_collections("shop", Some(&c), "")), Some(vec!["users", "orders", "UserEvents"]));
        assert_eq!(names(visible_collections("shop", Some(&c), "user")), Some(vec!["users", "UserEvents"]), "case-insensitive");
        assert_eq!(names(visible_collections("shop", Some(&c), "sho")), Some(vec!["users", "orders", "UserEvents"]), "db name match shows all");
        assert_eq!(names(visible_collections("shop", Some(&c), "zzz")), None);
        // Not loaded yet: visible while searching only if the db name matches.
        assert_eq!(names(visible_collections("shop", None, "sho")), Some(vec![]));
        assert_eq!(names(visible_collections("shop", None, "users")), None);
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(20 * 1024 * 1024), "20 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn colors_parse_from_hex() {
        assert!(parse_color("#e5484d").is_some());
        assert!(parse_color("e5484d").is_none(), "needs the #");
        assert!(parse_color("#zzzzzz").is_none());
    }
}
