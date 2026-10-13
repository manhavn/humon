//! The collection tab layout: query bar, toolbar and the documents area.

use super::*;
use gpui_kit::component::Sizable;

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
        // Query row: the three boxes, then Find and Explain; the rest is quiet.
        let query_bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(ui::SPACE))
            .px(px(ui::INSET))
            .py(px(ui::SPACE))
            .border_b_1()
            .border_color(border)
            .text_size(px(ui::TEXT))
            .child(
                labeled("Filter", query_box(&tab.filter), muted)
                    .flex_grow(3.)
                    .flex_basis(px(0.)),
            )
            .child(
                labeled("Sort", query_box(&tab.sort), muted)
                    .flex_grow(1.)
                    .flex_basis(px(0.)),
            )
            .child(
                labeled("Project", query_box(&tab.projection), muted)
                    .flex_grow(1.)
                    .flex_basis(px(0.)),
            )
            .child(
                ui::primary("find", t!("tab.find")).on_click(cx.listener(move |this, _, window, cx| {
                    this.run_query(id, window, cx);
                })),
            )
            .child(
                ui::action("explain", t!("tab.explain")).on_click(cx.listener(move |this, _, window, cx| {
                    this.run_explain(id, window, cx);
                    cx.notify();
                })),
            )
            .child(
                ui::tool("history", t!("history.toggle"), tab.history_open).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.history_open = !tab.history_open;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                ui::tool("favorite", t!("history.save"), false).on_click(cx.listener(move |this, _, _, cx| {
                    this.save_favorite(id, cx);
                    cx.notify();
                })),
            )
            .child(
                ui::tool("reset", t!("tab.reset"), false).on_click(cx.listener(move |this, _, window, cx| {
                    this.reset_query(id, window, cx);
                })),
            );

        // Results row: where we are, how to look at it, what to do with it, paging.
        let page = tab.page;
        let views = [ViewMode::Tree, ViewMode::Table, ViewMode::Json, ViewMode::Schema];
        let view_switch = ui::segmented(("view", id))
            .children(
                views
                    .iter()
                    .zip(["tab.view_tree", "tab.view_table", "tab.view_json", "tab.view_schema"])
                    .map(|(mode, key)| {
                        Button::new(SharedString::from(format!("view-{key}")))
                            .small()
                            .label(t!(key))
                            .selected(tab.view == *mode)
                    }),
            )
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, _, cx| {
                if let (Some(tab), Some(&ix)) = (this.tab_mut(id), clicked.first()) {
                    tab.view = views[ix];
                }
                cx.notify();
            }));
        let size_switch = ui::segmented(("size", id))
            .children(PAGE_SIZES.iter().map(|size| {
                Button::new(SharedString::from(format!("size-{size}")))
                    .small()
                    .label(size.to_string())
                    .selected(tab.page_size == *size)
            }))
            .on_click(cx.listener(move |this, clicked: &Vec<usize>, window, cx| {
                let Some(&ix) = clicked.first() else { return };
                if let Some(tab) = this.tab_mut(id) {
                    tab.page_size = PAGE_SIZES[ix];
                }
                this.refresh_code(id, window, cx);
                this.go_to_page(id, 0, window, cx);
            }));
        let mut tools = div().flex().items_center().gap(px(ui::SPACE / 2.)).child(
            ui::tool("open-shell", t!("tab.shell"), false).on_click(cx.listener(move |this, _, window, cx| {
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
            })),
        );
        tools = tools.child(
            ui::tool("export-toggle", t!("export.button"), tab.export_open).on_click(cx.listener(
                move |this, _, _, cx| {
                    if let Some(tab) = this.tab_mut(id) {
                        tab.export_open = !tab.export_open;
                    }
                    cx.notify();
                },
            )),
        );
        if tab.writable() {
            tools = tools.child(
                ui::tool("indexes-toggle", t!("tab.indexes"), tab.indexes_open).on_click(cx.listener(
                    move |this, _, window, cx| {
                        let open = this.tab_mut(id).map(|t| {
                            t.indexes_open = !t.indexes_open;
                            t.indexes_open
                        });
                        if open == Some(true) {
                            this.load_indexes(id, window, cx);
                        }
                        cx.notify();
                    },
                )),
            );
        }
        tools = tools.child(
            ui::tool("code-toggle", t!("tab.code"), tab.code_open).on_click(cx.listener(move |this, _, window, cx| {
                if let Some(tab) = this.tab_mut(id) {
                    tab.code_open = !tab.code_open;
                }
                this.refresh_code(id, window, cx);
                cx.notify();
            })),
        );
        if tab.writable() {
            tools = tools.child(
                ui::tool("add", t!("tab.add"), false)
                    .disabled(tab.draft.is_some())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_draft(id, "{\n  \n}".into(), window, cx);
                    })),
            );
        }
        let toolbar = ui::bar(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(ui::SPACE))
                    .min_w(px(120.))
                    .text_color(muted)
                    .child(range)
                    .when(tab.loading, |d| d.child(ui::note(t!("tab.loading"), muted)))
                    .when_some(tab.notice.clone(), |d, n| d.child(ui::note(n, accent))),
            )
            .child(view_switch)
            .child(div().flex_1())
            .child(tools)
            .child(size_switch)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(ui::SPACE / 2.))
                    .child(
                        ui::icon_tool("prev", IconName::ChevronLeft, t!("tab.previous"))
                            .disabled(page == 0 || tab.loading)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.go_to_page(id, page.saturating_sub(1), window, cx);
                            })),
                    )
                    .child(
                        ui::icon_tool("next", IconName::ChevronRight, t!("tab.next"))
                            .disabled(!has_next || tab.loading)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.go_to_page(id, page + 1, window, cx);
                            })),
                    )
                    .child(
                        ui::icon_tool("refresh", IconName::RefreshCw, t!("tab.refresh"))
                            .disabled(tab.loading)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.refresh(id, window, cx);
                            })),
                    ),
            );

        let mut docs = div()
            .id(("docs", id))
            .flex_1()
            .p(px(ui::INSET))
            .flex()
            .flex_col()
            .gap(px(ui::SPACE));
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
            .when(tab.indexes_open, |d| d.child(self.render_indexes(tab, cx)))
            .when(tab.explain.is_some() || tab.explaining, |d| {
                d.child(self.render_explain(tab, cx))
            })
            .when_some(tab.error.clone(), |d, e| {
                d.child(
                    div()
                        .px(px(ui::INSET))
                        .py(px(ui::SPACE))
                        .text_size(px(ui::TEXT))
                        .text_color(danger)
                        .child(e),
                )
            })
            .child(docs)
            .child(self.render_status_bar(tab, cx))
    }
}

/// A query box with its name to its left, quiet.
pub(crate) fn labeled(label: &'static str, input: impl IntoElement, muted: Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(ui::SPACE))
        .min_w_0()
        .child(
            div()
                .flex_none()
                .text_size(px(ui::SMALL))
                .text_color(muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(input))
}
