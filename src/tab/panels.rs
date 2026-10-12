//! The query history and export panels, and the export itself.

use super::*;

impl Humon {
    /// Asks where to save, then streams the export there on tokio.
    pub(crate) fn export(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if tab.exporting {
            return;
        }
        let format = tab.export_format;
        let args = FindArgs {
            filter: tab.applied.filter.clone(),
            sort: tab.applied.sort.clone(),
            projection: tab.applied.projection.clone(),
            skip: if tab.export_all {
                0
            } else {
                tab.page * tab.page_size as u64
            },
            limit: if tab.export_all { 0 } else { tab.page_size },
        };
        let (client, db, coll) = (tab.client.clone(), tab.db.clone(), tab.coll.clone());
        let downloads = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Downloads");
        let picked = cx.prompt_for_new_path(&downloads, Some(&format!("{coll}.{}", format.extension())));
        tab.exporting = true;
        tab.notice = Some(t!("export.running").into());
        cx.spawn_in(window, async move |this, cx| {
            let path = match picked.await {
                Ok(Ok(Some(path))) => path,
                _ => {
                    // Dialog cancelled.
                    this.update(cx, |this, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.exporting = false;
                            tab.notice = None;
                        }
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let res = RT.spawn(mongo::export(client, db, coll, args, format, path)).await;
            this.update(cx, |this, cx| {
                if let Some(tab) = this.tab_mut(id) {
                    tab.exporting = false;
                    match res {
                        Ok(Ok(n)) => {
                            tab.notice = Some(t!("export.done", n = n, file = file).into());
                            tab.export_open = false;
                        }
                        Ok(Err(e)) => {
                            tab.notice = None;
                            tab.error = Some(e);
                        }
                        Err(e) => {
                            tab.notice = None;
                            tab.error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Format and scope choice for exporting the current query.
    pub(crate) fn render_export(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, panel) = (theme.border, theme.muted_foreground, theme.sidebar);
        let id = tab.id;
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .bg(panel)
            .text_xs();
        for (format, label) in [(ExportFormat::Json, "JSON"), (ExportFormat::Csv, "CSV")] {
            row = row.child(
                Button::new(SharedString::from(format!("format-{label}")))
                    .label(label)
                    .when(tab.export_format == format, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.export_format = format;
                        }
                        cx.notify();
                    })),
            );
        }
        let all_label = match &tab.count {
            Count::Known(n) => t!("export.all_count", n = thousands(*n)).to_string(),
            _ => t!("export.all").to_string(),
        };
        row = row.child(div().w(px(12.)));
        for (all, label) in [
            (false, t!("export.page", n = tab.docs.len()).to_string()),
            (true, all_label),
        ] {
            row = row.child(
                Button::new(SharedString::from(format!("scope-{all}")))
                    .label(label)
                    .when(tab.export_all == all, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.export_all = all;
                        }
                        cx.notify();
                    })),
            );
        }
        row.child(div().flex_1())
            .when(tab.export_format == ExportFormat::Csv, |d| {
                d.child(div().text_color(muted).child(t!("export.csv_note").to_string()))
            })
            .child(
                Button::new("export-save")
                    .primary()
                    .label(if tab.exporting {
                        t!("export.running")
                    } else {
                        t!("export.save")
                    })
                    .disabled(tab.exporting)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.export(id, window, cx);
                        cx.notify();
                    })),
            )
    }

    /// Favorites first, then recent queries; clicking one runs it.
    pub(crate) fn render_history(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, panel, warning) = (
            theme.border,
            theme.muted_foreground,
            theme.list_hover,
            theme.sidebar,
            theme.warning,
        );
        let mono = theme.mono_font_family.clone();
        let id = tab.id;
        let key = history::key(&tab.conn_id, &tab.db, &tab.coll);
        let favorites = self.history.favorites(&key);
        let recent = self.history.recent(&key);
        let row = |prefix: String, query: &Query, icon: &'static str, color: Hsla| {
            let q = query.clone();
            div()
                .id(SharedString::from(format!("{prefix}-{id}")))
                .flex()
                .gap_2()
                .items_center()
                .px_3()
                .py_1()
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .child(div().w(px(14.)).text_color(color).child(icon))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .font_family(mono.clone())
                        .child(query.summary()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.apply_query(id, q.clone(), window, cx);
                    cx.notify();
                }))
        };
        let mut list = div()
            .id(("history", id))
            .flex()
            .flex_col()
            .max_h(px(260.))
            .overflow_y_scroll()
            .border_b_1()
            .border_color(border)
            .bg(panel)
            .text_xs();
        if favorites.is_empty() && recent.is_empty() {
            return list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(muted)
                    .child(t!("history.empty").to_string()),
            );
        }
        if !favorites.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_color(muted)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("history.favorites").to_string()),
            );
            for (i, fav) in favorites.iter().enumerate() {
                let k = key.clone();
                list = list.child(
                    row(format!("fav-{i}"), fav, "★", warning).child(
                        div()
                            .id(SharedString::from(format!("unfav-{i}-{id}")))
                            .px_1()
                            .text_color(muted)
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.history.remove_favorite(&k, i);
                                this.persist_history();
                                cx.notify();
                            })),
                    ),
                );
            }
        }
        if !recent.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_color(muted)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("history.recent").to_string()),
            );
            for (i, q) in recent.iter().enumerate() {
                list = list.child(row(format!("recent-{i}"), q, "↺", muted));
            }
        }
        list
    }
}
