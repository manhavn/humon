//! The sidebar: saved connections, the database/collection tree and search.

use super::*;

impl Humon {
    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
}

pub(crate) fn icon_button(
    id: String,
    label: impl Into<SharedString>,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div().id(SharedString::from(id)).px_1().rounded_sm().hover(move |s| s.bg(hover)).child(label.into()).on_click(on_click)
}
