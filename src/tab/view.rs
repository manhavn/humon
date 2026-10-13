//! The collection tab layout: query bar, toolbar and the documents area.

use super::*;

impl Humon {
    pub fn render_tab(&self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = &self.tabs[ix];
        let id = tab.id;
        let theme = cx.theme();
        let (border, muted, danger, accent, primary) = (
            theme.border,
            theme.muted_foreground,
            theme.danger,
            theme.success,
            theme.primary,
        );
        let syntax = theme.highlight_theme.clone();

        let window = page_window(tab.page, tab.page_size, tab.docs.len(), &tab.count);
        let count_text = match &tab.count {
            Count::Counting => t!("tab.counting").to_string(),
            Count::Known(n) => thousands(*n),
            Count::TooSlow | Count::Failed(_) => t!("tab.many").to_string(),
        };
        let range = if tab.docs.is_empty() {
            if tab.loading || tab.error.is_some() {
                String::new()
            } else {
                t!("tab.no_docs").into()
            }
        } else {
            t!(
                "tab.range",
                from = thousands(window.from),
                to = thousands(window.to),
                total = count_text
            )
            .into()
        };
        let has_next = window.has_next;
        let query_bar =
            div()
                .flex()
                .gap_2()
                .items_end()
                .p_3()
                .border_b_1()
                .border_color(border)
                .child(
                    labeled("Filter", query_box(&tab.filter), muted)
                        .flex_grow(1.)
                        .flex_basis(relative(0.5)),
                )
                .child(
                    labeled("Sort", query_box(&tab.sort), muted)
                        .flex_grow(1.)
                        .flex_basis(relative(0.2)),
                )
                .child(
                    labeled("Project", query_box(&tab.projection), muted)
                        .flex_grow(1.)
                        .flex_basis(relative(0.2)),
                )
                .child(
                    Button::new("find")
                        .primary()
                        .label(t!("tab.find"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_query(id, window, cx);
                        })),
                )
                .child(
                    Button::new("reset")
                        .label(t!("tab.reset"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reset_query(id, window, cx);
                        })),
                )
                .child(
                    Button::new("history")
                        .label(t!("history.toggle"))
                        .when(tab.history_open, |b| b.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(tab) = this.tab_mut(id) {
                                tab.history_open = !tab.history_open;
                            }
                            cx.notify();
                        })),
                )
                .child(Button::new("favorite").label(t!("history.save")).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.save_favorite(id, cx);
                        cx.notify();
                    },
                )));

        let page = tab.page;
        let mut toolbar = div()
            .flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(border)
            .text_color(muted)
            .child(range)
            .when(tab.loading, |d| {
                d.child(div().text_xs().child(t!("tab.loading").to_string()))
            })
            .when_some(tab.notice.clone(), |d, n| {
                d.child(div().text_xs().text_color(accent).child(n))
            })
            .child(div().flex_1());
        toolbar = toolbar.child(Button::new("open-shell").label(t!("tab.shell")).on_click(cx.listener(
            move |this, _, window, cx| {
                let Some((conn, db, coll)) = this
                    .tab_mut(id)
                    .map(|t| (t.conn_id.clone(), t.target.db.clone(), t.target.name.clone()))
                else {
                    return;
                };
                if let Some(console) = this.open_console(&conn, &db, window, cx) {
                    this.set_console_input(console, crate::palette::shell_starter(&coll), window, cx);
                }
                cx.notify();
            },
        )));
        toolbar = toolbar.child(
            Button::new("export-toggle")
                .label(t!("export.button"))
                .when(tab.export_open, |b| b.primary())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(tab) = this.tab_mut(id) {
                        tab.export_open = !tab.export_open;
                    }
                    cx.notify();
                })),
        );
        toolbar = toolbar.child(
            Button::new("code-toggle")
                .label(t!("tab.code"))
                .when(tab.code_open, |b| b.primary())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(tab) = this.tab_mut(id) {
                        tab.code_open = !tab.code_open;
                    }
                    this.refresh_code(id, window, cx);
                    cx.notify();
                })),
        );
        if tab.writable() {
            toolbar = toolbar.child(
                Button::new("add")
                    .label(t!("tab.add"))
                    .disabled(tab.draft.is_some())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_draft(id, "{\n  \n}".into(), window, cx);
                    })),
            );
        }
        for (mode, key) in [
            (ViewMode::Tree, "tab.view_tree"),
            (ViewMode::Table, "tab.view_table"),
            (ViewMode::Json, "tab.view_json"),
            (ViewMode::Schema, "tab.view_schema"),
        ] {
            toolbar = toolbar.child(
                Button::new(SharedString::from(format!("view-{key}")))
                    .label(t!(key))
                    .when(tab.view == mode, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.view = mode;
                        }
                        cx.notify();
                    })),
            );
        }
        toolbar = toolbar.child(div().w(px(8.)));
        for size in PAGE_SIZES {
            let selected = tab.page_size == size;
            toolbar = toolbar.child(
                Button::new(SharedString::from(format!("size-{size}")))
                    .label(size.to_string())
                    .when(selected, |b| b.primary())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.page_size = size;
                        }
                        this.refresh_code(id, window, cx);
                        this.go_to_page(id, 0, window, cx);
                    })),
            );
        }
        toolbar = toolbar
            .child(
                Button::new("prev")
                    .label("‹")
                    .disabled(page == 0 || tab.loading)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.go_to_page(id, page.saturating_sub(1), window, cx);
                    })),
            )
            .child(
                Button::new("next")
                    .label("›")
                    .disabled(!has_next || tab.loading)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.go_to_page(id, page + 1, window, cx);
                    })),
            )
            .child(
                Button::new("refresh")
                    .label("↻")
                    .disabled(tab.loading)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.refresh(id, window, cx);
                    })),
            );

        let mut docs = div().id(("docs", id)).flex_1().p_3().flex().flex_col().gap_2();
        // The table also scrolls sideways; the tree only needs vertical scroll.
        docs = if tab.view == ViewMode::Table {
            docs.overflow_scroll()
        } else {
            docs.overflow_y_scroll()
        };
        if let Some(draft) = &tab.draft {
            docs = docs.child(
                div()
                    .p_2()
                    .border_1()
                    .border_color(primary)
                    .rounded_md()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("tab.new_document").to_string())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("tab.array_hint").to_string()),
                            ),
                    )
                    .child(self.render_editor(id, draft, t!("tab.insert").into(), true, cx)),
            );
        }
        match tab.view {
            ViewMode::Tree => {
                for (i, doc) in tab.docs.iter().enumerate() {
                    docs = docs.child(self.render_card(tab, i, doc, &syntax, cx));
                }
            }
            ViewMode::Json => {
                docs = docs.child(
                    div()
                        .flex_1()
                        .min_h(px(200.))
                        .child(Editor::new(&tab.json).readonly(true).h(relative(1.))),
                );
            }
            ViewMode::Schema => {
                docs = docs.child(self.render_schema(tab, &syntax, cx));
            }
            ViewMode::Table => {
                if let Some((_, _, editor)) = &tab.editing {
                    docs = docs.child(
                        div()
                            .p_2()
                            .border_1()
                            .border_color(primary)
                            .rounded_md()
                            .child(self.render_editor(id, editor, t!("tab.save").into(), false, cx)),
                    );
                }
                if !tab.docs.is_empty() {
                    docs = docs.child(self.render_table(tab, &syntax, cx));
                }
            }
        }

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(query_bar)
            .when(tab.history_open, |d| d.child(self.render_history(tab, cx)))
            .child(toolbar)
            .when(tab.export_open, |d| d.child(self.render_export(tab, cx)))
            .when(tab.code_open, |d| d.child(self.render_code(tab, cx)))
            .when_some(tab.error.clone(), |d, e| {
                d.child(div().px_3().py_2().text_color(danger).child(e))
            })
            .child(docs)
            .child(self.render_status_bar(tab, cx))
    }
}

pub(crate) fn labeled(label: &'static str, input: impl IntoElement, muted: Hsla) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w_0()
        .child(div().text_xs().text_color(muted).child(label))
        .child(input)
}
