//! Documents as cards with a colored, collapsible field tree.

use super::*;

impl Humon {
    pub(crate) fn render_card(&self, tab: &Tab, ix: usize, doc: &Value, syntax: &Arc<HighlightTheme>, cx: &mut Context<Self>) -> AnyElement {
        let id = tab.id;
        let theme = cx.theme();
        let (border, muted, danger, hover, bg) = (theme.border, theme.muted_foreground, theme.danger, theme.list_hover, theme.background);
        let (primary, mono) = (theme.primary, theme.mono_font_family.clone());

        if let Some((edit_ix, _, editor)) = &tab.editing
            && *edit_ix == ix
        {
            return div()
                .p_2()
                .border_1()
                .border_color(primary)
                .rounded_md()
                .child(self.render_editor(id, editor, t!("tab.save").into(), false, cx))
                .into_any_element();
        }

        let writable = tab.writable() && doc.get("_id").is_some();
        let confirm = tab.confirm_delete == Some(ix);
        let group: SharedString = format!("card-{id}-{ix}").into();
        let action = |label: String, key: &str| {
            div()
                .id(SharedString::from(format!("{key}-{id}-{ix}")))
                .px_2()
                .border_1()
                .border_color(border)
                .rounded_sm()
                .bg(bg)
                .text_xs()
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .child(label)
        };
        let mut actions = div()
            .absolute()
            .top_1()
            .right_2()
            .flex()
            .gap_1()
            .text_color(muted)
            .when(!confirm, |d| d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)));
        if writable {
            actions = actions
                .child(action(t!("card.edit").into(), "edit").on_click(cx.listener(move |this, _, window, cx| this.start_edit(id, ix, window, cx))))
                .child(action(t!("card.clone").into(), "clone").on_click(cx.listener(move |this, _, window, cx| {
                    let Some(doc) = this.tab_mut(id).and_then(|t| t.docs.get(ix).cloned()) else { return };
                    let mut copy = doc.as_object().cloned().unwrap_or_default();
                    copy.remove("_id");
                    this.open_draft(id, to_shell(&Value::Object(copy), ""), window, cx);
                })));
        }
        let shell = to_shell(doc, "");
        actions = actions.child(action(t!("card.copy").into(), "copy").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(shell.clone()));
        }));
        if writable {
            actions = actions.child(
                action(if confirm { t!("card.confirm_delete") } else { t!("card.delete") }.into(), "delete")
                    .when(confirm, |d| d.text_color(danger).border_color(danger))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.delete_doc(id, ix, window, cx);
                        cx.notify();
                    })),
            );
        }

        let mut rows = Vec::new();
        if let Value::Object(map) = doc {
            for (k, v) in map {
                self.push_rows(&mut rows, tab, &format!("{ix}"), k, v, 0, syntax, cx);
            }
        }
        div()
            .id(SharedString::from(format!("card-{id}-{ix}")))
            .group(group)
            .relative()
            .p_2()
            .border_1()
            .border_color(border)
            .rounded_md()
            .font_family(mono)
            .text_xs()
            .children(rows)
            .child(actions)
            .when_some(tab.card_error.as_ref().filter(|(i, _)| *i == ix).map(|(_, e)| e.clone()), |d, e| {
                d.child(div().pt_1().text_color(danger).child(e))
            })
            .into_any_element()
    }

    /// One row per field; nested objects and arrays start collapsed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_rows(&self, rows: &mut Vec<AnyElement>, tab: &Tab, parent: &str, key: &str, value: &Value, depth: usize, syntax: &Arc<HighlightTheme>, cx: &mut Context<Self>) {
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.list_hover);
        let path = format!("{parent}/{key}");
        let container = match value {
            Value::Object(m) if !is_typed(m) => Some(if m.is_empty() { "{}".to_string() } else { t!("tree.fields", n = m.len()).into() }),
            Value::Array(a) => Some(t!("tree.array", n = a.len()).into()),
            _ => None,
        };
        let key_color = syntax.style("property").and_then(|s| s.color).unwrap_or(theme.foreground);
        let row = div().flex().gap_1().pl(px(depth as f32 * 16.)).min_h(px(18.)).items_center().whitespace_nowrap();
        let Some(summary) = container else {
            let text = to_shell(value, "");
            let highlights: Vec<_> = tokenize(&text)
                .into_iter()
                .filter_map(|(r, kind)| syntax.style(kind).map(|s| (r, s)))
                .collect();
            rows.push(
                row.child(div().w(px(10.)))
                    .child(div().text_color(key_color).child(format!("{key}:")))
                    .child(StyledText::new(text).with_highlights(highlights))
                    .into_any_element(),
            );
            return;
        };
        let open = tab.expanded.contains(&path);
        let id = tab.id;
        rows.push(
            row.id(SharedString::from(format!("row-{id}-{path}")))
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .on_click(cx.listener({
                    let path = path.clone();
                    move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id)
                            && !tab.expanded.remove(&path)
                        {
                            tab.expanded.insert(path.clone());
                        }
                        cx.notify();
                    }
                }))
                .child(div().w(px(10.)).text_color(muted).child(if open { "▾" } else { "▸" }))
                .child(div().text_color(key_color).child(format!("{key}:")))
                .child(div().text_color(muted).child(summary))
                .into_any_element(),
        );
        if !open {
            return;
        }
        match value {
            Value::Object(m) => {
                for (k, v) in m {
                    self.push_rows(rows, tab, &path, k, v, depth + 1, syntax, cx);
                }
            }
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    self.push_rows(rows, tab, &path, &i.to_string(), v, depth + 1, syntax, cx);
                }
            }
            _ => {}
        }
    }
}

/// Whether an object is an extended JSON wrapper like `{"$oid": …}`, shown as one value.
pub(crate) fn is_typed(m: &Map<String, Value>) -> bool {
    m.len() <= 2 && m.keys().next().is_some_and(|k| k.starts_with('$'))
}
