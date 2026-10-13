//! The query history and export panels, and the export itself.

use super::*;
use crate::explain::Verdict;
use gpui_kit::component::Sizable;

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
        let target = tab.target.clone();
        let downloads = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Downloads");
        let picked = cx.prompt_for_new_path(&downloads, Some(&format!("{}.{}", target.name, format.extension())));
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
            let res = RT.spawn(target.export(args, format, path)).await;
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
        let mut row = ui::panel(border, panel).flex_row().flex_wrap().items_center();
        let formats = [ExportFormat::Json, ExportFormat::Csv];
        row = row.child(
            ui::segmented("export-format")
                .children(formats.iter().zip(["JSON", "CSV"]).map(|(format, label)| {
                    Button::new(SharedString::from(format!("format-{label}")))
                        .label(label)
                        .selected(tab.export_format == *format)
                }))
                .on_click(cx.listener(move |this, clicked: &Vec<usize>, _, cx| {
                    if let (Some(tab), Some(&ix)) = (this.tab_mut(id), clicked.first()) {
                        tab.export_format = formats[ix];
                    }
                    cx.notify();
                })),
        );
        let all_label = match &tab.count {
            Count::Known(n) => t!("export.all_count", n = thousands(*n)).to_string(),
            _ => t!("export.all").to_string(),
        };
        let page_label = t!("export.page", n = tab.docs.len()).to_string();
        row = row.child(
            ui::segmented("export-scope")
                .child(Button::new("scope-page").label(page_label).selected(!tab.export_all))
                .child(Button::new("scope-all").label(all_label).selected(tab.export_all))
                .on_click(cx.listener(move |this, clicked: &Vec<usize>, _, cx| {
                    if let (Some(tab), Some(&ix)) = (this.tab_mut(id), clicked.first()) {
                        tab.export_all = ix == 1;
                    }
                    cx.notify();
                })),
        );
        row.child(div().flex_1())
            .when(tab.export_format == ExportFormat::Csv, |d| {
                d.child(div().text_color(muted).child(t!("export.csv_note").to_string()))
            })
            .child(
                Button::new("export-save")
                    .small()
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
        let key = history::key(&tab.conn_id, &tab.target.db, &tab.target.name);
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
            .flex_none()
            .max_h(px(260.))
            .py(px(ui::SPACE / 2.))
            .overflow_y_scroll()
            .border_b_1()
            .border_color(border)
            .bg(panel);
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

    /// Rewrites the code panel from what the query bar holds now.
    pub(crate) fn refresh_code(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        if !tab.code_open {
            return;
        }
        let find = crate::codegen::Find::parse(
            &tab.target.db,
            &tab.target.name,
            &tab.typed_query(cx),
            Some(tab.page_size),
        );
        match find {
            Ok(find) => {
                tab.code_error = None;
                let code = crate::codegen::to_code(tab.code_lang, &find);
                tab.code_view.update(cx, |s, cx| s.set_value(code, window, cx));
            }
            Err(e) => tab.code_error = Some(e),
        }
    }

    /// The query in another language, to copy into an application.
    pub(crate) fn render_code(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, panel, danger) = (theme.border, theme.sidebar, theme.danger);
        let id = tab.id;
        let langs = ui::segmented("code-lang")
            .children(crate::codegen::Lang::ALL.iter().map(|lang| {
                Button::new(SharedString::from(format!("code-{}", lang.label())))
                    .label(lang.label())
                    .selected(tab.code_lang == *lang)
            }))
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, window, cx| {
                if let (Some(tab), Some(&ix)) = (this.tab_mut(id), clicked.first()) {
                    tab.code_lang = crate::codegen::Lang::ALL[ix];
                }
                this.refresh_code(id, window, cx);
                cx.notify();
            }));
        let code = tab.code_view.read(cx).value().to_string();
        let lines = code.lines().count().max(1);
        ui::panel(border, panel)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(ui::SPACE))
                    .child(langs)
                    .child(div().flex_1())
                    .child(
                        Button::new("code-copy")
                            .small()
                            .label(t!("card.copy"))
                            .disabled(tab.code_error.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                                if let Some(tab) = this.tab_mut(id) {
                                    tab.notice = Some(t!("tree.copied").into());
                                }
                                cx.notify();
                            })),
                    ),
            )
            .map(|d| match &tab.code_error {
                Some(e) => d.child(div().text_color(danger).child(e.clone())),
                None => d.child(
                    Editor::new(&tab.code_view)
                        .readonly(true)
                        .h(px((lines as f32 * 18. + 16.).clamp(52., 320.))),
                ),
            })
    }

    pub(crate) fn load_indexes(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.indexes = None;
        let fut = tab.target.clone().indexes();
        self.run(fut, window, cx, move |this, res, _, _| {
            if let Some(tab) = this.tab_mut(id) {
                tab.indexes = Some(res);
            }
        });
    }

    fn create_index(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let keys = tab.index_keys.read(cx).value().to_string();
        tab.index_error = None;
        let fut = tab.target.clone().create_index(keys, tab.index_unique);
        self.run(fut, window, cx, move |this, res, window, cx| {
            let Some(tab) = this.tab_mut(id) else { return };
            match res {
                Ok(name) => {
                    tab.notice = Some(t!("indexes.created", name = name).into());
                    tab.index_keys.update(cx, |s, cx| s.set_value("", window, cx));
                    this.load_indexes(id, window, cx);
                }
                Err(e) => tab.index_error = Some(e),
            }
        });
    }

    fn drop_index(&mut self, id: usize, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        if tab.confirm_drop_index.as_deref() != Some(name.as_str()) {
            tab.confirm_drop_index = Some(name);
            return;
        }
        tab.confirm_drop_index = None;
        let fut = tab.target.clone().drop_index(name);
        self.run(fut, window, cx, move |this, res, window, cx| match res {
            Ok(()) => this.load_indexes(id, window, cx),
            Err(e) => {
                if let Some(tab) = this.tab_mut(id) {
                    tab.index_error = Some(e);
                }
            }
        });
    }

    /// Explains what the query bar holds now.
    pub(crate) fn run_explain(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let Query {
            filter,
            sort,
            projection,
        } = tab.typed_query(cx);
        let args = FindArgs {
            filter,
            sort,
            projection,
            skip: 0,
            limit: tab.page_size,
        };
        tab.explaining = true;
        let fut = tab.target.clone().explain(args);
        self.run(fut, window, cx, move |this, res, _, _| {
            if let Some(tab) = this.tab_mut(id) {
                tab.explaining = false;
                tab.explain = Some(res);
            }
        });
    }

    pub(crate) fn render_indexes(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, panel, danger) = (theme.border, theme.muted_foreground, theme.sidebar, theme.danger);
        let mono = theme.mono_font_family.clone();
        let id = tab.id;
        let mut list = div().flex().flex_col().gap_1();
        match &tab.indexes {
            None => list = list.child(div().text_color(muted).child(t!("indexes.loading").to_string())),
            Some(Err(e)) => list = list.child(div().text_color(danger).child(e.clone())),
            Some(Ok(indexes)) => {
                for index in indexes {
                    let mut badges = Vec::new();
                    if index.unique {
                        badges.push(t!("indexes.unique").to_string());
                    }
                    if index.sparse {
                        badges.push(t!("indexes.sparse").to_string());
                    }
                    if index.partial {
                        badges.push(t!("indexes.partial").to_string());
                    }
                    if let Some(s) = index.ttl {
                        badges.push(t!("indexes.ttl", seconds = s).to_string());
                    }
                    let name = index.name.clone();
                    let confirm = tab.confirm_drop_index.as_deref() == Some(index.name.as_str());
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(180.))
                                    .flex_none()
                                    .truncate()
                                    .font_family(mono.clone())
                                    .child(index.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(mono.clone())
                                    .child(index.keys.clone()),
                            )
                            .child(div().text_color(muted).child(badges.join(" · ")))
                            .when(index.name != "_id_", |d| {
                                d.child(
                                    Button::new(SharedString::from(format!("drop-index-{}", index.name)))
                                        .small()
                                        .label(if confirm {
                                            t!("card.confirm_delete")
                                        } else {
                                            t!("indexes.drop")
                                        })
                                        .when(confirm, |b| b.danger())
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.drop_index(id, name.clone(), window, cx);
                                            cx.notify();
                                        })),
                                )
                            }),
                    );
                }
            }
        }
        let create = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_none().child(t!("indexes.new").to_string()))
            .child(div().flex_1().min_w_0().child(query_box(&tab.index_keys)))
            .child(
                gpui_kit::component::checkbox::Checkbox::new("index-unique")
                    .label(t!("indexes.unique").to_string())
                    .checked(tab.index_unique)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.index_unique = *checked;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("index-create")
                    .small()
                    .primary()
                    .label(t!("indexes.create"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.create_index(id, window, cx);
                        cx.notify();
                    })),
            );
        ui::panel(border, panel)
            .child(list)
            .child(create)
            .when_some(tab.index_error.clone(), |d, e| {
                d.child(div().text_color(danger).child(e))
            })
    }

    pub(crate) fn render_explain(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, panel, danger, warning, success) = (
            theme.border,
            theme.muted_foreground,
            theme.sidebar,
            theme.danger,
            theme.warning,
            theme.success,
        );
        let id = tab.id;
        let body = match (&tab.explain, tab.explaining) {
            (_, true) | (None, _) => div().text_color(muted).child(t!("explain.running").to_string()),
            (Some(Err(e)), _) => div().text_color(danger).child(e.clone()),
            (Some(Ok(s)), _) => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(match s.verdict() {
                    Verdict::CollectionScan => div().text_color(warning).child(t!("explain.collscan").to_string()),
                    Verdict::Index(names) => div()
                        .text_color(success)
                        .child(t!("explain.index", names = names.join(", ")).to_string()),
                    Verdict::Plan(stage) => div().child(t!("explain.plan", stage = stage).to_string()),
                    Verdict::Unknown => div().text_color(muted).child(t!("explain.unknown").to_string()),
                })
                .child(
                    div().child(
                        t!(
                            "explain.stats",
                            returned = thousands(s.returned as u64),
                            docs = thousands(s.docs_examined as u64),
                            keys = thousands(s.keys_examined as u64),
                            ms = s.millis
                        )
                        .to_string(),
                    ),
                )
                .child(div().text_color(muted).child(s.stages.join(" → "))),
        };
        ui::panel(border, panel)
            .flex_row()
            .items_start()
            .child(div().flex_1().child(body))
            .child(
                Button::new("explain-close")
                    .small()
                    .label(t!("settings.close"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.explain = None;
                        }
                        cx.notify();
                    })),
            )
    }
}
