//! Root view: sidebar with saved connections and the db/collection tree, the
//! tab bar, and the active collection tab (see tab.rs).

use std::future::Future;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::Client;

use crate::mongo::{self, CollKind, CollectionInfo, DatabaseInfo, RT};
use crate::store::{self, SavedConnection};
use crate::tab::Tab;

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
    form_error: Option<String>,
    /// Saved id awaiting a second click to confirm deletion.
    confirm_delete: Option<String>,
    load_error: Option<String>,
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
        let form_name = cx.new(|cx| InputState::new(window, cx).placeholder("Ad, örn. Prod Atlas"));
        let form_uri = cx.new(|cx| InputState::new(window, cx).placeholder("mongodb+srv://kullanici:sifre@cluster.mongodb.net"));
        let (saved, load_error) = match store::list() {
            Ok(list) => (list, None),
            Err(e) => (vec![], Some(e)),
        };
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
            form_error: None,
            confirm_delete: None,
            load_error,
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
            self.form_error = Some("Bağlantı adresi boş olamaz".into());
            return;
        }
        match store::save(&name, &uri) {
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
        let conn = &mut self.conns[ix];
        conn.connecting = true;
        conn.error = None;
        let uri = store::resolve_uri(&conn.saved);
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
        match store::delete(saved_id) {
            Ok(()) => self.conns.retain(|c| c.saved.id != saved_id),
            Err(e) => {
                if let Some(ix) = self.conn_ix(saved_id) {
                    self.conns[ix].error = Some(e);
                }
            }
        }
    }

    fn refresh_dbs(&mut self, saved_id: &str, window: &mut Window, cx: &mut Context<Self>) {
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
        let conn = &mut self.conns[ix];
        let Some(client) = conn.client.clone() else { return };
        let db = &mut conn.dbs[db_ix];
        db.open = !db.open;
        if !db.open || db.collections.is_some() {
            return;
        }
        db.loading = true;
        let (id, name) = (saved_id.to_string(), db.info.name.clone());
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

    fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(theme.border)
            .child(Input::new(&self.form_name))
            .child(Input::new(&self.form_uri))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .when(!self.conns.is_empty(), |d| {
                        d.child(Button::new("form-cancel").label("İptal").on_click(cx.listener(|this, _, _, cx| {
                            this.adding = false;
                            cx.notify();
                        })))
                    })
                    .child(Button::new("form-save").primary().label("Kaydet ve bağlan").on_click(cx.listener(|this, _, window, cx| {
                        this.save_connection(window, cx);
                        cx.notify();
                    }))),
            )
            .when_some(self.form_error.clone(), |d, e| d.child(div().text_xs().text_color(theme.danger).child(e)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, hover, active_bg, accent) = (theme.muted_foreground, theme.danger, theme.list_hover, theme.list_active, theme.primary);
        let (sidebar_bg, border, warning) = (theme.sidebar, theme.border, theme.warning);
        let active = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id));

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
                    .child("Bağlantılar")
                    .child(
                        div().id("add-conn").px_2().rounded_sm().cursor_pointer().hover(|s| s.bg(hover)).child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.adding = !this.adding;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.adding, |d| d.child(self.render_form(cx)))
            .when_some(self.load_error.clone(), |d, e| d.child(div().px_3().text_xs().text_color(danger).child(e)));

        for conn in &self.conns {
            let id = conn.saved.id.clone();
            let connected = conn.client.is_some();
            let dot = if conn.connecting { warning } else if connected { accent } else { border };
            let confirm = self.confirm_delete.as_deref() == Some(id.as_str());
            let group: SharedString = format!("conn-{id}").into();
            let row = div()
                .id(SharedString::from(format!("conn-{id}")))
                .group(group.clone())
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .on_click(cx.listener({
                    let id = id.clone();
                    move |this, _, window, cx| {
                        this.toggle_conn(&id, window, cx);
                        cx.notify();
                    }
                }))
                .child(div().text_color(muted).child(if conn.open { "▾" } else { "▸" }))
                .child(div().size(px(7.)).rounded_full().bg(dot))
                .child(div().flex_1().min_w_0().overflow_hidden().font_weight(FontWeight::SEMIBOLD).child(conn.saved.name.clone()))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .text_sm()
                        .text_color(muted)
                        .when(!confirm, |d| d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)))
                        .when(connected, |d| {
                            d.child(icon_button(format!("refresh-{id}"), "↻", hover, cx.listener({
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
                            icon_button(format!("del-{id}"), if confirm { "Sil?" } else { "×" }, hover, cx.listener({
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
            if conn.connecting {
                list = list.child(div().pl_8().text_xs().text_color(muted).child("Bağlanıyor…"));
            }
            if let Some(e) = &conn.error {
                list = list.child(div().pl_8().pr_3().text_xs().text_color(danger).child(e.clone()));
            }
            if !conn.open {
                continue;
            }
            for (di, db) in conn.dbs.iter().enumerate() {
                list = list.child(
                    div()
                        .id(SharedString::from(format!("db-{id}-{di}")))
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
                        .child(div().text_color(muted).child(if db.open { "▾" } else { "▸" }))
                        .child(div().flex_1().min_w_0().overflow_hidden().child(db.info.name.clone()))
                        .child(div().text_xs().text_color(muted).child(format_size(db.info.size_on_disk))),
                );
                if !db.open {
                    continue;
                }
                if db.loading {
                    list = list.child(div().pl_16().text_xs().text_color(muted).child("Yükleniyor…"));
                } else if db.collections.as_ref().is_some_and(|c| c.is_empty()) {
                    list = list.child(div().pl_16().text_xs().text_color(muted).child("Collection yok"));
                }
                for coll in db.collections.iter().flatten() {
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
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("coll-{id}-{di}-{}", coll.name)))
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
                                d.child(div().text_xs().text_color(muted).child(if kind == CollKind::View { "view" } else { "timeseries" }))
                            }),
                    );
                }
            }
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
            let group: SharedString = format!("tab-{id}").into();
            bar = bar.child(
                div()
                    .id(("tab", id))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .max_w(px(220.))
                    .pl_3()
                    .pr_1()
                    .py_1()
                    .border_r_1()
                    .border_color(border)
                    .cursor_pointer()
                    .text_color(if active { fg } else { muted })
                    .when(active, |d| d.bg(bg).border_t_2().border_color(accent))
                    .when(!active, |d| d.hover(|s| s.bg(hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_tab = Some(id);
                        cx.notify();
                    }))
                    .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| {
                        this.close_tab(id);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::SEMIBOLD).overflow_hidden().whitespace_nowrap().child(tab.coll.clone()))
                            .child(div().text_xs().text_color(muted).overflow_hidden().whitespace_nowrap().child(tab.db.clone())),
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

fn icon_button(
    id: String,
    label: &'static str,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div().id(SharedString::from(id)).px_1().rounded_sm().hover(move |s| s.bg(hover)).child(label).on_click(on_click)
}

impl Render for Humon {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, muted) = (theme.background, theme.foreground, theme.muted_foreground);
        let content = match self.active_tab.and_then(|id| self.tabs.iter().position(|t| t.id == id)) {
            Some(ix) => self.render_tab(ix, window, cx).into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(if self.conns.is_empty() {
                    "Soldaki formdan ilk bağlantını ekle."
                } else {
                    "Soldan bir bağlantı ve collection seç. ⌘ + tık ya da orta tık yeni sekmede açar."
                })
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
