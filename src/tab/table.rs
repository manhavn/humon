//! Documents as a table, one column per top-level field.

use super::*;

pub(crate) const COLUMN_WIDTH: f32 = 200.;

pub(crate) const ACTIONS_WIDTH: f32 = 72.;

impl Humon {
    /// Top-level fields as columns; nested values show as a summary.
    pub(crate) fn render_table(
        &self,
        tab: &Tab,
        syntax: &Arc<HighlightTheme>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, panel, danger, fg) = (
            theme.border,
            theme.muted_foreground,
            theme.list_hover,
            theme.sidebar,
            theme.danger,
            theme.foreground,
        );
        let mono = theme.mono_font_family.clone();
        let id = tab.id;
        let columns = table_columns(&tab.docs);
        let width = ACTIONS_WIDTH + COLUMN_WIDTH * columns.len() as f32;
        let writable = tab.writable();

        let cell = |w: f32| {
            div()
                .flex_none()
                .w(px(w))
                .px_2()
                .py_1()
                .border_r_1()
                .border_color(border)
                .truncate()
        };
        let mut header = div()
            .flex()
            .flex_none()
            .w(px(width))
            .bg(panel)
            .border_b_1()
            .border_color(border)
            .font_weight(FontWeight::SEMIBOLD)
            .child(cell(ACTIONS_WIDTH));
        for c in &columns {
            header = header.child(cell(COLUMN_WIDTH).text_color(fg).child(c.clone()));
        }
        let mut table = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(width))
            .border_1()
            .border_color(border)
            .rounded_md()
            .font_family(mono)
            .text_xs()
            .child(header);

        for (ix, doc) in tab.docs.iter().enumerate() {
            let confirm = tab.confirm_delete == Some(ix);
            let has_id = doc.get("_id").is_some();
            let icon = |key: &str, label: &'static str| {
                div()
                    .id(SharedString::from(format!("t{key}-{id}-{ix}")))
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(hover))
                    .child(label)
            };
            let mut actions = div().flex().gap_1().text_color(muted);
            if writable && has_id {
                actions = actions.child(
                    icon("edit", "✎")
                        .on_click(cx.listener(move |this, _, window, cx| this.start_edit(id, ix, window, cx))),
                );
            }
            let shell = to_shell(doc, "");
            actions = actions.child(
                icon("copy", "⧉")
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(shell.clone()))),
            );
            if writable && has_id {
                actions = actions.child(
                    icon("delete", if confirm { "?" } else { "🗑" })
                        .when(confirm, |d| d.text_color(danger))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.delete_doc(id, ix, window, cx);
                            cx.notify();
                        })),
                );
            }
            let mut row = div()
                .flex()
                .flex_none()
                .w(px(width))
                .border_b_1()
                .border_color(border)
                .hover(|s| s.bg(hover))
                .child(cell(ACTIONS_WIDTH).child(actions));
            for c in &columns {
                let content = match doc.get(c) {
                    None => div(),
                    Some(Value::Object(m)) if !is_typed(m) => div()
                        .text_color(muted)
                        .child(t!("tree.fields", n = m.len()).to_string()),
                    Some(Value::Array(a)) => div().text_color(muted).child(t!("tree.array", n = a.len()).to_string()),
                    Some(v) => {
                        // One line per cell: newlines would break the row height.
                        let text = to_shell(v, "").replace('\n', "⏎");
                        let highlights: Vec<_> = tokenize(&text)
                            .into_iter()
                            .filter_map(|(r, k)| syntax.style(k).map(|s| (r, s)))
                            .collect();
                        div().child(StyledText::new(text).with_highlights(highlights))
                    }
                };
                row = row.child(cell(COLUMN_WIDTH).child(content));
            }
            table = table.child(row);
            if let Some((i, e)) = &tab.card_error
                && *i == ix
            {
                table = table.child(div().px_2().py_1().text_color(danger).child(e.clone()));
            }
        }
        table
    }
}

/// Column order for the table: `_id` first, then fields in the order they first appear.
pub(crate) fn table_columns(docs: &[Value]) -> Vec<String> {
    let mut columns: Vec<String> = Vec::new();
    for doc in docs {
        if let Value::Object(m) = doc {
            for k in m.keys() {
                if !columns.iter().any(|c| c == k) {
                    columns.push(k.clone());
                }
            }
        }
    }
    if let Some(pos) = columns.iter().position(|c| c == "_id") {
        let id = columns.remove(pos);
        columns.insert(0, id);
    }
    columns
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` macro, which shadows `#[test]`.
    use super::table_columns;

    #[test]
    fn table_columns_keep_first_seen_order_with_id_first() {
        let docs = vec![
            serde_json::json!({ "name": "a", "_id": 1 }),
            serde_json::json!({ "age": 3, "name": "b" }),
        ];
        assert_eq!(table_columns(&docs), ["_id", "name", "age"]);
    }
}
